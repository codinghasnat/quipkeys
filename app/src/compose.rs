//! The "write it for me" hotkey, start to finish.
//!
//! 1. Type what you want ("reply saying Friday works, many thanks") and copy
//!    whatever it should know (the email). Either order.
//! 2. Press the hotkey. QuipKeys reads the clipboard, asks the chosen model
//!    (ai.rs), removes your instruction and pastes the text in its place,
//!    then puts your clipboard back.
//!
//! Safety rules:
//! * The instruction is deleted only when QuipKeys knows exactly where it is:
//!   typed just now (it counts the characters), or typed before a click, in
//!   which case it is selected and *checked* before anything is replaced.
//! * If you carry on typing or click while the model is writing, nothing is
//!   pasted; the text is left on the clipboard instead.
//! * Copied text is used as context only if it was copied in the last
//!   [`CONTEXT_FRESH`], and never if it looks like a password or key.
//!
//! The platform layers supply [`Ops`]; the sequence itself is here so it can
//! be tested without a keyboard.

use std::time::Duration;

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
    /// Nothing was changed.
    Failed(String),
    /// The text couldn't go in safely; it's on the clipboard.
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

/// Asks the model: (instruction, context) -> the text, or why not.
pub type Writer<'a> = &'a dyn Fn(&str, Option<&str>) -> Result<String, String>;

/// Run one compose request. `write` asks the model (ai::write in the app).
pub fn run(req: &ComposeReq, gen0: u64, ops: &dyn Ops, write: Writer) {
    let ctx = context(ops, &req.instruction);
    if req.instruction.is_empty() && ctx.is_none() {
        ops.status(Status::Failed(
            "Type what you'd like written, and copy anything it should know (like the email), then press the hotkey.".into(),
        ));
        return;
    }
    ops.status(Status::Writing);
    let text = match write(&req.instruction, ctx.as_deref()) {
        Ok(t) => t,
        Err(why) => return ops.status(Status::Failed(why)),
    };
    if ops.generation() != gen0 {
        ops.set_clipboard(&text);
        return ops.status(Status::OnClipboard("You carried on typing, so the text is on your clipboard instead.".into()));
    }
    let saved = ops.clipboard_text();
    match &req.grab {
        Grab::Typed(n) => ops.backspace(*n),
        Grab::Earlier(instruction) => {
            ops.select_left(instruction.chars().count());
            ops.wait(40);
            ops.copy();
            ops.wait(150);
            if ops.clipboard_text().as_deref() != Some(instruction.as_str()) {
                ops.deselect();
                ops.set_clipboard(&text);
                return ops.status(Status::OnClipboard(
                    "Your instruction wasn't right before the cursor, so the text is on your clipboard: paste it where you want it.".into(),
                ));
            }
        }
        Grab::Nothing => {}
    }
    ops.set_clipboard(&text);
    ops.wait(60);
    ops.paste();
    // Give the app time to read the clipboard before it's put back.
    ops.wait(400);
    if let Some(s) = saved {
        ops.set_clipboard(&s);
    }
    ops.status(Status::Done);
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
            run(&req, gen0, ops.as_ref(), &|i, c| crate::ai::write(&provider, i, c));
        }
        BUSY.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A text field (caret at the end, optional selection of the last n
    /// characters) and a clipboard.
    #[derive(Default)]
    struct Fake {
        screen: RefCell<String>,
        selected: RefCell<usize>,
        clip: RefCell<Option<String>>,
        age: Option<Duration>,
        gen: RefCell<u64>,
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
    const REPLY: &str = "Hi Sam,\n\nFriday at 3pm works. Many thanks!\n\nHasnat";

    fn writer(seen: &RefCell<Option<(String, Option<String>)>>) -> impl Fn(&str, Option<&str>) -> Result<String, String> + '_ {
        move |i, c| {
            *seen.borrow_mut() = Some((i.to_string(), c.map(String::from)));
            Ok(REPLY.to_string())
        }
    }

    #[test]
    fn typed_instruction_is_replaced_and_clipboard_restored() {
        let instr = "say friday works, many thanks";
        let f = Fake::new(instr, Some(EMAIL));
        let seen = RefCell::new(None);
        let req = ComposeReq { instruction: instr.into(), grab: Grab::Typed(instr.len()) };
        run(&req, 0, &f, &writer(&seen));
        assert_eq!(*f.screen.borrow(), REPLY);
        assert_eq!(f.clip.borrow().as_deref(), Some(EMAIL));
        assert_eq!(seen.borrow().clone().unwrap(), (instr.to_string(), Some(EMAIL.to_string())));
        assert_eq!(f.last(), Status::Done);
    }

    #[test]
    fn earlier_instruction_is_checked_then_replaced() {
        let f = Fake::new("Dear team,\nsay yes", Some(EMAIL));
        let seen = RefCell::new(None);
        let req = ComposeReq { instruction: "say yes".into(), grab: Grab::Earlier("say yes".into()) };
        run(&req, 0, &f, &writer(&seen));
        assert_eq!(*f.screen.borrow(), format!("Dear team,\n{REPLY}"));
        assert_eq!(f.clip.borrow().as_deref(), Some(EMAIL));
    }

    #[test]
    fn earlier_instruction_not_at_caret_changes_nothing() {
        // The user clicked back somewhere else: the selection isn't the instruction.
        let f = Fake::new("say yes\nsomething else", Some(EMAIL));
        let seen = RefCell::new(None);
        let req = ComposeReq { instruction: "say yes".into(), grab: Grab::Earlier("say yes".into()) };
        run(&req, 0, &f, &writer(&seen));
        assert_eq!(*f.screen.borrow(), "say yes\nsomething else");
        assert_eq!(f.clip.borrow().as_deref(), Some(REPLY));
        assert!(matches!(f.last(), Status::OnClipboard(_)));
    }

    #[test]
    fn typing_while_it_writes_leaves_the_text_on_the_clipboard() {
        let f = Fake::new("say yes", Some(EMAIL));
        *f.gen.borrow_mut() = 7; // keys arrived since the hotkey (generation 5)
        let seen = RefCell::new(None);
        run(&ComposeReq { instruction: "say yes".into(), grab: Grab::Typed(7) }, 5, &f, &writer(&seen));
        assert_eq!(*f.screen.borrow(), "say yes");
        assert_eq!(f.clip.borrow().as_deref(), Some(REPLY));
    }

    #[test]
    fn model_error_changes_nothing() {
        let f = Fake::new("say yes", Some(EMAIL));
        run(&ComposeReq { instruction: "say yes".into(), grab: Grab::Typed(7) }, 0, &f, &|_, _| Err("no key".into()));
        assert_eq!(*f.screen.borrow(), "say yes");
        assert_eq!(f.clip.borrow().as_deref(), Some(EMAIL));
        assert_eq!(f.last(), Status::Failed("no key".into()));
    }

    #[test]
    fn copied_email_alone_gets_a_reply() {
        let f = Fake::new("", Some(EMAIL));
        let seen = RefCell::new(None);
        run(&ComposeReq { instruction: String::new(), grab: Grab::Nothing }, 0, &f, &writer(&seen));
        assert_eq!(*f.screen.borrow(), REPLY);
        // Nothing typed and nothing copied: nothing to do.
        let f = Fake::new("", None);
        run(&ComposeReq { instruction: String::new(), grab: Grab::Nothing }, 0, &f, &writer(&seen));
        assert!(matches!(f.last(), Status::Failed(_)));
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
}
