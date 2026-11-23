//! macOS: a CGEventTap on key-down and mouse-down events, using the
//! CoreGraphics C API directly (so events can be swallowed).
//!
//! Same design as the Windows layer:
//! * our own injected events carry a marker in EVENT_SOURCE_USER_DATA and are
//!   ignored;
//! * when a word needs fixing the space is swallowed and a worker thread types
//!   the fix; the user's plain keys are held back meanwhile and replayed in
//!   order afterwards;
//! * terminals, code editors and password managers are excluded (the list
//!   lives in `quipkeys-exclude.txt`, see settings.rs). Password fields turn on
//!   macOS "secure input", which hides keys from every event tap, so they are
//!   skipped automatically.
//!
//! Needs permission: System Settings > Privacy & Security > Accessibility.
//! The tap runs on its own thread; until permission is granted that thread
//! retries every second, so QuipKeys starts working the moment it's allowed.

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Instant;

use crate::compose::{self, Ops, Status};
use crate::core::{Action, ComposeReq, Core, Key};
use crate::settings;
use crate::ui::{notify, UiEvent};

type CFTypeRef = *const c_void;
type CGEventRef = *mut c_void;
type CFMachPortRef = *mut c_void;
type CGEventTapProxy = *mut c_void;
type TapCallback = unsafe extern "C" fn(CGEventTapProxy, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, callback: TapCallback, user: *mut c_void) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(ev: CGEventRef, field: u32) -> i64;
    fn CGEventSetIntegerValueField(ev: CGEventRef, field: u32, value: i64);
    fn CGEventGetFlags(ev: CGEventRef) -> u64;
    fn CGEventSetFlags(ev: CGEventRef, flags: u64);
    fn CGEventKeyboardGetUnicodeString(ev: CGEventRef, max: usize, len: *mut usize, buf: *mut u16);
    fn CGEventKeyboardSetUnicodeString(ev: CGEventRef, len: usize, buf: *const u16);
    fn CGEventCreateKeyboardEvent(source: *const c_void, keycode: u16, down: bool) -> CGEventRef;
    fn CGEventPost(tap: u32, ev: CGEventRef);
    fn CGEventCreateCopy(ev: CGEventRef) -> CGEventRef;
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
    static kAXTrustedCheckOptionPrompt: CFTypeRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: CFTypeRef;
    fn CFMachPortCreateRunLoopSource(alloc: CFTypeRef, port: CFMachPortRef, order: isize) -> *mut c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(rl: *mut c_void, source: *mut c_void, mode: CFTypeRef);
    fn CFRunLoopRun();
    fn CFRelease(cf: CFTypeRef);
    static kCFBooleanTrue: CFTypeRef;
    static kCFTypeDictionaryKeyCallBacks: u8;
    static kCFTypeDictionaryValueCallBacks: u8;
    fn CFDictionaryCreate(
        alloc: CFTypeRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        n: isize,
        key_cb: *const u8,
        value_cb: *const u8,
    ) -> CFTypeRef;
}

// Event types and fields (CGEventTypes.h).
const KEY_DOWN: u32 = 10;
const LEFT_MOUSE_DOWN: u32 = 1;
const RIGHT_MOUSE_DOWN: u32 = 3;
const OTHER_MOUSE_DOWN: u32 = 25;
const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_BY_USER_INPUT: u32 = 0xFFFF_FFFF;
const FIELD_KEYCODE: u32 = 9;
const FIELD_TARGET_PID: u32 = 41;
const FIELD_USER_DATA: u32 = 42;
const HID_TAP: u32 = 0;
const ANNOTATED_SESSION_TAP: u32 = 2;
const HEAD_INSERT: u32 = 0;
const FLAG_SHIFT: u64 = 0x0002_0000;
const FLAG_CONTROL: u64 = 0x0004_0000;
const FLAG_OPTION: u64 = 0x0008_0000;
const FLAG_COMMAND: u64 = 0x0010_0000;

