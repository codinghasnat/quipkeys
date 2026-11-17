//! The visible part: a tray icon (Windows) / menu-bar icon (macOS) with a
//! small menu, and the welcome window shown on first run.
//!
//! Key handling never runs here: the platform layer has its own thread. This
//! thread only shows state and passes menu choices back.

use std::sync::{Mutex, OnceLock};

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use wry::{WebView, WebViewBuilder};

#[cfg(target_os = "macos")]
use crate::macos as platform;
use crate::settings;
#[cfg(windows)]
use crate::windows as platform;

const WELCOME: &str = include_str!("welcome.html");
const WORDS: &str = include_str!("words.html");
#[cfg(target_os = "macos")]
const HOTKEY: &str = "Ctrl+Option+Q";
#[cfg(windows)]
const HOTKEY: &str = "Ctrl+Alt+Q";
#[cfg(windows)]
const NEXT_KEY: &str = "Ctrl+Alt+N";
#[cfg(target_os = "macos")]
const NEXT_KEY: &str = "Ctrl+Option+N";

#[derive(Debug)]
#[allow(dead_code)] // some variants are only sent on one platform
pub enum UiEvent {
    /// Correction switched on/off with the hotkey.
    Enabled(bool),
    /// The user started typing in another app (its program name).
    App(String),
    /// The keyboard tap is live (macOS: permission was granted).
    Running,
    Error(String),
    Menu(MenuId),
    /// A message from the welcome or "My words" window.
    Ipc(String),
    /// Words QuipKeys just changed: (what was on screen, what replaced it).
    Fixed(Vec<(String, String)>),
}

/// How long the icon flashes blue after a change.
const FLASH: std::time::Duration = std::time::Duration::from_millis(650);
/// How many recent changes the menu lists.
const RECENT: usize = 5;

static PROXY: OnceLock<Mutex<EventLoopProxy<UiEvent>>> = OnceLock::new();

/// Send an event to the UI thread (from any thread).
pub fn notify(e: UiEvent) {
    if let Some(p) = PROXY.get() {
        let _ = p.lock().unwrap().send_event(e);
    }
}

struct Items {
    status: MenuItem,
    /// "Recent fixes" and its fixed rows (newest first), so you can see what changed.
    recent: Submenu,
    recent_rows: Vec<MenuItem>,
    enabled: CheckMenuItem,
    skip: CheckMenuItem,
    sound: CheckMenuItem,
    learn: CheckMenuItem,
    forget: MenuItem,
    login: CheckMenuItem,
    excluded: MenuItem,
    words: MenuItem,
    welcome: MenuItem,
    quit: MenuItem,
}

/// One of the app's small windows. Fields drop in order: the webview must go
/// before the window it lives in.
struct Page {
    webview: WebView,
    window: Window,
}

struct App {
    tray: Option<TrayIcon>,
    items: Items,
    on: bool,
    running: bool,
    /// Program name of the app last typed in, for "Don't correct in …".
    app: String,
    welcome: Option<Page>,
    /// The "My words" window.
    words: Option<Page>,
    /// Recent changes, newest first; kept in memory only, gone when QuipKeys quits.
    fixes: std::collections::VecDeque<(String, String)>,
    /// The icon shows the "just fixed" colour until then.
    flash_until: Option<std::time::Instant>,
}

