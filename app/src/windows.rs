//! Windows: a low-level keyboard hook (WH_KEYBOARD_LL) plus a mouse hook.
//!
//! * Our own injected keys carry a marker in `dwExtraInfo` and are ignored.
//! * When a word needs fixing, the space is swallowed and a worker thread
//!   types the fix (backspaces + text) with SendInput. While it does, the
//!   user's plain keys are held back and replayed afterwards in order, so
//!   nothing lands in the middle of a correction.
//! * Terminals, code editors and password managers are excluded (the list
//!   lives in `quipkeys-exclude.txt`, see settings.rs), and Win32 password
//!   boxes are detected and skipped.
//! * The hooks run on their own thread with its own message loop, so nothing
//!   the tray icon or welcome window does can ever delay a key.

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::{Condvar, Mutex, OnceLock};

use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::compose::{self, Ops, Status};
use crate::core::{Action, ComposeReq, Core, Key};
use crate::settings;
use crate::ui::{notify, UiEvent};

/// Marks the key events we inject ourselves.
const MARKER: usize = 0x5155_4B59; // "QUKY"

enum Job {
    /// Delete `n` characters, then type `text`.
    Fix(usize, String),
    /// A user key held back while a fix was being typed.
    Held(Key),
    /// A caret key (Enter, Tab, arrows...) or dead key held back the same way:
    /// replayed as the same key press, then treated as `Key`.
    Raw { key: Key, vk: u16, scan: u16, extended: bool },
    /// Forget the tracked text (a click or special key arrived while busy).
    Reset,
}

struct Shared {
    busy: bool,
    jobs: VecDeque<Job>,
    last_window: HWND,
    excluded: bool,
    /// Exclusion-list generation `excluded` was computed for.
    gen: u32,
}

unsafe impl Send for Shared {}

static CORE: OnceLock<Mutex<Core>> = OnceLock::new();
static SHARED: OnceLock<(Mutex<Shared>, Condvar)> = OnceLock::new();
static mut KB_HOOK: HHOOK = std::ptr::null_mut();
static mut MOUSE_HOOK: HHOOK = std::ptr::null_mut();

pub const DEFAULT_EXCLUDE: &[&str] = &[
    "cmd.exe", "powershell.exe", "pwsh.exe", "windowsterminal.exe", "wt.exe", "conhost.exe", "openconsole.exe",
    "code.exe", "devenv.exe", "idea64.exe", "pycharm64.exe", "rider64.exe", "sublime_text.exe", "notepad++.exe",
    "mintty.exe", "putty.exe", "alacritty.exe", "wezterm-gui.exe", "keepass.exe", "keepassxc.exe",
    "1password.exe", "bitwarden.exe", "lastpass.exe",
];

pub fn start(core: Core) {
    CORE.set(Mutex::new(core)).ok();
    SHARED
        .set((
            Mutex::new(Shared { busy: false, jobs: VecDeque::new(), last_window: std::ptr::null_mut(), excluded: false, gen: u32::MAX }),
            Condvar::new(),
        ))
        .ok();
    std::thread::spawn(worker);
    compose::watch_clipboard(|| unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() as u64 });
    std::thread::spawn(|| unsafe {
        let module = GetModuleHandleW(std::ptr::null());
        KB_HOOK = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0);
        MOUSE_HOOK = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0);
        if KB_HOOK.is_null() {
            notify(UiEvent::Error("QuipKeys couldn't start watching the keyboard.".into()));
            return;
        }
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}

/// Windows needs no extra permission for a keyboard hook.
pub fn has_permission() -> bool {
    true
}

pub fn request_permission() {}

/// Switch correction on or off (from the tray menu).
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

/// Run a key through the core; report toggles to the tray.
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