// Virtual key codes (Events.h, ANSI layout positions).
const KC_Q: i64 = 12;
const KC_N: i64 = 45;
const KC_W: i64 = 13;
const KC_C: u16 = 8;
const KC_V: u16 = 9;
const KC_LEFT: u16 = 123;
const KC_RIGHT: u16 = 124;
/// Real arrow-key events carry the numeric-pad and function-key flags.
const ARROW_FLAGS: u64 = 0x0020_0000 | 0x0080_0000;
const KC_RETURN: i64 = 36;
const KC_TAB: i64 = 48;
const KC_SPACE: i64 = 49;
const KC_DELETE: u16 = 51;
const KC_ESCAPE: i64 = 53;
const KC_KEYPAD_ENTER: i64 = 76;
/// Home, PgUp, Fwd-delete, End, PgDn, arrows, Help.
const KC_NAV: &[i64] = &[114, 115, 116, 117, 119, 121, 123, 124, 125, 126];

/// Marks the events we post ourselves.
const MARKER: i64 = 0x5155_4B59; // "QUKY"

enum Job {
    Fix(usize, String),
    Held(Key),
    /// Any other key press (Enter, arrows, a shortcut, a dead key) held back
    /// while a fix is typed: a copy of the event, re-posted in order.
    Raw(Key, HeldEvent),
    Reset,
}

/// An owned copy of a key event (released when posted).
struct HeldEvent(CGEventRef);
unsafe impl Send for HeldEvent {}

struct Shared {
    busy: bool,
    jobs: VecDeque<Job>,
    last_pid: i64,
    excluded: bool,
    /// Exclusion-list generation `excluded` was computed for.
    gen: u32,
}

struct Tap(CFMachPortRef);
unsafe impl Send for Tap {}
unsafe impl Sync for Tap {}

static CORE: OnceLock<Mutex<Core>> = OnceLock::new();
static SHARED: OnceLock<(Mutex<Shared>, Condvar)> = OnceLock::new();
static TAP: OnceLock<Tap> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Executable names (lower case) where QuipKeys stays out of the way.
pub const DEFAULT_EXCLUDE: &[&str] = &[
    "terminal", "iterm2", "alacritty", "kitty", "wezterm-gui", "warp", "stable", "ghostty", "hyper", "tabby",
    "code", "code - insiders", "cursor", "windsurf", "zed", "xcode", "sublime_text", "idea", "pycharm", "webstorm",
    "clion", "rider", "goland", "android studio", "nvim", "neovide", "macvim", "emacs",
    "1password", "1password 7", "bitwarden", "keychain access", "keepassxc", "lastpass", "loginwindow",
];

pub fn start(core: Core) {
    START.set(Instant::now()).ok();
    CORE.set(Mutex::new(core)).ok();
    SHARED
        .set((Mutex::new(Shared { busy: false, jobs: VecDeque::new(), last_pid: -1, excluded: false, gen: u32::MAX }), Condvar::new()))
        .ok();
    std::thread::spawn(worker);
    compose::watch_clipboard(pasteboard_change_count);
    std::thread::spawn(|| unsafe {
        let mask = (1u64 << KEY_DOWN) | (1 << LEFT_MOUSE_DOWN) | (1 << RIGHT_MOUSE_DOWN) | (1 << OTHER_MOUSE_DOWN);
        // Wait for permission (the welcome window walks the user through it).
        let tap = loop {
            let tap = CGEventTapCreate(ANNOTATED_SESSION_TAP, HEAD_INSERT, 0, mask, callback, std::ptr::null_mut());
            if !tap.is_null() {
                break tap;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        };
        TAP.set(Tap(tap)).ok();
        let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
        CFRunLoopAddSource(CFRunLoopGetCurrent(), source, kCFRunLoopCommonModes);
        CGEventTapEnable(tap, true);
        RUNNING.store(true, Ordering::Relaxed);
        notify(UiEvent::Running);
        CFRunLoopRun();
    });
}

/// Seconds east of UTC for local time now (so the stats' "today" is the user's day).
pub fn utc_offset_seconds() -> i64 {
    unsafe {
        let now = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&now, &mut tm);
        tm.tm_gmtoff as i64
    }
}

