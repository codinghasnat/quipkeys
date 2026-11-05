//! The visible part: a tray icon (Windows) / menu-bar icon (macOS) with a
//! small menu, and the welcome window shown on first run.
//!
//! Key handling never runs here: the platform layer has its own thread. This
//! thread only shows state and passes menu choices back.

use std::sync::{Mutex, OnceLock};

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use wry::{WebView, WebViewBuilder};

#[cfg(target_os = "macos")]
use crate::macos as platform;
use crate::settings;
#[cfg(windows)]
use crate::windows as platform;

const WELCOME: &str = include_str!("welcome.html");
#[cfg(target_os = "macos")]
const HOTKEY: &str = "Ctrl+Option+Q";
#[cfg(windows)]
const HOTKEY: &str = "Ctrl+Alt+Q";

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
    /// A message from the welcome window.
    Ipc(String),
}

static PROXY: OnceLock<Mutex<EventLoopProxy<UiEvent>>> = OnceLock::new();

/// Send an event to the UI thread (from any thread).
pub fn notify(e: UiEvent) {
    if let Some(p) = PROXY.get() {
        let _ = p.lock().unwrap().send_event(e);
    }
}

struct Items {
    status: MenuItem,
    enabled: CheckMenuItem,
    skip: CheckMenuItem,
    login: CheckMenuItem,
    excluded: MenuItem,
    words: MenuItem,
    welcome: MenuItem,
    quit: MenuItem,
}

struct App {
    tray: Option<TrayIcon>,
    items: Items,
    on: bool,
    running: bool,
    /// Program name of the app last typed in, for "Don't correct in …".
    app: String,
    welcome: Option<(Window, WebView)>,
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
        enabled: CheckMenuItem::new(format!("Correct my typing\t{HOTKEY}"), true, true, None),
        skip: CheckMenuItem::new("Don't correct in this app", false, false, None),
        login: CheckMenuItem::new("Start at login", true, settings::start_at_login(), None),
        excluded: MenuItem::new("Edit excluded apps…", true, None),
        words: MenuItem::new("My words…", true, None),
        welcome: MenuItem::new("Welcome & privacy…", true, None),
        quit: MenuItem::new("Quit QuipKeys", true, None),
    };
    let mut app = App { tray: None, items, on: true, running: platform::has_permission(), app: String::new(), welcome: None };

    event_loop.run(move |event, target, flow| {
        *flow = if app.welcome.is_some() {
            // Keep the welcome window's permission status fresh.
            ControlFlow::WaitUntil(std::time::Instant::now() + std::time::Duration::from_millis(700))
        } else {
            ControlFlow::Wait
        };
        match event {
            Event::NewEvents(StartCause::Init) => {
                app.tray = Some(build_tray(&app.items));
                app.refresh();
                if !settings::onboarded() || !platform::has_permission() {
                    app.open_welcome(target);
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => app.push_state(),
            Event::UserEvent(e) => app.handle(e, target, flow),
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                app.welcome = None;
            }
            _ => {}
        }
    });
}

