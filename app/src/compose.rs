//! The "write it for me" hotkey, start to finish.
//!
//! 1. Type what you want ("reply saying Friday works, many thanks"). Copying
//!    something it should know is optional: QuipKeys also reads what's around
//!    the cursor and on screen (if allowed).
//! 2. Press the hotkey. QuipKeys asks the chosen model (ai.rs) and, as the
//!    text streams in, removes your instruction and pastes the text in its
//!    place a sentence at a time, then puts your clipboard back.
//!
//! Safety rules:
//! * The instruction is deleted only when QuipKeys knows exactly where it is:
//!   typed just now (it counts the characters), or typed before a click, in
//!   which case it is selected and *checked* before anything is replaced.
//! * If you carry on typing or click while it's writing, nothing more is
//!   pasted; the rest is left on the clipboard instead.
//! * Copied text is used only if it was copied in the last [`CONTEXT_FRESH`],
//!   and never if it looks like a password or key. Screen text is never read
//!   in password fields (the platform layer) or banking and password apps.
//!
//! The platform layers supply [`Ops`]; the sequence itself is here so it can
//! be tested without a keyboard.

use std::time::Duration;

use crate::ai::{Request, Screen};
use crate::core::{ComposeReq, Grab};

/// Copied text older than this isn't used as context.
pub const CONTEXT_FRESH: Duration = Duration::from_secs(15 * 60);

/// What the platform layer can do for the compose sequence.
pub trait Ops {
    /// The clipboard's text, if it holds text.
    fn clipboard_text(&self) -> Option<String>;
    /// How long ago the clipboard last changed, if known.
    fn clipboard_age(&self) -> Option<Duration>;
    fn set_clipboard(&self, text: &str);
    /// What can be read around the cursor and on screen (accessibility APIs).
    /// None in password fields or when nothing can be read.
    fn screen(&self) -> Option<Screen>;
    /// Press Backspace `n` times.
    fn backspace(&self, n: usize);
    /// Shift+Left `n` times: select the `n` characters before the caret.
    fn select_left(&self, n: usize);
    /// Collapse a selection to its end (Right arrow).
    fn deselect(&self);
    /// Ctrl+C / Cmd+C.
    fn copy(&self);
    /// Ctrl+V / Cmd+V.
    fn paste(&self);
    /// The core's key generation (see Core::generation).
    fn generation(&self) -> u64;
    fn wait(&self, ms: u64);
    fn status(&self, s: Status);
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Asking the model.
    Writing,
    /// The text is in.
    Done,
    /// Nothing was changed (or, if `partial`, it stopped partway).
    Failed(String),
    /// The text couldn't (all) go in safely; it's on the clipboard.
    OnClipboard(String),
}

/// Text from the clipboard that may be used as context.
pub fn context(ops: &dyn Ops, instruction: &str) -> Option<String> {
    let text = ops.clipboard_text()?;
    let t = text.trim();
    if t.is_empty() || t == instruction.trim() {
        return None;
    }
    if ops.clipboard_age().is_some_and(|a| a > CONTEXT_FRESH) {
        return None;
    }
    // One "word" of up to 100 characters: a password, a key, a code, a link.
    // Not worth the risk of sending it anywhere.
    if !t.contains(char::is_whitespace) && t.chars().count() <= 100 {
        return None;
    }
    Some(text)
}

/// Apps and pages whose screen text is never read (the app and window name
/// still set the tone).
pub fn private_place(app: &str, title: &str) -> bool {
    let s = format!("{} {}", app.to_lowercase(), title.to_lowercase());
    ["bank", "paypal", "venmo", "revolut", "monzo", "wise.com", "wallet", "crypto", "coinbase", "1password", "bitwarden", "keychain", "lastpass", "keepass"]
        .iter()
        .any(|w| s.contains(w))
}

/// The screen as it may be sent: without the instruction itself (it's in the
/// text box, just before the cursor) and without private places' text.
fn screen_for(ops: &dyn Ops, instruction: &str) -> Option<Screen> {
    let mut s = ops.screen()?;
    if private_place(&s.app, &s.title) {
        return Some(Screen { app: s.app, title: s.title, ..Default::default() });
    }
    let i = instruction.trim();
    if !i.is_empty() {
        let b = s.before.trim_end();
        if let Some(rest) = b.strip_suffix(i) {
            s.before = rest.to_string();
        }
        if let Some(pos) = s.window.rfind(i) {
            s.window.replace_range(pos..pos + i.len(), "");
        }
    }
    Some(s)
}