/// Is QuipKeys allowed to watch and type (Accessibility), and running?
pub fn has_permission() -> bool {
    RUNNING.load(Ordering::Relaxed) || unsafe { AXIsProcessTrusted() }
}

/// Show the system "allow QuipKeys to control this computer" prompt, which
/// also adds QuipKeys to the Accessibility list, then open that list.
pub fn request_permission() {
    unsafe {
        let keys = [kAXTrustedCheckOptionPrompt];
        let values = [kCFBooleanTrue];
        let opts = CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            1,
            &kCFTypeDictionaryKeyCallBacks,
            &kCFTypeDictionaryValueCallBacks,
        );
        AXIsProcessTrustedWithOptions(opts);
        if !opts.is_null() {
            CFRelease(opts);
        }
    }
    settings::open("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility");
}

/// Switch correction on or off (from the menu).
pub fn set_enabled(on: bool) {
    let mut core = CORE.get().unwrap().lock().unwrap();
    if core.enabled != on {
        core.on_key(Key::Toggle);
    }
}

/// QK_DEBUG=1: log every decision to stderr (run the binary from a terminal).
fn debug() -> bool {
    static D: OnceLock<bool> = OnceLock::new();
    *D.get_or_init(|| std::env::var_os("QK_DEBUG").is_some())
}

/// Switch learning of the user's typing habits on or off.
pub fn set_learning(on: bool) {
    if let Some(c) = CORE.get() {
        c.lock().unwrap().set_learning(on);
    }
}

/// Forget everything learned about the user's typing.
pub fn forget_profile() {
    if let Some(c) = CORE.get() {
        c.lock().unwrap().forget_profile();
    }
}

/// Letters of typing learned from so far.
pub fn learned_letters() -> u64 {
    CORE.get().map(|c| c.lock().unwrap().learned_letters()).unwrap_or(0)
}

/// Words undone with Backspace this session (not saved), for the My words window.
pub fn undone_words() -> Vec<String> {
    CORE.get().map(|c| c.lock().unwrap().undone_words()).unwrap_or_default()
}

/// Run a key through the core; report toggles to the menu.
fn feed(key: Key) -> Action {
    let toggle = key == Key::Toggle;
    let mut core = CORE.get().unwrap().lock().unwrap();
    let shown = debug().then(|| format!("{key:?}"));
    let action = core.on_key(key);
    if let Some(k) = shown {
        if action != Action::Pass || !k.starts_with("Char") {
            eprintln!("[quipkeys] {k} -> {action:?}");
        }
    }
    if toggle {
        notify(UiEvent::Enabled(core.enabled));
    }
    let notes = core.take_notes();
    if !notes.is_empty() {
        notify(UiEvent::Fixed(notes));
    }
    action
}

/// Lower-case executable name of a process.
fn process_name(pid: i64) -> String {
    let mut buf = [0u8; 4096];
    let n = unsafe { libc::proc_pidpath(pid as i32, buf.as_mut_ptr() as *mut c_void, buf.len() as u32) };
    if n <= 0 {
        return String::new();
    }
    let path = String::from_utf8_lossy(&buf[..n as usize]);
    path.rsplit('/').next().unwrap_or("").to_lowercase()
}

fn now_ms() -> f64 {
    START.get().unwrap().elapsed().as_secs_f64() * 1000.0
}

