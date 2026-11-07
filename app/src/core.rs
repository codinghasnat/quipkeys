//! Platform-independent heart of the background app: it mirrors what the
//! user has typed into the focused field (as far as it can be sure), feeds
//! finished words to the QuipKeys stream, and turns the stream's edits into
//! keyboard actions: "delete N characters, then type this".
//!
//! The platform layer (Windows hook, macOS event tap) converts OS key events
//! into [`Key`]s, asks [`Core::on_key`] what to do, and performs the answer.

use std::path::PathBuf;
use std::time::SystemTime;

use quipkeys::{Config, Engine, Stream};

/// A key event, already translated by the platform layer.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    /// A printable character (letter, digit, punctuation), with key-down time in ms.
    Char(char, f64),
    Space(f64),
    Backspace,
    /// Enter, Tab, Escape, arrows, Home/End, a mouse click, a shortcut with
    /// Ctrl/Cmd/Alt, or a focus change: what's on screen is no longer what we
    /// think, so forget the tracked text.
    Reset,
    /// The on/off hotkey.
    Toggle,
}

/// What the platform layer should do with the key it just reported.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Let the key through untouched.
    Pass,
    /// Swallow the key; then delete `backspaces` characters and type `text`.
    Replace { backspaces: usize, text: String },
    /// Swallow the key and do nothing else (the toggle hotkey).
    Swallow,
}

/// How many recent words may still be changed on screen. Older edits from the
/// stream (rare) are ignored rather than reaching far back into the text.
const EDITABLE_WORDS: usize = 6;

pub struct Core {
    engine: &'static Engine,
    stream: Stream<'static>,
    /// Text of each committed chunk as currently on screen, in stream order.
    shown: Vec<String>,
    /// Index in `shown` of the first chunk we still trust to be on screen.
    anchor: usize,
    /// The word being typed, and its key times.
    cur: String,
    cur_times: Vec<f32>,
    /// Time origin (ms) for key times: platforms report absolute times,
    /// which lose precision as f32; the stream gets times relative to this.
    t0: Option<f64>,
    /// The last replacement, for one-key undo: (chunk indices changed,
    /// text that was on screen before, text typed instead).
    last: Option<(Vec<usize>, String, String)>,
    pub enabled: bool,
    /// Personal word list: words the user taught QuipKeys (by undoing a fix),
    /// kept in a text file so they survive restarts. One word per line.
    words_path: Option<PathBuf>,
    words_stamp: Option<SystemTime>,
    personal: Vec<String>,
}

const WORDS_HEADER: &str = "# Your words: QuipKeys never changes these, and fixes typos of them.\n\
# Words are added here when you undo a correction (Backspace right after it).\n\
# Add or remove words freely, one per line.\n";

/// The letters of a typed chunk that make it a word ("Hasnat," -> "hasnat").
fn word_core(chunk: &str) -> String {
    chunk.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()
}

impl Core {
    /// A core without a word file (tests).
    #[cfg(test)]
    pub fn new() -> Core {
        Core::with_words(None)
    }

    /// A core that loads and saves the personal word list at `words_path`.
    pub fn with_words(words_path: Option<PathBuf>) -> Core {
        // Sounds-like matching is off unless the user switches on "Spell by
        // sound" (quipkeys::decoder::set_sound_mode), so ordinary typing is
        // corrected exactly as before.
        Core::build(Config::default(), words_path)
    }

    fn build(cfg: Config, words_path: Option<PathBuf>) -> Core {
        let engine: &'static Engine = Box::leak(Box::new(Engine::new(cfg)));
        let mut core = Core {
            engine,
            stream: Stream::new(engine),
            shown: Vec::new(),
            anchor: 0,
            cur: String::new(),
            cur_times: Vec::new(),
            t0: None,
            last: None,
            enabled: true,
            words_path,
            words_stamp: None,
            personal: Vec::new(),
        };
        core.reset();
        core
    }

    /// Re-read the word list if the file changed (e.g. the user edited it).
    fn reload_words(&mut self) {
        let Some(path) = &self.words_path else { return };
        let stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        if stamp.is_some() && stamp == self.words_stamp {
            return;
        }
        self.words_stamp = stamp;
        self.personal = std::fs::read_to_string(path)
            .map(|t| {
                t.lines().map(word_core).filter(|w| !w.is_empty() && !w.starts_with('#')).collect()
            })
            .unwrap_or_default();
    }

