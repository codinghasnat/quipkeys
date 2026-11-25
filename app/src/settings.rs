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
    if !words_path().exists() {
        let _ = std::fs::write(words_path(), WORDS_HEADER);
    }
}

const WORDS_HEADER: &str = "# Your words: QuipKeys never changes these, and fixes typos of them.\n\
# Add and remove them in the My words window, or edit this file: one word per line.\n";

/// A word as it is stored: lower case; letters, apostrophes and hyphens; 2-40 long.
pub fn clean_word(raw: &str) -> Option<String> {
    let w = raw.trim().to_lowercase();
    let ok = (2..=40).contains(&w.chars().count())
        && w.chars().all(|c| c.is_ascii_lowercase() || c == '\'' || c == '-')
        && w.chars().any(|c| c.is_ascii_lowercase());
    ok.then_some(w)
}

/// The saved words, oldest first.
pub fn words() -> Vec<String> {
    let text = std::fs::read_to_string(words_path()).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    for w in text.lines().filter(|l| !l.trim_start().starts_with('#')).filter_map(clean_word) {
        if !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// Add a word. Ok(word as saved), or Err(why not). Appends one line; the
/// rest of the file (comments, hand-added entries) is left exactly as it is.
pub fn add_word(raw: &str) -> Result<String, &'static str> {
    let w = clean_word(raw).ok_or("Use one word: letters only, at least 2 of them.")?;
    if words().contains(&w) {
        return Err("That word is already in your list.");
    }
    let mut text = std::fs::read_to_string(words_path()).unwrap_or_else(|_| WORDS_HEADER.to_string());
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&w);
    text.push('\n');
    std::fs::write(words_path(), text).map_err(|_| "Couldn't save your word list.")?;
    Ok(w)
}