/// Translate a key-down into a [`Key`].
unsafe fn translate(ev: CGEventRef) -> Key {
    let kc = CGEventGetIntegerValueField(ev, FIELD_KEYCODE);
    let flags = CGEventGetFlags(ev);
    let t = now_ms();
    let (ctrl, opt, cmd) = (flags & FLAG_CONTROL != 0, flags & FLAG_OPTION != 0, flags & FLAG_COMMAND != 0);
    if ctrl && opt && kc == KC_Q {
        return Key::Toggle;
    }
    if ctrl && opt && kc == KC_N {
        return Key::NextGuess;
    }
    if ctrl && opt && kc == KC_W {
        return Key::Compose;
    }
    let caret_key = matches!(kc, KC_RETURN | KC_TAB | KC_ESCAPE | KC_KEYPAD_ENTER) || KC_NAV.contains(&kc);
    if ctrl || cmd || (opt && (caret_key || kc == KC_SPACE || kc == KC_DELETE as i64)) {
        return Key::Reset; // a shortcut, or Option moving the caret / deleting a word
    }
    if opt {
        // An Option-composed character (é, ü, €): it goes into the word, and
        // we can't model it, so that word is left alone.
        return Key::Unknown;
    }
    match kc {
        KC_SPACE => return Key::Space(t),
        k if k == KC_DELETE as i64 => return Key::Backspace,
        _ if caret_key => return Key::Reset,
        _ => {}
    }
    let _ = FLAG_SHIFT; // the character below already reflects Shift / Caps Lock
    let mut buf = [0u16; 8];
    let mut len = 0usize;
    CGEventKeyboardGetUnicodeString(ev, buf.len(), &mut len, buf.as_mut_ptr());
    if len == 1 {
        if let Some(c) = char::from_u32(buf[0] as u32) {
            // 0xF700..0xF8FF: function keys and other non-text keys.
            if !c.is_control() && !(0xF700..=0xF8FF).contains(&(c as u32)) {
                return Key::Char(c, t);
            }
        }
    }
    if len == 0 {
        return Key::Unknown; // a dead key: an accent waiting for its letter
    }
    Key::Reset // function keys and anything else we can't model
}

unsafe extern "C" fn callback(_proxy: CGEventTapProxy, ty: u32, ev: CGEventRef, _user: *mut c_void) -> CGEventRef {
    if ty == TAP_DISABLED_BY_TIMEOUT || ty == TAP_DISABLED_BY_USER_INPUT {
        // macOS switches a tap off if it is ever too slow; switch it back on.
        if let Some(tap) = TAP.get() {
            CGEventTapEnable(tap.0, true);
        }
        // Keys typed while the tap was off were never seen: forget the tracked text.
        let (lock, _) = SHARED.get().unwrap();
        let mut sh = lock.lock().unwrap();
        if sh.busy {
            sh.jobs.push_back(Job::Reset);
        } else {
            feed(Key::Reset);
        }
        return ev;
    }
    if CGEventGetIntegerValueField(ev, FIELD_USER_DATA) == MARKER {
        return ev;
    }
    let (lock, cv) = SHARED.get().unwrap();
    let mut sh = lock.lock().unwrap();
    if ty != KEY_DOWN {
        // A click: the caret may have moved.
        if sh.busy {
            sh.jobs.push_back(Job::Reset);
        } else {
            feed(Key::Reset);
        }
        return ev;
    }
    // Focus moved to another app?
    let pid = CGEventGetIntegerValueField(ev, FIELD_TARGET_PID);
    let gen = settings::EXCLUDE_GEN.load(Ordering::Relaxed);
    if pid > 0 && (pid != sh.last_pid || gen != sh.gen) {
        let moved = pid != sh.last_pid;
        sh.last_pid = pid;
        sh.gen = gen;
        let name = process_name(pid);
        sh.excluded = settings::is_excluded(&name);
        if moved {
            if pid != std::process::id() as i64 {
                notify(UiEvent::App(name));
            }
            if sh.busy {
                sh.jobs.push_back(Job::Reset);
            } else {
                feed(Key::Reset);
            }
        }
    }
    let key = translate(ev);
    if key != Key::Toggle && sh.excluded {
        return ev;
    }
    if sh.busy {
        return match key {
            Key::Char(..) | Key::Space(_) | Key::Backspace | Key::Toggle | Key::NextGuess | Key::Compose => {
                sh.jobs.push_back(Job::Held(key));
                std::ptr::null_mut()
            }
            // Enter, arrows, shortcuts, dead keys: they must not overtake the
            // fix and the held keys ("teh cat" + Enter would send without
            // "cat"). Hold a copy and re-post it in order.
            _ => {
                let copy = CGEventCreateCopy(ev);
                if copy.is_null() {
                    sh.jobs.push_back(Job::Reset);
                    return ev;
                }
                sh.jobs.push_back(Job::Raw(key, HeldEvent(copy)));
                std::ptr::null_mut()
            }
        };
    }
    let action = feed(key);
    match action {
        Action::Pass => ev,
        Action::Swallow => std::ptr::null_mut(),
        Action::Compose(req) => {
            start_compose(req);
            std::ptr::null_mut()
        }
        Action::Replace { backspaces, text } => {
            sh.busy = true;
            sh.jobs.push_back(Job::Fix(backspaces, text));
            cv.notify_one();
            std::ptr::null_mut()
        }
    }
}

