//! QuipKeys background app. Watches typing system-wide and corrects each word
//! as you press space. Ctrl+Alt+Q (Windows) / Ctrl+Option+Q (macOS) toggles it;
//! Backspace straight after a correction undoes it.

#![cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]

mod core;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

fn main() {
    eprintln!("QuipKeys: loading the engine...");
    let t = std::time::Instant::now();
    let core = core::Core::new();
    eprintln!("QuipKeys: ready in {:.1} s. Toggle: Ctrl+Alt+Q (Windows) / Ctrl+Option+Q (macOS).", t.elapsed().as_secs_f32());
    #[cfg(windows)]
    windows::run(core);
    #[cfg(target_os = "macos")]
    macos::run(core);
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = core;
        eprintln!("QuipKeys: this platform isn't supported yet (Windows and macOS only).");
    }
}