/// Lower-case exe name of the process owning a window.
unsafe fn process_name(hwnd: HWND) -> String {
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if h.is_null() {
        return String::new();
    }
    let mut buf = [0u16; 512];
    let mut len = buf.len() as u32;
    let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
    CloseHandle(h);
    if ok == 0 {
        return String::new();
    }
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    path.rsplit('\\').next().unwrap_or("").to_lowercase()
}

/// Is the focused control a classic Win32 password box?
unsafe fn focus_is_password(hwnd: HWND) -> bool {
    let tid = GetWindowThreadProcessId(hwnd, std::ptr::null_mut());
    let mut gui: GUITHREADINFO = std::mem::zeroed();
    gui.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    if GetGUIThreadInfo(tid, &mut gui) == 0 || gui.hwndFocus.is_null() {
        return false;
    }
    let style = GetWindowLongW(gui.hwndFocus, GWL_STYLE) as u32;
    let mut class = [0u16; 32];
    let n = GetClassNameW(gui.hwndFocus, class.as_mut_ptr(), class.len() as i32);
    let class = String::from_utf16_lossy(&class[..n.max(0) as usize]).to_lowercase();
    class == "edit" && style & 0x20 != 0 // ES_PASSWORD
}

fn down(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

/// Translate a key-down into a [`Key`], or None for keys we ignore (bare modifiers).
unsafe fn translate(kb: &KBDLLHOOKSTRUCT, hwnd: HWND) -> Option<Key> {
    let vk = kb.vkCode as u16;
    let t = kb.time as f64;
    match vk {
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT | VK_CONTROL | VK_LCONTROL | VK_RCONTROL | VK_MENU | VK_LMENU | VK_RMENU
        | VK_LWIN | VK_RWIN | VK_CAPITAL | VK_NUMLOCK | VK_SCROLL => return None,
        _ => {}
    }
    let ctrl = down(VK_CONTROL);
    let alt = down(VK_MENU);
    // AltGr arrives as left Ctrl + right Alt. It types characters ("@" is
    // AltGr+Q on German keyboards), so it is neither the hotkey nor a shortcut.
    let altgr = down(VK_RMENU) && down(VK_LCONTROL) && !down(VK_RCONTROL) && !down(VK_LMENU);
    if ctrl && alt && !altgr && vk == b'Q' as u16 {
        return Some(Key::Toggle);
    }
    if ctrl && alt && !altgr && vk == b'N' as u16 {
        return Some(Key::NextGuess);
    }
    if ctrl && alt && !altgr && vk == b'W' as u16 {
        return Some(Key::Compose);
    }
    if !altgr && (ctrl || alt) || down(VK_LWIN) || down(VK_RWIN) {
        return Some(Key::Reset); // a shortcut
    }
    match vk {
        VK_SPACE => return Some(Key::Space(t)),
        VK_BACK => return Some(Key::Backspace),
        VK_RETURN | VK_TAB | VK_ESCAPE | VK_LEFT | VK_RIGHT | VK_UP | VK_DOWN | VK_HOME | VK_END | VK_PRIOR | VK_NEXT
        | VK_DELETE | VK_INSERT => return Some(Key::Reset),
        _ => {}
    }
    // Ask Windows which character this key makes in the user's layout.
    let mut state = [0u8; 256];
    if down(VK_SHIFT) {
        state[VK_SHIFT as usize] = 0x80;
    }
    if GetKeyState(VK_CAPITAL as i32) & 1 != 0 {
        state[VK_CAPITAL as usize] = 0x01;
    }
    if altgr {
        state[VK_CONTROL as usize] = 0x80;
        state[VK_MENU as usize] = 0x80;
    }
    let layout = GetKeyboardLayout(GetWindowThreadProcessId(hwnd, std::ptr::null_mut()));
    let mut buf = [0u16; 8];
    // Flag 0x4: don't change the keyboard state (keeps dead keys working).
    let n = ToUnicodeEx(kb.vkCode, kb.scanCode, state.as_ptr(), buf.as_mut_ptr(), buf.len() as i32, 0x4, layout);
    if n == 1 {
        if let Some(c) = char::from_u32(buf[0] as u32) {
            if !c.is_control() {
                return Some(Key::Char(c, t));
            }
        }
    }
    if n < 0 || altgr {
        // A dead key (accent waiting for its letter) or an AltGr combination we
        // can't read: a character goes into the word, so leave that word alone.
        return Some(Key::Unknown);
    }
    Some(Key::Reset) // function keys and anything else we can't model
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(KB_HOOK, code, wparam, lparam);
    }
    let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
    let msg = wparam as u32;
    if kb.dwExtraInfo == MARKER || !(msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN) {
        return CallNextHookEx(KB_HOOK, code, wparam, lparam);
    }
    let (lock, cv) = SHARED.get().unwrap();
    let mut sh = lock.lock().unwrap();
    // Focus change, exclusions, password boxes.
    let hwnd = GetForegroundWindow();
    let gen = settings::EXCLUDE_GEN.load(Ordering::Relaxed);
    if hwnd != sh.last_window || gen != sh.gen {
        let moved = hwnd != sh.last_window;
        sh.last_window = hwnd;
        sh.gen = gen;
        let name = process_name(hwnd);
        sh.excluded = settings::is_excluded(&name);
        if moved {
            if name != "quipkeys.exe" {
                notify(UiEvent::App(name));
            }
            if sh.busy {
                sh.jobs.push_back(Job::Reset);
            } else {
                feed(Key::Reset);
            }
        }
    }
    let Some(key) = translate(kb, hwnd) else {
        return CallNextHookEx(KB_HOOK, code, wparam, lparam);
    };
    if key != Key::Toggle && (sh.excluded || focus_is_password(hwnd)) {
        return CallNextHookEx(KB_HOOK, code, wparam, lparam);
    }
    if sh.busy {
        // A fix is being typed. Hold plain keys back and replay them after;
        // let anything else through and forget the tracked text.
        return match key {
            Key::Char(..) | Key::Space(_) | Key::Backspace | Key::Toggle | Key::NextGuess | Key::Compose => {
                sh.jobs.push_back(Job::Held(key));
                1
            }
            // Enter, Tab, arrows, dead keys: they must not overtake the fix and
            // the held keys ("teh cat" + Enter would send without "cat"), so
            // hold them too and replay the same key press in order.
            _ if !(down(VK_CONTROL) || down(VK_MENU) || down(VK_LWIN) || down(VK_RWIN)) => {
                sh.jobs.push_back(Job::Raw { key, vk: kb.vkCode as u16, scan: kb.scanCode as u16, extended: kb.flags & LLKHF_EXTENDED != 0 });
                1
            }
            // A shortcut can't be replayed faithfully (its modifiers may be
            // released by then): let it through and forget the tracked text.
            _ => {
                sh.jobs.push_back(Job::Reset);
                CallNextHookEx(KB_HOOK, code, wparam, lparam)
            }
        };
    }
    let action = feed(key);
    match action {
        Action::Pass => CallNextHookEx(KB_HOOK, code, wparam, lparam),
        Action::Swallow => 1,
        Action::Compose(req) => {
            start_compose(req);
            1
        }
        Action::Replace { backspaces, text } => {
            sh.busy = true;
            sh.jobs.push_back(Job::Fix(backspaces, text));
            cv.notify_one();
            1
        }
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let msg = wparam as u32;
        if msg == WM_LBUTTONDOWN || msg == WM_RBUTTONDOWN || msg == WM_MBUTTONDOWN {
            let (lock, _) = SHARED.get().unwrap();
            if let Ok(mut sh) = lock.try_lock() {
                if sh.busy {
                    sh.jobs.push_back(Job::Reset);
                } else if let Ok(mut core) = CORE.get().unwrap().try_lock() {
                    core.on_key(Key::Reset);
                }
            }
        }
    }
    CallNextHookEx(MOUSE_HOOK, code, wparam, lparam)
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
            Job::Raw { key, vk, scan, extended } => {
                let ext = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
                send(&[key_input(vk, scan, ext), key_input(vk, scan, ext | KEYEVENTF_KEYUP)]);
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

fn key_input(vk: u16, scan: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: MARKER } },
    }
}