pub fn run() {
    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UiEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        // A menu-bar app: no Dock icon.
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }
    PROXY.set(Mutex::new(event_loop.create_proxy())).ok();
    {
        let proxy = Mutex::new(event_loop.create_proxy());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            let _ = proxy.lock().unwrap().send_event(UiEvent::Menu(e.id));
        }));
    }

    let items = Items {
        status: MenuItem::new("QuipKeys is correcting", false, None),
        recent: Submenu::new("Recent fixes", true),
        recent_rows: (0..RECENT).map(|_| MenuItem::new("", false, None)).collect(),
        enabled: CheckMenuItem::new(format!("Correct my typing\t{HOTKEY}"), true, true, None),
        skip: CheckMenuItem::new("Don't correct in this app", false, false, None),
        sound: CheckMenuItem::new("Spell by sound", true, settings::sound_mode(), None),
        learn: CheckMenuItem::new("Learn my typing habits", true, settings::learning(), None),
        forget: MenuItem::new("Forget what it learned", true, None),
        login: CheckMenuItem::new("Start at login", true, settings::start_at_login(), None),
        excluded: MenuItem::new("Edit excluded apps…", true, None),
        words: MenuItem::new("My words…", true, None),
        welcome: MenuItem::new("Welcome & privacy…", true, None),
        quit: MenuItem::new("Quit QuipKeys", true, None),
    };
    for row in &items.recent_rows {
        let _ = items.recent.append(row);
    }
    let _ = items.recent.append(&PredefinedMenuItem::separator());
    let _ = items.recent.append(&MenuItem::new(format!("Wrong word? Press {NEXT_KEY} straight after it"), false, None));
    let mut app = App { tray: None, items, on: true, running: platform::has_permission(), app: String::new(), welcome: None, words: None, fixes: Default::default(), flash_until: None };

    event_loop.run(move |event, target, flow| {
        let now = std::time::Instant::now();
        let window_tick = (app.welcome.is_some() || app.words.is_some()).then(|| now + std::time::Duration::from_millis(700));
        // Wake for whichever comes first: the open window's refresh, or the end of an icon flash.
        *flow = match [window_tick, app.flash_until].into_iter().flatten().min() {
            Some(at) => ControlFlow::WaitUntil(at),
            None => ControlFlow::Wait,
        };
        match event {
            Event::NewEvents(StartCause::Init) => {
                app.tray = Some(build_tray(&app.items));
                app.refresh();
                if !settings::onboarded() || !platform::has_permission() {
                    app.open_welcome(target);
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if app.flash_until.is_some_and(|t| t <= std::time::Instant::now()) {
                    app.flash_until = None;
                    app.refresh();
                }
                app.push_state();
                app.push_words();
            }
            Event::UserEvent(e) => app.handle(e, target, flow),
            Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. } => {
                if app.welcome.as_ref().is_some_and(|p| p.window.id() == window_id) {
                    app.welcome = None;
                }
                if app.words.as_ref().is_some_and(|p| p.window.id() == window_id) {
                    app.words = None;
                }
            }
            _ => {}
        }
    });
}

fn build_tray(items: &Items) -> TrayIcon {
    let menu = Menu::new();
    let _ = menu.append_items(&[
        &items.status,
        &items.recent,
        &PredefinedMenuItem::separator(),
        &items.enabled,
        &items.skip,
        &items.sound,
        &PredefinedMenuItem::separator(),
        &items.learn,
        &items.forget,
        &PredefinedMenuItem::separator(),
        &items.login,
        &items.words,
        &items.excluded,
        &items.welcome,
        &PredefinedMenuItem::separator(),
        &items.quit,
    ]);
    let builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("QuipKeys").with_icon(icon(true));
    #[cfg(target_os = "macos")]
    let builder = builder.with_icon_as_template(true);
    builder.build().expect("could not create the tray icon")
}

impl App {
    fn handle(&mut self, e: UiEvent, target: &EventLoopWindowTarget<UiEvent>, flow: &mut ControlFlow) {
        match e {
            UiEvent::Enabled(on) => {
                self.on = on;
                self.refresh();
            }
            UiEvent::App(name) => {
                self.app = name;
                self.refresh();
            }
            UiEvent::Running => {
                self.running = true;
                self.refresh();
                self.push_state();
            }
            UiEvent::Fixed(changes) => {
                for (from, to) in changes {
                    self.fixes.push_front((from, to));
                }
                self.fixes.truncate(RECENT);
                self.flash_until = Some(std::time::Instant::now() + FLASH);
                self.refresh();
            }
            UiEvent::Error(msg) => {
                self.items.status.set_text(&msg);
            }
            UiEvent::Menu(id) => {
                let it = &self.items;
                if id == *it.enabled.id() {
                    self.on = it.enabled.is_checked();
                    platform::set_enabled(self.on);
                } else if id == *it.skip.id() {
                    settings::set_excluded(&self.app, it.skip.is_checked());
                } else if id == *it.learn.id() {
                    settings::set_learning(it.learn.is_checked());
                    platform::set_learning(it.learn.is_checked());
                } else if id == *it.forget.id() {
                    platform::forget_profile();
                } else if id == *it.sound.id() {
                    settings::set_sound_mode(it.sound.is_checked());
                } else if id == *it.login.id() {
                    settings::set_start_at_login(it.login.is_checked());
                } else if id == *it.excluded.id() {
                    settings::write_exclusions_if_missing();
                    settings::open(&settings::exclude_path().display().to_string());
                } else if id == *it.words.id() {
                    self.open_words(target);
                } else if id == *it.welcome.id() {
                    self.open_welcome(target);
                } else if id == *it.quit.id() {
                    *flow = ControlFlow::Exit;
                }
                self.refresh();
            }
            UiEvent::Ipc(msg) => self.ipc(&msg),
        }
    }