fn worker() {
    let (lock, cv) = SHARED.get().unwrap();
    loop {
        let job = {
            let mut sh = lock.lock().unwrap();
            loop {
                if let Some(j) = sh.jobs.pop_front() {
                    break j;
                }
                sh.busy = false;
                sh = cv.wait(sh).unwrap();
            }
        };
        match job {
            Job::Fix(n, text) => type_fix(n, &text),
            Job::Reset => {
                feed(Key::Reset);
            }
            Job::Raw(key, held) => {
                unsafe {
                    CGEventSetIntegerValueField(held.0, FIELD_USER_DATA, MARKER);
                    CGEventPost(HID_TAP, held.0);
                    CFRelease(held.0 as CFTypeRef);
                }
                pause("QK_KEY_DELAY_MS", 2.0);
                feed(key);
            }
            Job::Held(key) => {
                let action = feed(key.clone());
                match action {
                    Action::Pass => replay(&key),
                    Action::Replace { backspaces, text } => type_fix(backspaces, &text),
                    Action::Compose(req) => start_compose(req),
                    Action::Swallow => {}
                }
            }
        }
    }
}

/// Post one key press (down + up), tagged with our marker. With `text`, the
/// event carries that text instead of whatever the key code would type.
fn post_key(keycode: u16, text: Option<&[u16]>) {
    unsafe {
        for down in [true, false] {
            let ev = CGEventCreateKeyboardEvent(std::ptr::null(), keycode, down);
            if ev.is_null() {
                return;
            }
            CGEventSetFlags(ev, 0); // a held Shift must not turn Delete into something else
            if let Some(t) = text {
                CGEventKeyboardSetUnicodeString(ev, t.len(), t.as_ptr());
            }
            CGEventSetIntegerValueField(ev, FIELD_USER_DATA, MARKER);
            CGEventPost(HID_TAP, ev);
            CFRelease(ev as CFTypeRef);
        }
    }
}

/// Pauses between injected keys. Apps handle Delete and typed text through
/// different paths, and without a pause some (Notes, browsers) apply a typed
/// letter before an earlier Delete, leaving "iI" instead of "I". Tunable for
/// testing: QK_KEY_DELAY_MS (default 2), QK_BS_DELAY_MS (after each Delete,
/// default 3), QK_SETTLE_MS (after the last Delete, default 15).
fn pause(var: &str, default_ms: f64) {
    let ms = std::env::var(var).ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(default_ms);
    if ms > 0.0 {
        std::thread::sleep(std::time::Duration::from_secs_f64(ms / 1000.0));
    }
}

fn type_text(text: &str) {
    let mut buf = [0u16; 2];
    for c in text.chars() {
        let units = c.encode_utf16(&mut buf);
        let kc = if c == ' ' { KC_SPACE as u16 } else { 0 };
        post_key(kc, Some(units));
        pause("QK_KEY_DELAY_MS", 2.0);
    }
}

fn type_fix(backspaces: usize, text: &str) {
    for _ in 0..backspaces {
        post_key(KC_DELETE, None);
        pause("QK_BS_DELAY_MS", 3.0);
    }
    if backspaces > 0 {
        pause("QK_SETTLE_MS", 15.0);
    }
    type_text(text);
}

fn replay(key: &Key) {
    match key {
        Key::Char(c, _) => type_text(&c.to_string()),
        Key::Space(_) => type_text(" "),
        Key::Backspace => {
            post_key(KC_DELETE, None);
            pause("QK_BS_DELAY_MS", 3.0);
        }
        _ => {}
    }
}