fn send(inputs: &[INPUT]) {
    if inputs.is_empty() {
        return;
    }
    unsafe {
        SendInput(inputs.len() as u32, inputs.as_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

fn type_fix(backspaces: usize, text: &str) {
    let mut v = Vec::with_capacity(backspaces * 2 + text.len() * 2);
    for _ in 0..backspaces {
        v.push(key_input(VK_BACK, 0, 0));
        v.push(key_input(VK_BACK, 0, KEYEVENTF_KEYUP));
    }
    push_text(&mut v, text);
    send(&v);
}

fn push_text(v: &mut Vec<INPUT>, text: &str) {
    for unit in text.encode_utf16() {
        v.push(key_input(0, unit, KEYEVENTF_UNICODE));
        v.push(key_input(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
    }
}

fn replay(key: &Key) {
    let mut v = Vec::new();
    match key {
        Key::Char(c, _) => push_text(&mut v, &c.to_string()),
        Key::Space(_) => push_text(&mut v, " "),
        Key::Backspace => {
            v.push(key_input(VK_BACK, 0, 0));
            v.push(key_input(VK_BACK, 0, KEYEVENTF_KEYUP));
        }
        _ => {}
    }
    send(&v);
}

// ------------------------------------------------------------ write it for me

fn start_compose(req: ComposeReq) {
    let gen0 = CORE.get().unwrap().lock().unwrap().generation();
    compose::spawn(req, gen0, Box::new(WinOps));
}

/// Keyboard and clipboard work for compose.rs.
struct WinOps;

/// Wait (up to 3 s) until the user has let go of Ctrl, Alt, Shift and Win,
/// so injected keys aren't combined with held ones (Ctrl+Alt+V...).
fn modifiers_released() {
    for _ in 0..150 {
        if ![VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN].into_iter().any(down) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

fn chord(modifier: u16, key: u16) {
    send(&[key_input(modifier, 0, 0), key_input(key, 0, 0), key_input(key, 0, KEYEVENTF_KEYUP), key_input(modifier, 0, KEYEVENTF_KEYUP)]);
}

impl Ops for WinOps {
    fn clipboard_text(&self) -> Option<String> {
        arboard::Clipboard::new().ok()?.get_text().ok()
    }
    fn clipboard_age(&self) -> Option<std::time::Duration> {
        compose::clipboard_age()
    }
    fn set_clipboard(&self, text: &str) {
        if let Ok(mut c) = arboard::Clipboard::new() {
            let _ = c.set_text(text.to_string());
        }
    }
    fn screen(&self) -> Option<crate::ai::Screen> {
        unsafe { read_screen() }
    }
    fn backspace(&self, n: usize) {
        modifiers_released();
        type_fix(n, "");
    }
    fn select_left(&self, n: usize) {
        modifiers_released();
        let mut v = vec![key_input(VK_SHIFT, 0, 0)];
        for _ in 0..n {
            v.push(key_input(VK_LEFT, 0, KEYEVENTF_EXTENDEDKEY));
            v.push(key_input(VK_LEFT, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP));
        }
        v.push(key_input(VK_SHIFT, 0, KEYEVENTF_KEYUP));
        send(&v);
    }
    fn deselect(&self) {
        send(&[key_input(VK_RIGHT, 0, KEYEVENTF_EXTENDEDKEY), key_input(VK_RIGHT, 0, KEYEVENTF_EXTENDEDKEY | KEYEVENTF_KEYUP)]);
    }
    fn copy(&self) {
        modifiers_released();
        chord(VK_CONTROL, b'C' as u16);
    }
    fn paste(&self) {
        modifiers_released();
        chord(VK_CONTROL, b'V' as u16);
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

// ------------------------------------------------- reading the screen (UIA)

/// The app, window title, text box around the cursor and window text, read
/// through UI Automation (no permission needed on Windows).
unsafe fn read_screen() -> Option<crate::ai::Screen> {
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    use windows::Win32::UI::Accessibility::*;
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    let hwnd = GetForegroundWindow();
    let mut title = [0u16; 512];
    let n = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
    let mut s = crate::ai::Screen {
        app: process_name(hwnd),
        title: String::from_utf16_lossy(&title[..n.max(0) as usize]),
        ..Default::default()
    };
    let Ok(uia) = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) else { return Some(s) };
    if let Ok(el) = uia.GetFocusedElement() {
        if el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false) || focus_is_password(hwnd) {
            return Some(s); // a password field: read nothing
        }
        if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
            let selection = tp.GetSelection().ok().filter(|a| a.Length().unwrap_or(0) > 0).and_then(|a| a.GetElement(0).ok());
            if let (Ok(doc), Some(sel)) = (tp.DocumentRange(), selection) {
                if let Ok(b) = doc.Clone() {
                    if b.MoveEndpointByRange(TextPatternRangeEndpoint_End, &sel, TextPatternRangeEndpoint_Start).is_ok() {
                        s.before = b.GetText(-1).map(|t| t.to_string()).unwrap_or_default();
                    }
                }
                s.selected = sel.GetText(5000).map(|t| t.to_string()).unwrap_or_default();
                if let Ok(a) = doc.Clone() {
                    if a.MoveEndpointByRange(TextPatternRangeEndpoint_Start, &sel, TextPatternRangeEndpoint_End).is_ok() {
                        s.after = a.GetText(5000).map(|t| t.to_string()).unwrap_or_default();
                    }
                }
            } else if let Ok(doc) = tp.DocumentRange() {
                s.before = doc.GetText(-1).map(|t| t.to_string()).unwrap_or_default();
            }
        } else if let Ok(vp) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            s.before = vp.CurrentValue().map(|t| t.to_string()).unwrap_or_default();
        }
    }
    if let Ok(root) = uia.ElementFromHandle(windows::Win32::Foundation::HWND(hwnd as _)) {
        s.window = window_text(&uia, &root);
    }
    Some(s)
}

/// The visible text of a window: its text elements, in reading order. Walks
/// at most a few thousand elements and stops after 300 ms.
unsafe fn window_text(
    uia: &windows::Win32::UI::Accessibility::IUIAutomation,
    root: &windows::Win32::UI::Accessibility::IUIAutomationElement,
) -> String {
    use windows::Win32::UI::Accessibility::*;
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(300);
    let mut out = String::new();
    let Ok(walker) = uia.ControlViewWalker() else { return out };
    // Depth-first: go down to the first child, else across, else back up.
    let mut stack: Vec<IUIAutomationElement> = vec![root.clone()];
    let mut seen = 0;
    while let Some(el) = stack.pop() {
        seen += 1;
        if seen > 4000 || out.len() > 40_000 || std::time::Instant::now() > deadline {
            break;
        }
        let ty = el.CurrentControlType().unwrap_or_default();
        if el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false) {
            continue;
        }
        if ty == UIA_TextControlTypeId {
            if let Ok(name) = el.CurrentName() {
                let t = name.to_string();
                let t = t.trim();
                if !t.is_empty() {
                    out.push_str(t);
                    out.push('\n');
                }
            }
        }
        // Children, first child on top of the stack.
        let mut kids = Vec::new();
        let mut c = walker.GetFirstChildElement(&el).ok();
        while let Some(k) = c {
            c = walker.GetNextSiblingElement(&k).ok();
            kids.push(k);
        }
        stack.extend(kids.into_iter().rev());
    }
    out
}