    /// Bring the menu and icon in line with the current state.
    fn refresh(&self) {
        let it = &self.items;
        it.enabled.set_checked(self.on);
        it.status.set_text(if !self.running {
            "Waiting for permission…"
        } else if !self.on {
            "QuipKeys is paused"
        } else {
            "QuipKeys is correcting"
        });
        if self.app.is_empty() {
            it.skip.set_text("Don't correct in this app");
            it.skip.set_enabled(false);
        } else {
            it.skip.set_text(format!("Don't correct in {}", display_name(&self.app)));
            it.skip.set_enabled(true);
            it.skip.set_checked(settings::is_excluded(&self.app));
        }
        it.login.set_checked(settings::start_at_login());
        it.sound.set_checked(settings::sound_mode());
        it.learn.set_checked(settings::learning());
        let short = |s: &str| if s.chars().count() > 28 { format!("{}…", s.chars().take(27).collect::<String>()) } else { s.to_string() };
        for (k, row) in it.recent_rows.iter().enumerate() {
            match self.fixes.get(k) {
                Some((from, to)) => row.set_text(format!("{}  →  {}", short(from), short(to))),
                None if k == 0 => row.set_text("Nothing changed yet"),
                None => row.set_text(""),
            }
        }
        it.recent.set_text(if self.fixes.is_empty() { "Recent fixes".to_string() } else { format!("Recent fixes: {}", short(&self.fixes[0].1)) });
        // Show how much it has to go on; nothing is applied below 2,000 letters.
        let letters = platform::learned_letters();
        it.forget.set_enabled(letters > 0);
        it.forget.set_text(if letters > 0 {
            format!("Forget what it learned ({} words of typing)", letters / 5)
        } else {
            "Forget what it learned".to_string()
        });
        if let Some(t) = &self.tray {
            let flashing = self.flash_until.is_some();
            let _ = t.set_icon(Some(if flashing { icon_flash() } else { icon(self.on && self.running) }));
            // The flash is in colour; the normal menu-bar icon is a tinted template.
            #[cfg(target_os = "macos")]
            t.set_icon_as_template(!flashing);
            let _ = t.set_tooltip(Some(if self.on { "QuipKeys: on" } else { "QuipKeys: paused" }));
        }
    }

    fn open_welcome(&mut self, target: &EventLoopWindowTarget<UiEvent>) {
        if let Some(p) = &self.welcome {
            p.window.set_focus();
            return;
        }
        self.welcome = open_window(target, "Welcome to QuipKeys", (600.0, 660.0), false, WELCOME);
    }

    fn open_words(&mut self, target: &EventLoopWindowTarget<UiEvent>) {
        if let Some(p) = &self.words {
            p.window.set_focus();
            return;
        }
        settings::write_words_if_missing();
        self.words = open_window(target, "My words", (520.0, 640.0), true, WORDS);
    }

    /// Send the saved words to the "My words" window (it redraws only on change).
    fn push_words(&self) {
        let Some(page) = &self.words else { return };
        let js_list = |v: Vec<String>| v.iter().map(|w| js_string(w)).collect::<Vec<_>>().join(",");
        let saved = settings::words();
        // Undone this session and not saved: offered with a Save button.
        let undone: Vec<String> = platform::undone_words().into_iter().filter(|w| !saved.contains(w)).collect();
        let _ = page.webview.evaluate_script(&format!("window.QKW && QKW.list([{}], [{}])", js_list(saved), js_list(undone)));
    }

