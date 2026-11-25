//! QuipKeys background app. Watches typing system-wide and corrects each word
//! as you press space. Lives in the tray (Windows) / menu bar (macOS).
//! Ctrl+Alt+Q (Windows) / Ctrl+Option+Q (macOS) toggles it; Backspace straight
//! after a correction undoes it.

// No console window on Windows in release builds.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]

mod ai;
mod compose;
mod core;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(windows, target_os = "macos"))]
mod settings;
mod stats;
mod update;
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
    stats::set_utc_offset(platform::utc_offset_seconds());
    stats::init(settings::config_dir().join("quipkeys-stats.txt"));
    update::clean_up();
    ui::start_update_checks();
    quipkeys::decoder::set_sound_mode(settings::sound_mode());
    platform::start(core::Core::open(Some(settings::words_path()), Some(settings::profile_path()), settings::learning()));
    ui::run();
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    let _ = core::Core::with_words(None);
    eprintln!("QuipKeys: this platform isn't supported yet (Windows and macOS only).");
}