    /// Add a word to the personal list (and its file).
    fn remember(&mut self, word: &str) {
        let w = word_core(word);
        if w.len() < 2 || self.personal.contains(&w) || self.engine.lx.id(&w).is_some() {
            return;
        }
        self.personal.push(w.clone());
        self.stream.teach(&w);
        if let Some(path) = &self.words_path {
            use std::io::Write;
            let new_file = !path.exists();
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                if new_file {
                    let _ = f.write_all(WORDS_HEADER.as_bytes());
                }
                let _ = writeln!(f, "{w}");
            }
            self.words_stamp = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        }
    }

    fn reset(&mut self) {
        self.stream = Stream::new(self.engine);
        self.reload_words();
        for w in &self.personal {
            self.stream.teach(w);
        }
        self.shown.clear();
        self.anchor = 0;
        self.cur.clear();
        self.cur_times.clear();
        self.t0 = None;
        self.last = None;
    }

    pub fn on_key(&mut self, key: Key) -> Action {
        if key == Key::Toggle {
            self.enabled = !self.enabled;
            self.reset();
            return Action::Swallow;
        }
        if !self.enabled {
            return Action::Pass;
        }
        match key {
            Key::Reset => {
                self.reset();
                Action::Pass
            }
            Key::Char(c, t) => {
                self.last = None;
                self.cur.push(c);
                let t0 = *self.t0.get_or_insert(t);
                self.cur_times.push((t - t0) as f32);
                Action::Pass
            }
            Key::Backspace => {
                // Backspace straight after a correction undoes it.
                if let Some((idxs, before, after)) = self.last.take() {
                    for &i in &idxs {
                        self.stream.reject(i);
                    }
                    // Undoing a fix teaches the word for good.
                    let typed: Vec<String> =
                        idxs.iter().filter_map(|&i| self.stream.words.get(i).map(|w| w.typed.clone())).collect();
                    for t in typed {
                        self.remember(&t);
                    }
                    let first = *idxs.iter().min().unwrap();
                    for (k, w) in self.stream.words.iter().enumerate().skip(first) {
                        if k < self.shown.len() {
                            self.shown[k] = w.output.clone();
                        }
                    }
                    // `after` ended with the space we typed; restore `before` + space.
                    return Action::Replace { backspaces: after.chars().count(), text: format!("{before} ") };
                }
                if self.cur.pop().is_some() {
                    self.cur_times.pop();
                    // Typing times stay aligned with the remaining letters.
                    self.cur_times.truncate(self.cur.chars().count());
                } else {
                    // Deleting into earlier text: we can no longer be sure what's on screen.
                    self.reset();
                }
                Action::Pass
            }
            Key::Space(_) => self.commit(),
            Key::Toggle => unreachable!(),
        }
    }

    /// The current word is finished (space pressed): decode it and work out
    /// what, if anything, must change on screen.
    fn commit(&mut self) -> Action {
        self.last = None;
        if self.cur.is_empty() {
            // Double space or space after a reset: nothing to decode.
            return Action::Pass;
        }
        let word = std::mem::take(&mut self.cur);
        let times = std::mem::take(&mut self.cur_times);
        let times = (times.len() == word.len()).then_some(times);
        let before_all: Vec<String> = self.shown.clone();
        let edits = self.stream.push(&word, times);
        self.shown.push(word.clone());
        // Keep the editable window bounded.
        if self.shown.len() - self.anchor > EDITABLE_WORDS * 4 {
            self.anchor = self.shown.len() - EDITABLE_WORDS;
        }
        if edits.is_empty() {
            return Action::Pass;
        }
        // New on-screen text for every chunk, from the stream.
        let new: Vec<String> = self.stream.words.iter().map(|w| w.output.clone()).collect();
        let first = edits.iter().map(|e| e.index).min().unwrap();
        if first < self.anchor || self.shown.len() - first > EDITABLE_WORDS {
            // Too far back to touch safely: accept the stream's view, change nothing.
            self.shown = new;
            return Action::Pass;
        }
        let mut old_shown = before_all;
        old_shown.push(word);
        let before = join_nonempty(&old_shown[first..]);
        let after = join_nonempty(&new[first..]);
        self.shown = new;
        if before == after {
            return Action::Pass;
        }
        // The space was swallowed: delete what's on screen from `first`, type
        // the corrected text, then the space.
        let backspaces = before.chars().count();
        let text = format!("{after} ");
        let idxs: Vec<usize> = edits.iter().map(|e| e.index).collect();
        self.last = Some((idxs, before, text.clone()));
        Action::Replace { backspaces, text }
    }
}