/// Asks the model, handing each new piece of text to the callback; returns
/// the whole text or why not.
pub type Writer<'a> = &'a dyn Fn(&Request, &mut dyn FnMut(&str)) -> Result<String, String>;

/// Where to cut streamed text into pastes: after a sentence or a line, once
/// there's enough to be worth a paste. None: wait for more.
fn cut_point(s: &str, min: usize) -> Option<usize> {
    if s.chars().count() < min {
        return None;
    }
    let b = s.as_bytes();
    // Cut after a newline, or after the space that follows punctuation.
    (1..b.len()).rev().find_map(|i| {
        if b[i - 1] == b'\n' {
            Some(i)
        } else if matches!(b[i - 1], b'.' | b'!' | b'?' | b',' | b';' | b':') && b[i] == b' ' {
            Some(i + 1)
        } else {
            None
        }
    })
}

/// Pastes streamed text into the field a piece at a time.
struct Paster<'a> {
    ops: &'a dyn Ops,
    grab: &'a Grab,
    gen0: u64,
    pending: String,
    /// The instruction has been removed and pasting has begun.
    started: bool,
    /// Pasting stopped (the user typed, or the instruction couldn't be found):
    /// everything from here on goes to the clipboard, with this reason.
    stopped: Option<String>,
    leftover: String,
}

impl Paster<'_> {
    fn feed(&mut self, piece: &str) {
        self.pending.push_str(piece);
        // A bigger first piece (one clean paste to start), then sentence by sentence.
        let min = if self.started { 24 } else { 60 };
        if let Some(cut) = cut_point(&self.pending, min) {
            let chunk: String = self.pending.drain(..cut).collect();
            self.emit(chunk);
        }
    }

    fn finish(&mut self) {
        let rest = std::mem::take(&mut self.pending);
        let rest = rest.trim_end().to_string();
        if !rest.is_empty() {
            self.emit(rest);
        }
    }

    fn emit(&mut self, mut chunk: String) {
        if self.stopped.is_none() && self.ops.generation() != self.gen0 {
            self.stopped = Some(if self.started {
                "You carried on typing, so the rest of the text is on your clipboard.".into()
            } else {
                "You carried on typing, so the text is on your clipboard instead.".into()
            });
        }
        if self.stopped.is_none() && !self.started {
            if let Err(why) = self.remove_instruction() {
                self.stopped = Some(why);
            }
            self.started = self.stopped.is_none();
            chunk = chunk.trim_start().to_string();
        }
        if self.stopped.is_some() {
            self.leftover.push_str(&chunk);
            return;
        }
        if chunk.is_empty() {
            return;
        }
        self.ops.set_clipboard(&chunk);
        self.ops.wait(40);
        self.ops.paste();
        // Let the app read the clipboard before the next piece replaces it.
        self.ops.wait(150);
    }

    fn remove_instruction(&self) -> Result<(), String> {
        match self.grab {
            Grab::Typed(n) => self.ops.backspace(*n),
            Grab::Earlier(instruction) => {
                self.ops.select_left(instruction.chars().count());
                self.ops.wait(40);
                self.ops.copy();
                self.ops.wait(150);
                if self.ops.clipboard_text().as_deref() != Some(instruction.as_str()) {
                    self.ops.deselect();
                    return Err("Your instruction wasn't right before the cursor, so the text is on your clipboard: paste it where you want it.".into());
                }
            }
            Grab::Nothing => {}
        }
        Ok(())
    }
}