    /// A message from the "My words" window.
    fn words_ipc(&mut self, msg: &str) {
        let Some(page) = &self.words else { return };
        let call = |js: String| {
            let _ = page.webview.evaluate_script(&js);
        };
        if let Some(raw) = msg.strip_prefix("add:") {
            match settings::add_word(raw) {
                Ok(w) => call(format!("QKW.added({})", js_string(&w))),
                Err(why) => call(format!("QKW.error({})", js_string(why))),
            }
        } else if let Some(raw) = msg.strip_prefix("remove:") {
            settings::remove_word(raw);
            call(format!("QKW.removed({})", js_string(raw.trim())));
        } else if msg == "file" {
            settings::open(&settings::words_path().display().to_string());
        }
        self.push_words();
    }

    fn push_state(&mut self) {
        let Some(Page { webview: wv, .. }) = &self.welcome else { return };
        let js = format!(
            "window.QK && QK.state({{perm: {}, running: {}, login: {}, onboarded: {}, sound: {}}})",
            platform::has_permission(),
            self.running,
            settings::start_at_login(),
            settings::onboarded(),
            settings::sound_mode()
        );
        let _ = wv.evaluate_script(&js);
    }

    fn ipc(&mut self, msg: &str) {
        if let Some(rest) = msg.strip_prefix("words:") {
            return self.words_ipc(rest);
        }
        match msg {
            "ready" => self.push_state(),
            "perm" => platform::request_permission(),
            "sound:1" => settings::set_sound_mode(true),
            "sound:0" => settings::set_sound_mode(false),
            "login:1" => settings::set_start_at_login(true),
            "login:0" => settings::set_start_at_login(false),
            "done" => {
                settings::set_onboarded();
                self.welcome = None;
            }
            "restart" => {
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).arg("--restarted").spawn();
                }
                std::process::exit(0);
            }
            m if m.starts_with("open:https://") => settings::open(&m[5..]),
            _ => {}
        }
        self.refresh();
    }
}

/// A JavaScript string literal for `s`.
fn js_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\u003c"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Open a small window showing one of the built-in pages.
fn open_window(
    target: &EventLoopWindowTarget<UiEvent>,
    title: &str,
    size: (f64, f64),
    resizable: bool,
    html: &str,
) -> Option<Page> {
    let size = tao::dpi::LogicalSize::new(size.0, size.1);
    let window = WindowBuilder::new()
        .with_title(title)
        .with_inner_size(size)
        .with_resizable(resizable)
        .with_maximizable(false)
        .build(target)
        .ok()?;
    if let Some(m) = window.current_monitor() {
        let scale = m.scale_factor();
        let (mw, mh) = (m.size().width as f64, m.size().height as f64);
        let (ww, wh) = (size.width * scale, size.height * scale);
        let p = m.position();
        window.set_outer_position(tao::dpi::PhysicalPosition::new(p.x as f64 + (mw - ww) / 2.0, p.y as f64 + (mh - wh) / 2.5));
    }
    let platform_js = format!("window.QK_PLATFORM = '{}';", if cfg!(target_os = "macos") { "mac" } else { "win" });
    let proxy = Mutex::new(PROXY.get().unwrap().lock().unwrap().clone());
    let webview = WebViewBuilder::new()
        .with_html(html)
        .with_initialization_script(&platform_js)
        .with_ipc_handler(move |req: wry::http::Request<String>| {
            let _ = proxy.lock().unwrap().send_event(UiEvent::Ipc(req.body().clone()));
        })
        // Links open in the real browser, never inside the window.
        .with_navigation_handler(|url| url.starts_with("about:") || url.starts_with("data:"))
        .build(&window)
        .ok()?;
    window.set_focus();
    Some(Page { webview, window })
}