fn build_tray(items: &Items) -> TrayIcon {
    let menu = Menu::new();
    let _ = menu.append_items(&[
        &items.status,
        &PredefinedMenuItem::separator(),
        &items.enabled,
        &items.skip,
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
                } else if id == *it.login.id() {
                    settings::set_start_at_login(it.login.is_checked());
                } else if id == *it.excluded.id() {
                    settings::write_exclusions_if_missing();
                    settings::open(&settings::exclude_path().display().to_string());
                } else if id == *it.words.id() {
                    settings::write_words_if_missing();
                    settings::open(&settings::words_path().display().to_string());
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
        if let Some(t) = &self.tray {
            let _ = t.set_icon(Some(icon(self.on && self.running)));
            #[cfg(target_os = "macos")]
            t.set_icon_as_template(true);
            let _ = t.set_tooltip(Some(if self.on { "QuipKeys: on" } else { "QuipKeys: paused" }));
        }
    }

    fn open_welcome(&mut self, target: &EventLoopWindowTarget<UiEvent>) {
        if let Some((w, _)) = &self.welcome {
            w.set_focus();
            return;
        }
        let size = tao::dpi::LogicalSize::new(600.0, 660.0);
        let window = match WindowBuilder::new()
            .with_title("Welcome to QuipKeys")
            .with_inner_size(size)
            .with_resizable(false)
            .with_maximizable(false)
            .build(target)
        {
            Ok(w) => w,
            Err(_) => return,
        };
        if let Some(m) = window.current_monitor() {
            let scale = m.scale_factor();
            let (mw, mh) = (m.size().width as f64, m.size().height as f64);
            let (ww, wh) = (size.width * scale, size.height * scale);
            let p = m.position();
            window.set_outer_position(tao::dpi::PhysicalPosition::new(p.x as f64 + (mw - ww) / 2.0, p.y as f64 + (mh - wh) / 2.5));
        }
        let platform_js = format!(
            "window.QK_PLATFORM = '{}';",
            if cfg!(target_os = "macos") { "mac" } else { "win" }
        );
        let proxy = Mutex::new(PROXY.get().unwrap().lock().unwrap().clone());
        let webview = WebViewBuilder::new()
            .with_html(WELCOME)
            .with_initialization_script(&platform_js)
            .with_ipc_handler(move |req: wry::http::Request<String>| {
                let _ = proxy.lock().unwrap().send_event(UiEvent::Ipc(req.body().clone()));
            })
            // Links open in the real browser, never inside the window.
            .with_navigation_handler(|url| url.starts_with("about:") || url.starts_with("data:"))
            .build(&window);
        let Ok(webview) = webview else { return };
        window.set_focus();
        self.welcome = Some((window, webview));
    }

    fn push_state(&mut self) {
        let Some((_, wv)) = &self.welcome else { return };
        let js = format!(
            "window.QK && QK.state({{perm: {}, running: {}, login: {}, onboarded: {}}})",
            platform::has_permission(),
            self.running,
            settings::start_at_login(),
            settings::onboarded()
        );
        let _ = wv.evaluate_script(&js);
    }

    fn ipc(&mut self, msg: &str) {
        match msg {
            "ready" => self.push_state(),
            "perm" => platform::request_permission(),
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

/// The QuipKeys mark (a "Q": a ring with a tail), drawn at 64×64 with
/// anti-aliasing. On macOS it's a template image (the system colours it);
/// on Windows it's blue when on and grey when paused.
fn icon(on: bool) -> Icon {
    const N: usize = 64;
    let (r, g, b) = if cfg!(target_os = "macos") {
        (0, 0, 0)
    } else if on {
        (0x3b, 0x6c, 0xf6)
    } else {
        (0x8a, 0x8f, 0x98)
    };
    let alpha_scale = if cfg!(target_os = "macos") && !on { 0.4 } else { 1.0 };
    let mut px = vec![0u8; N * N * 4];
    let inside = |x: f32, y: f32| -> bool {
        let (cx, cy) = (28.0, 28.0);
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        let ring = (15.0..=24.0).contains(&d);
        // Tail: a thick segment crossing the ring, from (35, 35) to (56, 56).
        let (ax, ay, bx, by) = (35.0f32, 35.0f32, 56.0f32, 56.0f32);
        let t = (((x - ax) * (bx - ax) + (y - ay) * (by - ay)) / ((bx - ax).powi(2) + (by - ay).powi(2))).clamp(0.0, 1.0);
        let (qx, qy) = (ax + t * (bx - ax), ay + t * (by - ay));
        let tail = ((x - qx).powi(2) + (y - qy).powi(2)).sqrt() <= 5.0;
        ring || tail
    };
    for y in 0..N {
        for x in 0..N {
            let mut hits = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    if inside(x as f32 + (sx as f32 + 0.5) / 4.0, y as f32 + (sy as f32 + 0.5) / 4.0) {
                        hits += 1;
                    }
                }
            }
            let i = (y * N + x) * 4;
            px[i] = r;
            px[i + 1] = g;
            px[i + 2] = b;
            px[i + 3] = (hits as f32 / 16.0 * 255.0 * alpha_scale) as u8;
        }
    }
    Icon::from_rgba(px, N as u32, N as u32).expect("valid icon")
}