// ------------------------------------------------------------ write it for me

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const std::ffi::c_char) -> *mut c_void;
    fn sel_registerName(name: *const std::ffi::c_char) -> *mut c_void;
    fn objc_msgSend();
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}
#[link(name = "AppKit", kind = "framework")]
extern "C" {
    static NSPasteboardTypeString: *mut c_void;
}

type Id = *mut c_void;

/// objc_msgSend with no arguments.
unsafe fn msg0(obj: Id, sel: &std::ffi::CStr) -> Id {
    let f: unsafe extern "C" fn(Id, Id) -> Id = std::mem::transmute(objc_msgSend as *const ());
    f(obj, sel_registerName(sel.as_ptr()))
}

/// objc_msgSend with one or two pointer arguments.
unsafe fn msg1(obj: Id, sel: &std::ffi::CStr, a: *const c_void) -> Id {
    let f: unsafe extern "C" fn(Id, Id, *const c_void) -> Id = std::mem::transmute(objc_msgSend as *const ());
    f(obj, sel_registerName(sel.as_ptr()), a)
}

unsafe fn msg2(obj: Id, sel: &std::ffi::CStr, a: *const c_void, b: *const c_void) -> Id {
    let f: unsafe extern "C" fn(Id, Id, *const c_void, *const c_void) -> Id = std::mem::transmute(objc_msgSend as *const ());
    f(obj, sel_registerName(sel.as_ptr()), a, b)
}

unsafe fn general_pasteboard() -> Id {
    let class = objc_getClass(c"NSPasteboard".as_ptr());
    if class.is_null() {
        return std::ptr::null_mut();
    }
    msg0(class, c"generalPasteboard")
}

/// The clipboard's plain text, if it has any.
fn pasteboard_text() -> Option<String> {
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let mut out = None;
        let board = general_pasteboard();
        if !board.is_null() {
            let s = msg1(board, c"stringForType:", NSPasteboardTypeString);
            if !s.is_null() {
                let utf8 = msg0(s, c"UTF8String") as *const std::ffi::c_char;
                if !utf8.is_null() {
                    out = Some(std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned());
                }
            }
        }
        objc_autoreleasePoolPop(pool);
        out
    }
}

/// Put plain text on the clipboard.
fn set_pasteboard_text(text: &str) {
    let Ok(c) = std::ffi::CString::new(text.replace('\0', "")) else { return };
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let board = general_pasteboard();
        let class = objc_getClass(c"NSString".as_ptr());
        if !board.is_null() && !class.is_null() {
            let s = msg1(class, c"stringWithUTF8String:", c.as_ptr() as *const c_void);
            if !s.is_null() {
                msg0(board, c"clearContents");
                msg2(board, c"setString:forType:", s, NSPasteboardTypeString);
            }
        }
        objc_autoreleasePoolPop(pool);
    }
}
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventSourceFlagsState(state: i32) -> u64;
}

/// NSPasteboard.generalPasteboard.changeCount: bumps on every copy. Only the
/// counter is read, never the contents.
fn pasteboard_change_count() -> u64 {
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let board = general_pasteboard();
        let n = if board.is_null() { 0 } else { msg0(board, c"changeCount") as u64 };
        objc_autoreleasePoolPop(pool);
        n
    }
}

fn start_compose(req: ComposeReq) {
    let gen0 = CORE.get().unwrap().lock().unwrap().generation();
    compose::spawn(req, gen0, Box::new(MacOps));
}

/// Keyboard and clipboard work for compose.rs.
struct MacOps;