/// Join chunks with single spaces, skipping chunks merged away ("").
fn join_nonempty(chunks: &[String]) -> String {
    chunks.iter().filter(|c| !c.is_empty()).cloned().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type `text` through the core and return what the screen would show.
    pub(super) fn simulate(core: &mut Core, keys: &[Key]) -> String {
        let mut screen = String::new();
        for k in keys {
            match core.on_key(k.clone()) {
                Action::Pass => match k {
                    Key::Char(c, _) => screen.push(*c),
                    Key::Space(_) => screen.push(' '),
                    Key::Backspace => {
                        screen.pop();
                    }
                    _ => {}
                },
                Action::Replace { backspaces, text } => {
                    for _ in 0..backspaces {
                        screen.pop();
                    }
                    screen.push_str(&text);
                }
                Action::Swallow => {}
            }
        }
        screen
    }

    pub(super) fn keys(s: &str) -> Vec<Key> {
        s.chars()
            .enumerate()
            .map(|(i, c)| match c {
                ' ' => Key::Space(1e9 + i as f64 * 120.0),
                '\u{8}' => Key::Backspace,
                c => Key::Char(c, 1e9 + i as f64 * 120.0),
            })
            .collect()
    }

    #[test]
    fn corrects_on_screen_as_you_type() {
        let mut c = Core::new();
        assert_eq!(simulate(&mut c, &keys("I wamt to gp to the stpre ")), "I want to go to the store ");
    }

    #[test]
    fn boundary_repair_reaches_back() {
        let mut c = Core::new();
        assert_eq!(simulate(&mut c, &keys("Hellon ice to mee tyou ")), "Hello nice to meet you ");
    }

    #[test]
    fn backspace_right_after_a_fix_undoes_it() {
        let mut c = Core::new();
        let mut k = keys("I wamt ");
        k.push(Key::Backspace);
        assert_eq!(simulate(&mut c, &k), "I wamt ");
        // ...and that spelling is now left alone.
        assert_eq!(simulate(&mut c, &keys("to wamt ")), "to wamt ");
    }

    #[test]
    fn toggle_and_reset() {
        let mut c = Core::new();
        let mut k = vec![Key::Toggle];
        k.extend(keys("I wamt "));
        assert_eq!(simulate(&mut c, &k), "I wamt ");
        let mut c = Core::new();
        let mut k = keys("I wa");
        k.push(Key::Reset);
        k.extend(keys("mt "));
        // After a reset (e.g. a click) the half-word before it is forgotten:
        // "mt" is judged alone, never rewritten as part of "wamt".
        assert!(!simulate(&mut c, &k).contains("want"));
    }
}

#[cfg(test)]
mod timing {
    use super::*;

    /// Run with: cargo test --release -- --ignored --nocapture space_latency
    #[test]
    #[ignore]
    fn space_latency() {
        let mut c = Core::new();
        let text = "teh quick brpwn fox jumsp ovr the lazy dpg and thrn it rna away intp the wodos beacuse it wsa scraed of teh hunetrs ";
        let mut worst = 0f64;
        let mut total = 0f64;
        let mut n = 0;
        for _ in 0..5 {
            for (i, ch) in text.chars().enumerate() {
                let t = i as f64 * 110.0;
                let k = if ch == ' ' { Key::Space(t) } else { Key::Char(ch, t) };
                let s = std::time::Instant::now();
                c.on_key(k.clone());
                if ch == ' ' {
                    let ms = s.elapsed().as_secs_f64() * 1000.0;
                    worst = worst.max(ms);
                    total += ms;
                    n += 1;
                }
            }
        }
        eprintln!("space: mean {:.2} ms, worst {:.2} ms over {n} words", total / n as f64, worst);
    }
}

#[cfg(test)]
mod pronoun {
    use super::tests::{keys, simulate};
    use super::*;

    /// "i"/"I" must always come out as "I", whatever surrounds it (a report
    /// of "I" turning into "il" was an injection problem, not the engine).
    #[test]
    fn i_is_always_i() {
        let mut c = Core::new();
        for w in ["the", "so", "when", "and", "what", "because", "maybe", "today"] {
            for after in ["think", "am", "was", "want", "like", "it", "know", "press"] {
                for i in ["i", "I"] {
                    c.on_key(Key::Reset);
                    let out = simulate(&mut c, &keys(&format!("{w} {i} {after} ok ")));
                    assert_eq!(out.split(' ').nth(1), Some("I"), "{w} {i} {after}: {out:?}");
                }
            }
        }
    }
}

#[cfg(test)]
mod personal {
    use super::tests::{keys, simulate};
    use super::*;

    #[test]
    fn undone_words_are_remembered_across_restarts() {
        let path = std::env::temp_dir().join(format!("qk-words-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut c = Core::with_words(Some(path.clone()));
        let mut k = keys("I wamt ");
        k.push(Key::Backspace);
        assert_eq!(simulate(&mut c, &k), "I wamt ");
        assert!(std::fs::read_to_string(&path).unwrap().lines().any(|l| l == "wamt"));
        // Same engine, new session: Enter, then the word again.
        c.on_key(Key::Reset);
        assert_eq!(simulate(&mut c, &keys("we wamt ")), "we wamt ");

        // A word added to the file by hand: kept as typed, and typos of it fixed.
        std::fs::write(&path, "# mine\nquipkeys\n").unwrap();
        // (make sure the new mtime is seen even on coarse file systems)
        c.words_stamp = None;
        c.on_key(Key::Reset);
        assert_eq!(simulate(&mut c, &keys("I use quipkeys ")), "I use quipkeys ");
        c.on_key(Key::Reset);
        assert_eq!(simulate(&mut c, &keys("I use quipkeyd ")), "I use quipkeys ");
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod sound {
    use super::tests::{keys, simulate};
    use super::*;

    #[test]
    fn spelling_by_sound_in_a_sentence() {
        // As with "Spell by sound" on (without touching the global switch,
        // which other tests running in parallel would see).
        let cfg = Config { phonetic: true, phon_base: Config::default().phon_base_sound, ..Config::default() };
        let mut c = Core::build(cfg, None);
        let out = simulate(&mut c, &keys("I took a pikcher of the new mashine "));
        assert!(out.contains("picture") && out.contains("machine"), "{out:?}");
    }
}