/// "notepad.exe" -> "Notepad", "google chrome" -> "Google Chrome".
fn display_name(program: &str) -> String {
    let base = program.strip_suffix(".exe").unwrap_or(program);
    base.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The QuipKeys logo (docs/assets/logo.svg): a keycap with a Q on it, drawn
/// at 64x64 with anti-aliasing. Windows gets the colour version (greyed out
/// while paused). macOS gets a template image, a keycap silhouette with the Q
/// cut out, which the system tints to suit the menu bar (faded while paused).
/// The icon for a moment after a fix: the keycap in autocorrect blue.
fn icon_flash() -> Icon {
    icon_colours(true, [0x1a, 0x56, 0xd6], [0xff, 0xff, 0xff], false)
}

fn icon(on: bool) -> Icon {
    let yellow = if on { [0xffu8, 0xe1, 0x4a] } else { [0xc9u8, 0xc3, 0xa8] };
    icon_colours(on, yellow, [0x17, 0x14, 0x0a], cfg!(target_os = "macos"))
}

/// Draw the logo: `top` is the keycap colour, `ink` the Q and base colour.
/// `template`: a silhouette with the Q cut out (macOS tints it).
fn icon_colours(on: bool, top_colour: [u8; 3], ink: [u8; 3], template: bool) -> Icon {
    const N: usize = 64;
    // Shapes in the SVG's 64-unit coordinates.
    let rrect = |x: f32, y: f32, x0: f32, y0: f32, w: f32, h: f32, r: f32| -> bool {
        let cx = x.clamp(x0 + r, x0 + w - r);
        let cy = y.clamp(y0 + r, y0 + h - r);
        (x - cx).powi(2) + (y - cy).powi(2) <= r * r && x >= x0 && x <= x0 + w && y >= y0 && y <= y0 + h
    };
    let base = |x: f32, y: f32| rrect(x, y, 4.0, 8.0, 56.0, 52.0, 13.0);
    let top = |x: f32, y: f32| rrect(x, y, 8.0, 4.0, 48.0, 46.0, 10.0);
    let q = |x: f32, y: f32| -> bool {
        let d = ((x - 30.0).powi(2) + (y - 25.5).powi(2)).sqrt();
        if (8.0..=14.0).contains(&d) {
            return true;
        }
        // Tail: square start inside the ring, round end outside.
        let (ax, ay, bx, by) = (32.5f32, 28.0f32, 44.0f32, 39.5f32);
        let t = ((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / ((bx - ax).powi(2) + (by - ay).powi(2));
        if t < 0.0 {
            return false;
        }
        let t = t.min(1.0);
        ((x - ax - t * (bx - ax)).powi(2) + (y - ay - t * (by - ay)).powi(2)).sqrt() <= 3.25
    };
    let yellow = top_colour;
    let mut px = vec![0u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            // 4x4 supersampling: average the colour of 16 sample points.
            let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            for sy in 0..4 {
                for sx in 0..4 {
                    let (fx, fy) = (x as f32 + (sx as f32 + 0.5) / 4.0, y as f32 + (sy as f32 + 0.5) / 4.0);
                    let c: Option<[u8; 3]> = if template {
                        // Silhouette with the Q cut out.
                        ((base(fx, fy) || top(fx, fy)) && !(top(fx, fy) && q(fx, fy))).then_some([0, 0, 0])
                    } else if top(fx, fy) {
                        Some(if q(fx, fy) { ink } else { yellow })
                    } else if base(fx, fy) {
                        Some(ink)
                    } else {
                        None
                    };
                    if let Some(c) = c {
                        r += c[0] as u32;
                        g += c[1] as u32;
                        b += c[2] as u32;
                        a += 1;
                    }
                }
            }
            let i = (y * N + x) * 4;
            if let (Some(r), Some(g), Some(b)) = (r.checked_div(a), g.checked_div(a), b.checked_div(a)) {
                px[i] = r as u8;
                px[i + 1] = g as u8;
                px[i + 2] = b as u8;
            }
            let fade = if template && !on { 0.4 } else { 1.0 };
            px[i + 3] = (a as f32 / 16.0 * 255.0 * fade) as u8;
        }
    }
    Icon::from_rgba(px, N as u32, N as u32).expect("valid icon")
}
