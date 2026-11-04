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

use crate::core::{Action, Core, Key};
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
    Reset,
}

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

/// Run a key through the core; report toggles to the menu.
fn feed(key: Key) -> Action {
    let toggle = key == Key::Toggle;
    let mut core = CORE.get().unwrap().lock().unwrap();
    let action = core.on_key(key);
    if toggle {
        notify(UiEvent::Enabled(core.enabled));
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
    if ctrl || opt || cmd {
        // A shortcut, or an Option-composed character we don't model.
        return Key::Reset;
    }
    match kc {
        KC_SPACE => return Key::Space(t),
        k if k == KC_DELETE as i64 => return Key::Backspace,
        KC_RETURN | KC_TAB | KC_ESCAPE | KC_KEYPAD_ENTER => return Key::Reset,
        k if KC_NAV.contains(&k) => return Key::Reset,
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
    Key::Reset // dead keys, function keys, anything we can't model
}

unsafe extern "C" fn callback(_proxy: CGEventTapProxy, ty: u32, ev: CGEventRef, _user: *mut c_void) -> CGEventRef {
    if ty == TAP_DISABLED_BY_TIMEOUT || ty == TAP_DISABLED_BY_USER_INPUT {
        // macOS switches a tap off if it is ever too slow; switch it back on.
        if let Some(tap) = TAP.get() {
            CGEventTapEnable(tap.0, true);
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
            Key::Char(..) | Key::Space(_) | Key::Backspace | Key::Toggle => {
                sh.jobs.push_back(Job::Held(key));
                std::ptr::null_mut()
            }
            _ => {
                sh.jobs.push_back(Job::Reset);
                ev
            }
        };
    }
    let action = feed(key);
    match action {
        Action::Pass => ev,
        Action::Swallow => std::ptr::null_mut(),
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
            Job::Held(key) => {
                let action = feed(key.clone());
                match action {
                    Action::Pass => replay(&key),
                    Action::Replace { backspaces, text } => type_fix(backspaces, &text),
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
    // A short pause keeps slower apps from dropping or reordering events.
    std::thread::sleep(std::time::Duration::from_micros(800));
}

fn type_text(text: &str) {
    let mut buf = [0u16; 2];
    for c in text.chars() {
        let units = c.encode_utf16(&mut buf);
        let kc = if c == ' ' { KC_SPACE as u16 } else { 0 };
        post_key(kc, Some(units));
    }
}

fn type_fix(backspaces: usize, text: &str) {
    for _ in 0..backspaces {
        post_key(KC_DELETE, None);
    }
    type_text(text);
}

fn replay(key: &Key) {
    match key {
        Key::Char(c, _) => type_text(&c.to_string()),
        Key::Space(_) => type_text(" "),
        Key::Backspace => post_key(KC_DELETE, None),
        _ => {}
    }
}