/// Run one compose request. `write` asks the model (ai::write in the app);
/// `read_screen` allows context from around the cursor and on screen.
pub fn run(req: &ComposeReq, gen0: u64, ops: &dyn Ops, write: Writer, read_screen: bool) {
    let copied = context(ops, &req.instruction);
    let screen = if read_screen { screen_for(ops, &req.instruction) } else { None };
    let has_context = copied.is_some() || screen.as_ref().is_some_and(Screen::has_text);
    if req.instruction.is_empty() && !has_context {
        ops.status(Status::Failed("Type what you'd like written (and copy anything it should know), then press the hotkey.".into()));
        return;
    }
    ops.status(Status::Writing);
    let started_at = std::time::Instant::now();
    let saved = ops.clipboard_text();
    let request = Request { instruction: req.instruction.clone(), copied, screen };
    let mut paster = Paster { ops, grab: &req.grab, gen0, pending: String::new(), started: false, stopped: None, leftover: String::new() };
    let result = write(&request, &mut |piece| paster.feed(piece));
    let written = result.as_ref().map(|t| t.trim().chars().count()).unwrap_or(0);
    match result {
        Err(why) if !paster.started => {
            // Nothing on screen changed; the clipboard may hold a check copy.
            if paster.stopped.is_some() {
                if let Some(s) = &saved {
                    ops.set_clipboard(s);
                }
            }
            ops.status(Status::Failed(why))
        }
        Err(why) => {
            if let Some(s) = &saved {
                ops.set_clipboard(s);
            }
            ops.status(Status::Failed(format!("It stopped partway: {why}")))
        }
        Ok(_) => {
            paster.finish();
            if let Some(why) = paster.stopped {
                ops.set_clipboard(paster.leftover.trim());
                ops.status(Status::OnClipboard(why));
            } else {
                // Give the app time to read the last piece before the clipboard is put back.
                ops.wait(250);
                if let Some(s) = &saved {
                    ops.set_clipboard(s);
                }
                let waited = started_at.elapsed().as_millis() as u64;
                crate::stats::with(|s| s.wrote(crate::stats::today(), written, req.instruction.chars().count(), waited));
                ops.status(Status::Done);
            }
        }
    }
}

// ------------------------------------------------- the app's side (not tests)

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

static BUSY: AtomicBool = AtomicBool::new(false);
static CLIP_SEQ: AtomicU64 = AtomicU64::new(0);
static CLIP_CHANGED: Mutex<Option<Instant>> = Mutex::new(None);

/// Watch the clipboard's change counter (never its contents) so context can
/// be limited to recent copies. `seq` reads the platform's counter.
pub fn watch_clipboard(seq: fn() -> u64) {
    *CLIP_CHANGED.lock().unwrap() = Some(Instant::now());
    CLIP_SEQ.store(seq(), Ordering::Relaxed);
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(500));
        let s = seq();
        if CLIP_SEQ.swap(s, Ordering::Relaxed) != s {
            *CLIP_CHANGED.lock().unwrap() = Some(Instant::now());
        }
    });
}

/// How long since the clipboard last changed (see `watch_clipboard`).
pub fn clipboard_age() -> Option<Duration> {
    CLIP_CHANGED.lock().unwrap().map(|t| t.elapsed())
}

