//! The few things QuipKeys remembers between runs: whether the welcome screen
//! has been seen, the apps it stays out of, and your personal word list (only
//! words you taught it by undoing a fix, or added yourself). Nothing else you
//! type is ever written anywhere.
//!
//! Where they live:
//! * Windows: next to `quipkeys.exe` if that folder is writable (portable),
//!   otherwise `%APPDATA%\QuipKeys`.
//! * macOS: `~/Library/Application Support/QuipKeys`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

pub fn config_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        #[cfg(windows)]
        {
            if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) {
                let probe = dir.join(".quipkeys-write-test");
                if std::fs::write(&probe, b"").is_ok() {
                    let _ = std::fs::remove_file(probe);
                    return dir;
                }
            }
            let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            let dir = base.join("QuipKeys");
            let _ = std::fs::create_dir_all(&dir);
            dir
        }
        #[cfg(not(windows))]
        {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
            let dir = home.join("Library/Application Support/QuipKeys");
            let _ = std::fs::create_dir_all(&dir);
            dir
        }
    })
    .clone()
}

fn settings_path() -> PathBuf {
    config_dir().join("quipkeys-settings.txt")
}

pub fn exclude_path() -> PathBuf {
    config_dir().join("quipkeys-exclude.txt")
}

/// Your personal word list (see core.rs).
pub fn words_path() -> PathBuf {
    config_dir().join("quipkeys-words.txt")
}

/// Make sure the word list exists (so it can be opened for editing).
pub fn write_words_if_missing() {
    let p = words_path();
    if !p.exists() {
        let _ = std::fs::write(
            p,
            "# Your words: QuipKeys never changes these, and fixes typos of them.\n\
             # Words are added here when you undo a correction (Backspace right after it).\n\
             # Add or remove words freely, one per line.\n",
        );
    }
}

/// A yes/no setting from quipkeys-settings.txt ("name=1").
fn flag(name: &str) -> bool {
    std::fs::read_to_string(settings_path())
        .map(|s| s.lines().any(|l| l.trim() == format!("{name}=1")))
        .unwrap_or(false)
}

fn set_flag(name: &str, on: bool) {
    let old = std::fs::read_to_string(settings_path()).unwrap_or_default();
    let mut lines: Vec<String> =
        old.lines().filter(|l| !l.trim().starts_with(&format!("{name}="))).map(String::from).collect();
    lines.push(format!("{name}={}", on as u8));
    let _ = std::fs::write(settings_path(), lines.join("\n") + "\n");
}

pub fn onboarded() -> bool {
    flag("onboarded")
}

pub fn set_onboarded() {
    set_flag("onboarded", true);
}

/// Spell-by-sound mode: expect words spelt the way they sound.
pub fn sound_mode() -> bool {
    flag("spell_by_sound")
}

pub fn set_sound_mode(on: bool) {
    set_flag("spell_by_sound", on);
    quipkeys::decoder::set_sound_mode(on);
}

// ---------------------------------------------------------------- exclusions

static EXCLUDE: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Bumped whenever the list changes, so platform layers re-check the focused app.
pub static EXCLUDE_GEN: AtomicU32 = AtomicU32::new(0);

/// Load the exclusion list (or the platform defaults if there's no file yet).
pub fn load_exclusions(defaults: &[&str]) {
    let list = match std::fs::read_to_string(exclude_path()) {
        Ok(text) => {
            text.lines().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect()
        }
        Err(_) => defaults.iter().map(|s| s.to_string()).collect(),
    };
    *EXCLUDE.lock().unwrap() = list;
    EXCLUDE_GEN.fetch_add(1, Ordering::Relaxed);
}

pub fn is_excluded(app: &str) -> bool {
    !app.is_empty() && EXCLUDE.lock().unwrap().iter().any(|e| e == app)
}

