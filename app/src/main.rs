//! QuipKeys background app. Watches typing system-wide and corrects each word
//! as you press space. Lives in the tray (Windows) / menu bar (macOS).
//! Ctrl+Alt+Q (Windows) / Ctrl+Option+Q (macOS) toggles it; Backspace straight
//! after a correction undoes it.

// No console window on Windows in release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]

mod core;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, target_os = "macos"))]
mod settings;
#[cfg(any(windows, target_os = "macos"))]
mod ui;
#[cfg(windows)]
mod windows;

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    #[cfg(target_os = "macos")]
    use crate::macos as platform;
    #[cfg(windows)]
    use crate::windows as platform;

    // After "Restart QuipKeys" the old copy may still be exiting: wait for it.
    let patience = if std::env::args().any(|a| a == "--restarted") { 30 } else { 1 };
    let mut alone = false;
    for _ in 0..patience {
        alone = settings::single_instance();
        if alone {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if !alone {
        eprintln!("QuipKeys is already running (look for its icon in the tray / menu bar).");
        return;
    }
    settings::load_exclusions(platform::DEFAULT_EXCLUDE);
    quipkeys::decoder::set_sound_mode(settings::sound_mode());
    platform::start(core::Core::with_words(Some(settings::words_path())));
    ui::run();
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    let _ = core::Core::with_words(None);
    eprintln!("QuipKeys: this platform isn't supported yet (Windows and macOS only).");
}