/// Start a compose request on its own thread (one at a time). `ops` does the
/// platform's keyboard and clipboard work.
#[cfg(any(windows, target_os = "macos"))]
pub fn spawn(req: ComposeReq, gen0: u64, ops: Box<dyn Ops + Send>) {
    use crate::ai::Provider;
    use crate::ui::{notify, UiEvent};
    if BUSY.swap(true, Ordering::SeqCst) {
        return; // already writing
    }
    std::thread::spawn(move || {
        let provider = crate::settings::ai_provider();
        if provider == Provider::Off {
            notify(UiEvent::OpenCompose);
            ops.status(Status::Failed("Choose a model for Write it for me first.".into()));
        } else {
            let read_screen = crate::settings::ai_screen();
            run(&req, gen0, ops.as_ref(), &|r, on| crate::ai::write(&provider, r, on), read_screen);
        }
        BUSY.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A text field (caret at the end, optional selection of the last n
    /// characters), a clipboard, and what the screen shows.
    #[derive(Default)]
    struct Fake {
        screen: RefCell<String>,
        selected: RefCell<usize>,
        clip: RefCell<Option<String>>,
        age: Option<Duration>,
        gen: RefCell<u64>,
        pastes: RefCell<usize>,
        seen: Option<Screen>,
        statuses: RefCell<Vec<Status>>,
    }

    impl Fake {
        fn new(screen: &str, clip: Option<&str>) -> Fake {
            Fake { screen: RefCell::new(screen.into()), clip: RefCell::new(clip.map(String::from)), age: Some(Duration::from_secs(5)), ..Default::default() }
        }
        fn last(&self) -> Status {
            self.statuses.borrow().last().unwrap().clone()
        }
        fn cut(&self, n: usize) {
            let s = self.screen.borrow().clone();
            let k = s.chars().count().saturating_sub(n);
            *self.screen.borrow_mut() = s.chars().take(k).collect();
        }
        fn text(&self) -> String {
            self.screen.borrow().clone()
        }
    }

    impl Ops for Fake {
        fn clipboard_text(&self) -> Option<String> {
            self.clip.borrow().clone()
        }
        fn clipboard_age(&self) -> Option<Duration> {
            self.age
        }
        fn set_clipboard(&self, text: &str) {
            *self.clip.borrow_mut() = Some(text.into());
        }
        fn screen(&self) -> Option<Screen> {
            self.seen.clone()
        }
        fn backspace(&self, n: usize) {
            self.cut(n);
        }
        fn select_left(&self, n: usize) {
            *self.selected.borrow_mut() = n.min(self.screen.borrow().chars().count());
        }
        fn deselect(&self) {
            *self.selected.borrow_mut() = 0;
        }
        fn copy(&self) {
            let n = *self.selected.borrow();
            let s = self.screen.borrow().clone();
            *self.clip.borrow_mut() = Some(s.chars().skip(s.chars().count() - n).collect());
        }
        fn paste(&self) {
            let n = std::mem::take(&mut *self.selected.borrow_mut());
            self.cut(n);
            let c = self.clip.borrow().clone().unwrap_or_default();
            self.screen.borrow_mut().push_str(&c);
            *self.pastes.borrow_mut() += 1;
        }
        fn generation(&self) -> u64 {
            *self.gen.borrow()
        }
        fn wait(&self, _: u64) {}
        fn status(&self, s: Status) {
            self.statuses.borrow_mut().push(s);
        }
    }

    const EMAIL: &str = "Hi Hasnat, can you make Friday at 3pm? Thanks, Sam";
    const REPLY: &str = "Hi Sam,\n\nFriday at 3pm works for me. Many thanks for moving it, it really helps.\n\nBest,\nHasnat";

    /// A model that streams REPLY in small pieces and records the request.
    fn writer(seen: &RefCell<Option<Request>>) -> impl Fn(&Request, &mut dyn FnMut(&str)) -> Result<String, String> + '_ {
        move |r, on| {
            *seen.borrow_mut() = Some(r.clone());
            for piece in REPLY.as_bytes().chunks(5) {
                on(std::str::from_utf8(piece).unwrap());
            }
            Ok(REPLY.to_string())
        }
    }

    fn req(instruction: &str, grab: Grab) -> ComposeReq {
        ComposeReq { instruction: instruction.into(), grab }
    }

    #[test]
    fn typed_instruction_is_replaced_piece_by_piece_and_clipboard_restored() {
        let instr = "say friday works, many thanks";
        let f = Fake::new(instr, Some(EMAIL));
        let seen = RefCell::new(None);
        run(&req(instr, Grab::Typed(instr.len())), 0, &f, &writer(&seen), true);
        assert_eq!(f.text(), REPLY);
        assert!(*f.pastes.borrow() > 1, "should arrive in pieces as it streams");
        assert_eq!(f.clip.borrow().as_deref(), Some(EMAIL));
        let r = seen.borrow().clone().unwrap();
        assert_eq!((r.instruction.as_str(), r.copied.as_deref()), (instr, Some(EMAIL)));
        assert_eq!(f.last(), Status::Done);
    }

    #[test]
    fn earlier_instruction_is_checked_then_replaced() {
        let f = Fake::new("Dear team,\nsay yes", Some(EMAIL));
        let seen = RefCell::new(None);
        run(&req("say yes", Grab::Earlier("say yes".into())), 0, &f, &writer(&seen), true);
        assert_eq!(f.text(), format!("Dear team,\n{REPLY}"));
        assert_eq!(f.clip.borrow().as_deref(), Some(EMAIL));
    }

    #[test]
    fn earlier_instruction_not_at_caret_changes_nothing() {
        let f = Fake::new("say yes\nsomething else", Some(EMAIL));
        let seen = RefCell::new(None);
        run(&req("say yes", Grab::Earlier("say yes".into())), 0, &f, &writer(&seen), true);
        assert_eq!(f.text(), "say yes\nsomething else");
        assert_eq!(f.clip.borrow().as_deref(), Some(REPLY));
        assert!(matches!(f.last(), Status::OnClipboard(_)));
    }

    #[test]
    fn typing_while_it_writes_stops_the_pasting() {
        // Keys arrived (generation 7) after the hotkey (generation 5): nothing is pasted.
        let f = Fake::new("say yes", Some(EMAIL));
        *f.gen.borrow_mut() = 7;
        let seen = RefCell::new(None);
        run(&req("say yes", Grab::Typed(7)), 5, &f, &writer(&seen), true);
        assert_eq!(f.text(), "say yes");
        assert_eq!(f.clip.borrow().as_deref(), Some(REPLY));
        // Typing that starts partway: what's pasted stays, the rest goes to the clipboard.
        let f = Fake::new("say yes", Some(EMAIL));
        let g = &f.gen;
        let w = |_: &Request, on: &mut dyn FnMut(&str)| {
            on("Hi Sam, Friday works for me and I really look forward to seeing you. ");
            *g.borrow_mut() = 1;
            on("See you then. Best wishes, Hasnat");
            Ok(String::new())
        };
        run(&req("say yes", Grab::Typed(7)), 0, &f, &w, true);
        assert_eq!(f.text(), "Hi Sam, Friday works for me and I really look forward to seeing you. ");
        assert_eq!(f.clip.borrow().as_deref(), Some("See you then. Best wishes, Hasnat"));
        assert!(matches!(f.last(), Status::OnClipboard(_)));
    }

    #[test]
    fn model_error_before_any_text_changes_nothing() {
        let f = Fake::new("say yes", Some(EMAIL));
        run(&req("say yes", Grab::Typed(7)), 0, &f, &|_, _| Err("no key".into()), true);
        assert_eq!(f.text(), "say yes");
        assert_eq!(f.clip.borrow().as_deref(), Some(EMAIL));
        assert_eq!(f.last(), Status::Failed("no key".into()));
    }

    #[test]
    fn screen_context_replaces_copying() {
        // Nothing copied, nothing typed: the conversation on screen is enough.
        let mut f = Fake::new("", None);
        f.seen = Some(Screen { app: "slack".into(), title: "#design".into(), window: "Priya: ship it Monday?".into(), ..Default::default() });
        let seen = RefCell::new(None);
        run(&req("", Grab::Nothing), 0, &f, &writer(&seen), true);
        assert_eq!(f.text(), REPLY);
        assert_eq!(seen.borrow().as_ref().unwrap().screen.as_ref().unwrap().window, "Priya: ship it Monday?");
        // Reading the screen switched off: nothing to go on.
        let mut f = Fake::new("", None);
        f.seen = Some(Screen { window: "Priya: ship it Monday?".into(), ..Default::default() });
        run(&req("", Grab::Nothing), 0, &f, &writer(&seen), false);
        assert!(matches!(f.last(), Status::Failed(_)));
    }

    #[test]
    fn the_instruction_is_not_sent_twice() {
        let mut f = Fake::new("Thanks! agree, short", None);
        f.seen = Some(Screen { app: "slack".into(), before: "Thanks! agree, short".into(), window: "Priya: ok?\nThanks! agree, short".into(), ..Default::default() });
        let seen = RefCell::new(None);
        run(&req("agree, short", Grab::Typed(12)), 0, &f, &writer(&seen), true);
        let s = seen.borrow().clone().unwrap().screen.unwrap();
        assert_eq!(s.before, "Thanks! ");
        assert!(!s.window.contains("agree, short"));
    }

    #[test]
    fn private_places_send_no_screen_text() {
        let mut f = Fake::new("", Some(EMAIL));
        f.seen = Some(Screen { app: "chrome.exe".into(), title: "Accounts - My Bank".into(), window: "Balance £1,234".into(), before: "x".into(), ..Default::default() });
        let seen = RefCell::new(None);
        run(&req("say thanks", Grab::Typed(10)), 0, &f, &writer(&seen), true);
        let s = seen.borrow().clone().unwrap().screen.unwrap();
        assert!(!s.has_text());
        assert_eq!(s.title, "Accounts - My Bank");
    }

    #[test]
    fn risky_or_stale_clipboard_is_not_context() {
        let f = Fake::new("", Some("hunter2-S3cret!"));
        assert_eq!(context(&f, "say yes"), None);
        let f = Fake::new("", Some("https://example.com/reset?token=abc"));
        assert_eq!(context(&f, "say yes"), None);
        let mut f = Fake::new("", Some(EMAIL));
        f.age = Some(Duration::from_secs(3600));
        assert_eq!(context(&f, "say yes"), None);
        let f = Fake::new("", Some("say yes"));
        assert_eq!(context(&f, "say yes"), None);
        let f = Fake::new("", Some(EMAIL));
        assert_eq!(context(&f, "say yes").as_deref(), Some(EMAIL));
    }

    #[test]
    fn pieces_break_at_sentences() {
        assert_eq!(cut_point("short", 24), None);
        let s = "Friday at 3pm works for me. Many tha";
        assert_eq!(&s[..cut_point(s, 24).unwrap()], "Friday at 3pm works for me. ");
    }
}