/// Wait (up to 3 s) until Control, Option, Shift and Command are let go, so
/// injected keys aren't combined with held ones.
fn modifiers_released() {
    const HELD: u64 = FLAG_SHIFT | FLAG_CONTROL | FLAG_OPTION | FLAG_COMMAND;
    for _ in 0..150 {
        // 1 = kCGEventSourceStateHIDSystemState: the physical keys.
        if unsafe { CGEventSourceFlagsState(1) } & HELD == 0 {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// One key press with modifier flags, tagged with our marker.
fn post_with_flags(keycode: u16, flags: u64) {
    unsafe {
        for down in [true, false] {
            let ev = CGEventCreateKeyboardEvent(std::ptr::null(), keycode, down);
            if ev.is_null() {
                return;
            }
            CGEventSetFlags(ev, flags);
            CGEventSetIntegerValueField(ev, FIELD_USER_DATA, MARKER);
            CGEventPost(HID_TAP, ev);
            CFRelease(ev as CFTypeRef);
        }
    }
    pause("QK_KEY_DELAY_MS", 2.0);
}

impl Ops for MacOps {
    fn clipboard_text(&self) -> Option<String> {
        pasteboard_text()
    }
    fn clipboard_age(&self) -> Option<std::time::Duration> {
        compose::clipboard_age()
    }
    fn set_clipboard(&self, text: &str) {
        set_pasteboard_text(text);
    }
    fn screen(&self) -> Option<crate::ai::Screen> {
        read_screen()
    }
    fn backspace(&self, n: usize) {
        modifiers_released();
        type_fix(n, "");
    }
    fn select_left(&self, n: usize) {
        modifiers_released();
        for _ in 0..n {
            post_with_flags(KC_LEFT, FLAG_SHIFT | ARROW_FLAGS);
        }
    }
    fn deselect(&self) {
        post_with_flags(KC_RIGHT, ARROW_FLAGS);
    }
    fn copy(&self) {
        modifiers_released();
        post_with_flags(KC_C, FLAG_COMMAND);
    }
    fn paste(&self) {
        modifiers_released();
        post_with_flags(KC_V, FLAG_COMMAND);
    }
    fn generation(&self) -> u64 {
        CORE.get().unwrap().lock().unwrap().generation()
    }
    fn wait(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    fn status(&self, s: Status) {
        notify(UiEvent::Compose(s));
    }
}

// ------------------------------------------------- reading the screen (AX)

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct CFRange {
    location: isize,
    length: isize,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXUIElementCreateSystemWide() -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(el: CFTypeRef, attr: CFTypeRef, out: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(el: CFTypeRef, attr: CFTypeRef, value: CFTypeRef) -> i32;
    fn AXUIElementGetPid(el: CFTypeRef, pid: *mut i32) -> i32;
    fn AXUIElementSetMessagingTimeout(el: CFTypeRef, secs: f32) -> i32;
    fn AXValueGetValue(v: CFTypeRef, ty: u32, out: *mut c_void) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithBytes(alloc: CFTypeRef, bytes: *const u8, len: isize, encoding: u32, external: bool) -> CFTypeRef;
    fn CFStringGetLength(s: CFTypeRef) -> isize;
    fn CFStringGetCharacters(s: CFTypeRef, range: CFRange, buf: *mut u16);
    fn CFGetTypeID(cf: CFTypeRef) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(a: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(a: CFTypeRef, i: isize) -> CFTypeRef;
    fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
}

const UTF8: u32 = 0x0800_0100;
const AX_VALUE_CFRANGE: u32 = 4;

/// An owned CF object, released when dropped.
struct Cf(CFTypeRef);

impl Drop for Cf {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

fn cf_str(s: &str) -> Cf {
    Cf(unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, UTF8, false) })
}

/// An attribute of an accessibility element (owned), if it has one.
fn ax(el: CFTypeRef, name: &str) -> Option<Cf> {
    let key = cf_str(name);
    let mut out: CFTypeRef = std::ptr::null();
    let err = unsafe { AXUIElementCopyAttributeValue(el, key.0, &mut out) };
    (err == 0 && !out.is_null()).then(|| Cf(out))
}

/// A CFString as UTF-16 (to split at the cursor, which AX counts in UTF-16).
fn utf16(cf: &Cf) -> Option<Vec<u16>> {
    unsafe {
        if CFGetTypeID(cf.0) != CFStringGetTypeID() {
            return None;
        }
        let n = CFStringGetLength(cf.0);
        let mut buf = vec![0u16; n.max(0) as usize];
        CFStringGetCharacters(cf.0, CFRange { location: 0, length: n }, buf.as_mut_ptr());
        Some(buf)
    }
}

fn ax_string(el: CFTypeRef, name: &str) -> Option<String> {
    ax(el, name).and_then(|v| utf16(&v)).map(|u| String::from_utf16_lossy(&u))
}

/// The app, window title, text box around the cursor and window text, read
/// through Accessibility (the permission QuipKeys already has).
fn read_screen() -> Option<crate::ai::Screen> {
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let out = read_screen_inner();
        objc_autoreleasePoolPop(pool);
        out
    }
}

unsafe fn read_screen_inner() -> Option<crate::ai::Screen> {
    let sys = Cf(AXUIElementCreateSystemWide());
    if sys.0.is_null() {
        return None;
    }
    // Never let a slow app hold us up for long.
    AXUIElementSetMessagingTimeout(sys.0, 0.25);
    let app = ax(sys.0, "AXFocusedApplication")?;
    let mut pid = 0i32;
    AXUIElementGetPid(app.0, &mut pid);
    // Chromium and Electron apps (Chrome, Slack, Teams, Discord, WhatsApp)
    // only expose their text once someone asks for it.
    let manual = cf_str("AXManualAccessibility");
    AXUIElementSetAttributeValue(app.0, manual.0, kCFBooleanTrue);
    let window = ax(app.0, "AXFocusedWindow");
    let mut s = crate::ai::Screen {
        app: process_name(pid as i64),
        title: window.as_ref().and_then(|w| ax_string(w.0, "AXTitle")).unwrap_or_default(),
        ..Default::default()
    };
    if let Some(field) = ax(sys.0, "AXFocusedUIElement") {
        if ax_string(field.0, "AXSubrole").as_deref() == Some("AXSecureTextField") {
            return Some(s); // a password field: read nothing
        }
        if let Some(value) = ax(field.0, "AXValue").and_then(|v| utf16(&v)) {
            let mut r = CFRange { location: value.len() as isize, length: 0 };
            if let Some(range) = ax(field.0, "AXSelectedTextRange") {
                AXValueGetValue(range.0, AX_VALUE_CFRANGE, &mut r as *mut CFRange as *mut c_void);
            }
            let start = (r.location.max(0) as usize).min(value.len());
            let end = (start + r.length.max(0) as usize).min(value.len());
            s.before = String::from_utf16_lossy(&value[..start]);
            s.selected = String::from_utf16_lossy(&value[start..end]);
            s.after = String::from_utf16_lossy(&value[end..]);
        }
    }
    if let Some(w) = &window {
        s.window = window_text(w.0);
    }
    Some(s)
}

/// The visible text of a window: its static texts, in reading order. Walks
/// at most a few thousand elements and stops after 300 ms.
unsafe fn window_text(window: CFTypeRef) -> String {
    let deadline = Instant::now() + std::time::Duration::from_millis(300);
    let mut out = String::new();
    let mut seen = 0;
    // Depth-first, children in order: (element, owned handle if we copied it).
    let mut stack: Vec<Cf> = Vec::new();
    let visit = |el: CFTypeRef, out: &mut String, stack: &mut Vec<Cf>| {
        let role = ax_string(el, "AXRole").unwrap_or_default();
        if role == "AXStaticText" {
            if let Some(t) = ax_string(el, "AXValue") {
                let t = t.trim();
                if !t.is_empty() {
                    out.push_str(t);
                    out.push('\n');
                }
            }
        }
        if role == "AXSecureTextField" {
            return;
        }
        if let Some(children) = ax(el, "AXChildren") {
            if CFGetTypeID(children.0) == CFArrayGetTypeID() {
                let n = CFArrayGetCount(children.0);
                // Push in reverse so the first child is visited first.
                for i in (0..n).rev() {
                    let c = CFArrayGetValueAtIndex(children.0, i);
                    if !c.is_null() {
                        CFRetain(c);
                        stack.push(Cf(c));
                    }
                }
            }
        }
    };
    visit(window, &mut out, &mut stack);
    while let Some(el) = stack.pop() {
        seen += 1;
        if seen > 4000 || out.len() > 40_000 || Instant::now() > deadline {
            break;
        }
        visit(el.0, &mut out, &mut stack);
    }
    out
}