/// Remove a word: drops only the lines that are that word.
pub fn remove_word(raw: &str) {
    let w = raw.trim().to_lowercase();
    let Ok(text) = std::fs::read_to_string(words_path()) else { return };
    let kept: Vec<&str> =
        text.lines().filter(|l| l.trim_start().starts_with('#') || clean_word(l).as_deref() != Some(w.as_str())).collect();
    let _ = std::fs::write(words_path(), kept.join("\n") + "\n");
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

/// The typing profile: tallies of the user's kinds of slips (counts, no text).
pub fn profile_path() -> PathBuf {
    config_dir().join("quipkeys-profile.txt")
}

/// Learn the user's typing habits (on unless switched off).
pub fn learning() -> bool {
    !flag("no_learning")
}

pub fn set_learning(on: bool) {
    set_flag("no_learning", !on);
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

// ---------------------------------------------------------- write it for me

/// A text setting from quipkeys-settings.txt ("name=value").
fn value(name: &str) -> Option<String> {
    let text = std::fs::read_to_string(settings_path()).ok()?;
    text.lines().find_map(|l| l.strip_prefix(&format!("{name}="))).map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

fn set_value(name: &str, v: &str) {
    let old = std::fs::read_to_string(settings_path()).unwrap_or_default();
    let mut lines: Vec<String> =
        old.lines().filter(|l| !l.trim().starts_with(&format!("{name}="))).map(String::from).collect();
    let v: String = v.chars().filter(|c| !c.is_control()).collect();
    if !v.trim().is_empty() {
        lines.push(format!("{name}={}", v.trim()));
    }
    let _ = std::fs::write(settings_path(), lines.join("\n") + "\n");
}

/// API keys, one per provider ("openai=sk-..."), in their own file (readable
/// only by you on macOS), never in the settings file. Environment variables
/// (ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY, XAI_API_KEY,
/// DEEPSEEK_API_KEY) work too.
fn keys_path() -> PathBuf {
    config_dir().join("quipkeys-api-keys.txt")
}

fn read_keys() -> Vec<(String, String)> {
    let mut keys: Vec<(String, String)> = std::fs::read_to_string(keys_path())
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('=').map(|(a, b)| (a.trim().to_string(), b.trim().to_string())))
        .filter(|(_, k)| !k.is_empty())
        .collect();
    // A Claude key saved by an earlier version.
    if !keys.iter().any(|(v, _)| v == "claude") {
        if let Ok(k) = std::fs::read_to_string(config_dir().join("quipkeys-claude-key.txt")) {
            if !k.trim().is_empty() {
                keys.push(("claude".into(), k.trim().to_string()));
            }
        }
    }
    keys
}

pub fn api_key(vendor: &str) -> String {
    if let Some((_, k)) = read_keys().into_iter().find(|(v, _)| v == vendor) {
        return k;
    }
    let env = match vendor {
        "claude" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        "gemini" => "GEMINI_API_KEY",
        "grok" => "XAI_API_KEY",
        "deepseek" => "DEEPSEEK_API_KEY",
        _ => return String::new(),
    };
    std::env::var(env).unwrap_or_default()
}

pub fn has_api_key(vendor: &str) -> bool {
    !api_key(vendor).trim().is_empty()
}

/// Save a provider's key (an empty string removes it).
pub fn set_api_key(vendor: &str, key: &str) {
    let key: String = key.trim().chars().filter(|c| !c.is_control() && !c.is_whitespace()).collect();
    let mut keys: Vec<(String, String)> = read_keys().into_iter().filter(|(v, _)| v != vendor).collect();
    if !key.is_empty() {
        keys.push((vendor.to_string(), key));
    }
    let mut text = String::from("# Your API keys for Write it for me: one per line, provider=key. Keep this file private.\n");
    for (v, k) in &keys {
        text.push_str(&format!("{v}={k}\n"));
    }
    let _ = std::fs::write(keys_path(), text);
    let _ = std::fs::remove_file(config_dir().join("quipkeys-claude-key.txt"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(keys_path(), std::fs::Permissions::from_mode(0o600));
    }
}

/// The chosen provider ("off" until the user picks one).
pub fn ai_kind() -> String {
    value("ai").filter(|k| crate::ai::vendor(k).is_some()).unwrap_or_else(|| "off".into())
}

/// The model chosen for a provider ("" = not chosen yet).
pub fn ai_model(vendor: &str) -> String {
    value(&format!("ai_model_{vendor}")).unwrap_or_default()
}

/// The address for a custom or local service ("" = the default).
pub fn ai_url(vendor: &str) -> String {
    value(&format!("ai_url_{vendor}")).unwrap_or_default()
}

/// The model "write it for me" uses (off until the user picks one).
pub fn ai_provider() -> crate::ai::Provider {
    use crate::ai::{vendor, Provider};
    let kind = ai_kind();
    let Some(v) = vendor(&kind) else { return Provider::Off };
    let model = Some(ai_model(&kind)).filter(|m| !m.is_empty()).or_else(|| v.defaults.first().map(|d| d.to_string())).unwrap_or_default();
    if kind == "claude" {
        return Provider::Claude { api_key: api_key("claude"), model };
    }
    let url = Some(ai_url(&kind)).filter(|u| !u.is_empty()).unwrap_or_else(|| v.chat_url.to_string());
    let who = match kind.as_str() {
        "local" => "the local model".to_string(),
        "custom" => "the custom service".to_string(),
        _ => v.name.to_string(),
    };
    Provider::Chat { who, url, api_key: api_key(&kind), model }
}

/// Use the text around the cursor and on screen as context (on unless switched off).
pub fn ai_screen() -> bool {
    !flag("ai_no_screen")
}

/// Save the settings window's choices for one provider.
pub fn set_ai(kind: &str, model: &str, url: &str, screen: bool) {
    let kind = if crate::ai::vendor(kind).is_some() { kind } else { "off" };
    set_value("ai", kind);
    if kind != "off" {
        set_value(&format!("ai_model_{kind}"), model);
        if matches!(kind, "custom" | "local") {
            set_value(&format!("ai_url_{kind}"), url);
        }
    }
    set_flag("ai_no_screen", !screen);
}

/// Check for updates once a day by itself (off unless switched on: it's the
/// only thing besides Write it for me that would use the internet).
pub fn auto_update() -> bool {
    flag("auto_update")
}

pub fn set_auto_update(on: bool) {
    set_flag("auto_update", on);
}
