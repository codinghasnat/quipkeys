//! Platform-independent heart of the background app: it mirrors what the
//! user has typed into the focused field (as far as it can be sure), feeds
//! finished words to the QuipKeys stream, and turns the stream's edits into
//! keyboard actions: "delete N characters, then type this".
//!
//! The platform layer (Windows hook, macOS event tap) converts OS key events
//! into [`Key`]s, asks [`Core::on_key`] what to do, and performs the answer.

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
}

impl Core {
    pub fn new() -> Core {
        let engine: &'static Engine = Box::leak(Box::new(Engine::new(Config::default())));
        Core {
            engine,
            stream: Stream::new(engine),
            shown: Vec::new(),
            anchor: 0,
            cur: String::new(),
            cur_times: Vec::new(),
            t0: None,
            last: None,
            enabled: true,
        }
    }

    fn reset(&mut self) {
        self.stream = Stream::new(self.engine);
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
    fn simulate(core: &mut Core, keys: &[Key]) -> String {
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

    fn keys(s: &str) -> Vec<Key> {
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