/// Add or remove an app, and save the list.
pub fn set_excluded(app: &str, excluded: bool) {
    let app = app.to_lowercase();
    let mut list = EXCLUDE.lock().unwrap();
    list.retain(|e| *e != app);
    if excluded {
        list.push(app);
    }
    let mut text = String::from(
        "# Apps QuipKeys stays out of: one program name per line, lower case.\n\
         # Edit freely; changes apply after choosing \"Reload\" in the menu or restarting.\n",
    );
    for e in list.iter() {
        text.push_str(e);
        text.push('\n');
    }
    let _ = std::fs::write(exclude_path(), text);
    EXCLUDE_GEN.fetch_add(1, Ordering::Relaxed);
}

/// Make sure the exclusion file exists (so it can be opened for editing).
pub fn write_exclusions_if_missing() {
    if !exclude_path().exists() {
        let first = EXCLUDE.lock().unwrap().first().cloned();
        match first {
            // Rewriting one entry in place saves the whole current list.
            Some(e) => set_excluded(&e, true),
            None => set_excluded("", false),
        }
    }
}

// ------------------------------------------------------------- start at login

#[cfg(windows)]
mod login {
    use windows_sys::Win32::System::Registry::*;

    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
    const NAME: &str = "QuipKeys";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn enabled() -> bool {
        unsafe {
            let mut size = 0u32;
            RegGetValueW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), wide(NAME).as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) == 0
        }
    }

    pub fn set(on: bool) {
        unsafe {
            let mut key: HKEY = std::ptr::null_mut();
            if RegOpenKeyExW(HKEY_CURRENT_USER, wide(RUN_KEY).as_ptr(), 0, KEY_SET_VALUE, &mut key) != 0 {
                return;
            }
            if on {
                let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
                let val = wide(&format!("\"{exe}\""));
                RegSetValueExW(key, wide(NAME).as_ptr(), 0, REG_SZ, val.as_ptr() as *const u8, (val.len() * 2) as u32);
            } else {
                RegDeleteValueW(key, wide(NAME).as_ptr());
            }
            RegCloseKey(key);
        }
    }
}

#[cfg(target_os = "macos")]
mod login {
    use std::path::PathBuf;

    fn plist() -> PathBuf {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        home.join("Library/LaunchAgents/com.quipkeys.agent.plist")
    }

    pub fn enabled() -> bool {
        plist().exists()
    }

    pub fn set(on: bool) {
        let p = plist();
        if !on {
            let _ = std::fs::remove_file(p);
            return;
        }
        let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
        let exe = exe.replace('&', "&amp;").replace('<', "&lt;");
        let text = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>com.quipkeys.agent</string>
  <key>ProgramArguments</key><array><string>{exe}</string></array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#
        );
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, text);
    }
}

pub fn start_at_login() -> bool {
    login::enabled()
}

pub fn set_start_at_login(on: bool) {
    login::set(on)
}

// ------------------------------------------------------------------- helpers

/// Open a web link in the default browser, or a file in the default editor.
pub fn open(target: &str) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let w = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        unsafe {
            ShellExecuteW(std::ptr::null_mut(), w("open").as_ptr(), w(target).as_ptr(), std::ptr::null(), std::ptr::null(), 1);
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = std::process::Command::new("open");
        if !target.contains("://") {
            cmd.arg("-t"); // a file: open it in the default text editor
        }
        let _ = cmd.arg(target).spawn();
    }
}

/// Only one QuipKeys at a time (two would correct every word twice).
/// Returns false if another copy is already running.
pub fn single_instance() -> bool {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows_sys::Win32::System::Threading::CreateMutexW;
        let name: Vec<u16> = "Local\\QuipKeysSingleInstance".encode_utf16().chain(std::iter::once(0)).collect();
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        !h.is_null() && GetLastError() != ERROR_ALREADY_EXISTS
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::fd::IntoRawFd;
        match std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(config_dir().join(".lock")) {
            // The lock is held for as long as the process lives.
            Ok(f) => unsafe { libc::flock(f.into_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 },
            Err(_) => true,
        }
    }
}
