//! Streaming correction: words arrive one at a time (at space / punctuation),
//! each word is decided immediately with left context, and the previous word
//! gets exactly one more chance once its right-hand neighbour is known.
//!
//! At every space we also reconsider *where the previous space went*: the last
//! two typed chunks are re-read as one word (premature space: "meetin g"), or
//! with the space moved by a key or two ("hellon ice", "mee tyou").
//!
//! Hysteresis rules:
//!  * the engine changes a word's spelling at most once; the only exception is
//!    a boundary fix, which may override an earlier spelling fix of the
//!    previous word, and happens at most once per word;
//!  * a word the user undoes is protected for the rest of the session, and the
//!    typed spelling is remembered so it is never "corrected" again.

use std::collections::HashSet;

use std::collections::HashMap;
use std::sync::Arc;

use crate::decoder::{Ctx, DocCache, Engine};

mod lattice;
use crate::lexicon::WordId;

/// How a raw token splits into punctuation + correctable core.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub lead: String,
    pub core: String,
    pub trail: String,
    /// Byte offset of `core` within the raw token.
    pub core_start: usize,
    /// Never touch (code, names, numbers, URLs, acronyms...).
    pub protected: bool,
    /// Capitalised mid-sentence: probably a name, so only fix when very sure.
    pub name_like: bool,
    pub ends_sentence: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Case {
    Lower,
    Capital,
}

/// Abbreviations whose trailing '.' does not end a sentence.
const ABBREV: &[&str] = &[
    "mr", "mrs", "ms", "dr", "st", "jr", "sr", "vs", "etc", "prof", "gen", "col", "lt", "sgt", "capt", "gov", "rev",
    "hon", "inc", "ltd", "co", "corp", "no", "vol", "fig", "approx", "dept", "est", "ave", "mt", "ft", "jan", "feb",
    "mar", "apr", "jun", "jul", "aug", "sep", "sept", "oct", "nov", "dec",
];

pub fn parse_token(raw: &str, sentence_start: bool) -> Token {
    let b = raw.as_bytes();
    let is_letter = |c: u8| c.is_ascii_alphabetic();
    let s = b.iter().position(|&c| is_letter(c)).unwrap_or(b.len());
    let mut e = b.iter().rposition(|&c| is_letter(c)).map(|i| i + 1).unwrap_or(s);
    // Possessive / contraction 's is handled as a suffix: "driver's" -> driver + 's.
    if e >= s + 3 && raw[s..e].to_ascii_lowercase().ends_with("'s") {
        e -= 2;
    }
    let (lead, core, trail) = (&raw[..s], &raw[s..e], &raw[e..]);
    let lower = core.to_ascii_lowercase();
    let abbrev = trail.starts_with('.') && ABBREV.contains(&lower.as_str());
    let ends_sentence = trail.contains(['.', '!', '?']) && !abbrev;
    let mut protected = core.is_empty()
        || abbrev
        || !raw.is_ascii()
        || raw.bytes().any(|c| c.is_ascii_digit() || b"_/\\@#$%^&*=+<>|~`{}[]".contains(&c))
        || core.bytes().any(|c| !(is_letter(c) || c == b'\'' || c == b'-'))
        || core.contains("--")
        || lead.bytes().any(|c| c == b'.' || c == b'-' || c == b':')
        // Dialect elisions: "countin'", "'em".
        || (trail.starts_with('\'') && !trail.starts_with("'s"))
        || lead.ends_with('\'')
        || trail.contains("..") && !trail.ends_with("...")
        || trail.contains(['.', ':']) && trail.len() > 1 && trail.bytes().skip(1).any(is_letter);
    let mut name_like = false;
    match case_of(core) {
        None if !shift_slip(core) => protected = true, // ALL CAPS / camelCase
        Some(Case::Capital) if !sentence_start && !(lower == "i" || lower.starts_with("i'")) => name_like = true,
        _ => {}
    }
    Token {
        lead: lead.to_string(),
        core: core.to_string(),
        trail: trail.to_string(),
        core_start: s,
        protected,
        name_like,
        ends_sentence,
    }
}

/// Shift-key timing slips: "THe" (Shift held one key too long), "hTe" (Shift
/// landed on the wrong key). Not camelCase like "fooBar", which splits into
/// two real-looking parts of 2+ letters.
fn shift_slip(core: &str) -> bool {
    let b = core.as_bytes();
    if b.len() < 2 || !b.iter().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let ups: Vec<usize> = (0..b.len()).filter(|&i| b[i].is_ascii_uppercase()).collect();
    match ups.as_slice() {
        [0, 1] => b.len() >= 3 && b[2..].iter().all(|c| c.is_ascii_lowercase()),
        [1] => true,
        _ => false,
    }
}

fn case_of(core: &str) -> Option<Case> {
    if shift_slip(core) {
        // Treat as a capitalised word at sentence start, lowercase otherwise
        // (decided by the caller from context); "THe" -> "The".
        return Some(Case::Capital);
    }
    let mut chars = core.chars().filter(|c| c.is_ascii_alphabetic());
    let first = chars.next()?;
    let rest_lower = chars.all(|c| c.is_ascii_lowercase());
    match (first.is_ascii_uppercase(), rest_lower) {
        (false, true) => Some(Case::Lower),
        (true, true) => Some(Case::Capital),
        _ if core.len() == 1 => Some(Case::Capital),
        _ => None,
    }
}

/// Apply the typed word's capitalisation to a lowercase replacement, and
/// capitalise words that are normally written that way (london -> London).
fn recase_lx(lx: &crate::lexicon::Lexicon, word: &str, capital: bool) -> String {
    let mut out = String::with_capacity(word.len());
    for (i, w) in word.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let proper = w.split('-').next().is_some_and(|p| lx.cap_share(p) >= CAP_SHARE);
        let make_cap = (capital && i == 0) || w == "i" || w.starts_with("i'") || proper;
        if make_cap {
            let mut cs = w.chars();
            if let Some(f) = cs.next() {
                out.push(f.to_ascii_uppercase());
                out.extend(cs);
            }
        } else {
            out.push_str(w);
        }
    }
    out
}

fn swapcase(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_uppercase() { c.to_ascii_lowercase() } else { c.to_ascii_uppercase() }).collect()
}

/// A word written capitalised at least this often mid-sentence is a proper noun.
const CAP_SHARE: f32 = 0.97;
/// ...and this often before we capitalise a correctly spelled lowercase word.
pub(crate) const CAP_FIX_SHARE: f32 = 0.995;

#[derive(Clone, Debug)]
pub struct Word {
    pub typed: String,
    /// Current text for this typed chunk. Empty if it was merged into the
    /// previous word ("meetin g" -> "meeting", "").
    pub output: String,
    /// Dictionary id of the (last) output word, used as context.
    out_id: Option<WordId>,
    tok: Token,
    times: Option<Vec<f32>>,
    pub changed: bool,
    pub resegmented: bool,
    pub protected: bool,
    pub sentence_start: bool,
}

impl Word {
    fn capital(&self) -> bool {
        if shift_slip(&self.tok.core) {
            // A capital was typed, so Shift was meant: "uSre" -> "Sure".
            return true;
        }
        case_of(&self.tok.core) == Some(Case::Capital)
    }
    fn core_times(&self) -> Option<&[f32]> {
        let t = self.times.as_ref()?;
        t.get(self.tok.core_start..self.tok.core_start + self.tok.core.len())
    }
}

/// An edit to already-committed text: chunk `index` changes from `old` to `new`.
#[derive(Clone, Debug, PartialEq)]
pub struct Edit {
    pub index: usize,
    pub old: String,
    pub new: String,
    pub confidence: f32,
    pub revision: bool,
}

/// One change the decoder considered (for training the learned gate).
#[derive(Clone, Debug)]
pub struct GateExample {
    pub chunk: usize,
    pub span: usize,
    pub outs: Vec<String>,
    pub feats: Vec<f32>,
    pub applied: bool,
}

pub struct Stream<'e> {
    pub engine: &'e Engine,
    /// When set, every considered change is recorded here.
    pub log: Option<Vec<GateExample>>,
    /// Neural re-ranker cache.
    nn_state: crate::nnlm::NnState,
    pub words: Vec<Word>,
    pub enabled: bool,
    ignore: HashSet<String>,
    /// Document memory: word counts so far and words learned this session.
    cache: Arc<DocCache>,
    /// Dictionary ids currently counted in the cache for each chunk.
    counted: Vec<Vec<crate::lexicon::WordId>>,
    /// Unknown words left exactly as typed, by count.
    typed_oov: HashMap<String, u32>,
    learned_upto: usize,
    /// Recent plain words: was each typed as a non-word? (sloppiness estimate)
    recent_oov: std::collections::VecDeque<bool>,
    pub sloppy: bool,
    /// Caps Lock appears to be on by accident (inverted case: "w NEED tHIS").
    pub caps_lock: bool,
    /// Recent gaps between keys within words (ms), for the typist's rhythm.
    gaps: std::collections::VecDeque<f32>,
    caps_run: usize,
}

impl<'e> Stream<'e> {
    pub fn new(engine: &'e Engine) -> Self {
        Stream {
            engine,
            log: None,
            nn_state: Default::default(),
            words: Vec::new(),
            enabled: true,
            ignore: HashSet::new(),
            cache: Arc::new(DocCache::default()),
            counted: Vec::new(),
            typed_oov: HashMap::new(),
            learned_upto: 0,
            recent_oov: Default::default(),
            sloppy: false,
            gaps: Default::default(),
            caps_lock: false,
            caps_run: 0,
        }
    }

    /// Index of the last non-empty chunk before `i`.
    fn prev_index(&self, i: usize) -> Option<usize> {
        (0..i).rev().find(|&k| !self.words[k].output.is_empty())
    }

    /// Context three words back from chunk `i`.
    fn ctx_prev3(&self, i: usize) -> Option<WordId> {
        self.ctx_prev2(i)?;
        let p = self.prev_index(i)?;
        // If the previous chunk is a split ("the cat"), two of the context
        // words come from it; otherwise step back one chunk.
        let out = &self.words[p].output;
        let parts: Vec<&str> = out.split(' ').collect();
        if parts.len() >= 3 {
            return self.id_of(parts[parts.len() - 3]);
        }
        if parts.len() == 2 {
            return self.ctx_prev(p);
        }
        self.ctx_prev2(p)
    }

    /// Context two words back from chunk `i`.
    fn ctx_prev2(&self, i: usize) -> Option<WordId> {
        let p = self.prev_index(i)?;
        if self.ctx_prev(i).is_none() {
            return None;
        }
        // A split chunk ("the cat") has its own internal previous word.
        let out = &self.words[p].output;
        if let Some((head, _)) = out.rsplit_once(' ') {
            return self.id_of(head);
        }
        self.ctx_prev(p)
    }

    fn ctx_prev(&self, i: usize) -> Option<WordId> {
        let p = &self.words[self.prev_index(i)?];
        if p.tok.ends_sentence {
            None
        } else {
            p.out_id
        }
    }

    fn id_of(&self, output: &str) -> Option<WordId> {
        let last = output.rsplit(' ').next().unwrap_or(output);
        let lt = parse_token(last, true);
        let lower = lt.core.to_ascii_lowercase();
        let full = format!("{lower}'s");
        if lt.trail.starts_with("'s") {
            if let Some(id) = self.engine.lx.id(&full) {
                return Some(id);
            }
        }
        self.engine.lx.id(&lower)
    }

    /// Commit a finished word (the raw token between spaces, including any
    /// punctuation). `times` are optional key-down timestamps (ms), one per byte.
    pub fn push(&mut self, raw: &str, times: Option<Vec<f32>>) -> Vec<Edit> {
        let mut pre = Vec::new();
        let raw_owned;
        let raw = match self.caps_lock_check(raw, &mut pre) {
            Some(fixed) => {
                raw_owned = fixed;
                raw_owned.as_str()
            }
            None => raw,
        };
        self.estimate_sloppiness(raw);
        // Typical gap between keys, from the last ~200 gaps (median).
        if let Some(t) = &times {
            for w in t.windows(2) {
                let g = w[1] - w[0];
                if g > 0.0 && g < 1500.0 {
                    self.gaps.push_back(g);
                }
            }
            while self.gaps.len() > 200 {
                self.gaps.pop_front();
            }
        }
        let med = if self.gaps.len() >= 20 {
            let mut v: Vec<f32> = self.gaps.iter().copied().collect();
            v.sort_by(|a, b| a.partial_cmp(b).unwrap());
            v[v.len() / 2]
        } else {
            0.0
        };
        crate::channel::RHYTHM.with(|r| r.set(med));
        let sloppy = self.sloppy;
        crate::decoder::SLOPPY.with(|s| s.set(sloppy));
        let mut edits = self.push_inner(raw, times);
        crate::decoder::SLOPPY.with(|s| s.set(false));
        self.update_memory();
        pre.append(&mut edits);
        pre
    }

    /// Accidental Caps Lock: three inverted-case words in a row ("w NEED TO")
    /// switch it on; from then on words are case-flipped before decoding, and
    /// the words that triggered it are flipped back too. Two normally-cased
    /// words switch it off. One genuine acronym (NASA) never triggers it.
    fn caps_lock_check(&mut self, raw: &str, edits: &mut Vec<Edit>) -> Option<String> {
        let letters: Vec<u8> = raw.bytes().filter(|c| c.is_ascii_alphabetic()).collect();
        if letters.len() < 2 {
            return self.caps_lock.then(|| swapcase(raw));
        }
        let rest_upper = letters[1..].iter().all(|c| c.is_ascii_uppercase());
        let inverted = rest_upper; // "NEED", "tHIS", "mY"
        let normal = letters[1..].iter().all(|c| c.is_ascii_lowercase());
        if self.caps_lock {
            if normal {
                self.caps_run += 1;
                if self.caps_run >= 2 {
                    self.caps_lock = false;
                    self.caps_run = 0;
                }
                return None;
            }
            self.caps_run = 0;
            // Under Caps Lock a mistimed Shift gives mixed words ("thANSK",
            // "cpULD"): flip anything that is mostly capitals.
            let ups = letters.iter().filter(|c| c.is_ascii_uppercase()).count();
            return (ups * 2 >= letters.len()).then(|| swapcase(raw));
        }
        if !inverted {
            self.caps_run = 0;
            return None;
        }
        self.caps_run += 1;
        if self.caps_run < 3 {
            return None;
        }
        self.caps_lock = true;
        self.caps_run = 0;
        // Flip back the earlier words of this run.
        let n = self.words.len();
        for k in n.saturating_sub(2)..n {
            let w = &self.words[k];
            if w.changed || w.typed.bytes().filter(|c| c.is_ascii_alphabetic()).skip(1).any(|c| c.is_ascii_lowercase()) {
                continue;
            }
            let fixed = swapcase(&w.typed);
            let tok = parse_token(&fixed, w.sentence_start);
            let old = std::mem::replace(&mut self.words[k].output, fixed.clone());
            let w = &mut self.words[k];
            w.protected = tok.protected;
            w.tok = tok;
            w.typed = fixed.clone();
            edits.push(Edit { index: k, old, new: fixed, confidence: 1.0, revision: true });
            let id = self.id_of(&self.words[k].output);
            self.words[k].out_id = id;
        }
        Some(swapcase(raw))
    }

    /// How sloppy is this typist? Share of recent plain words that were typed
    /// as non-words, with hysteresis so the error model doesn't flap.
    fn estimate_sloppiness(&mut self, raw: &str) {
        let cfg = &self.engine.cfg;
        if cfg.sloppy_on < 0.0 {
            self.sloppy = true;
            return;
        }
        if cfg.sloppy_on == 0.0 {
            return;
        }
        let t = parse_token(raw, false);
        let lower = t.core.to_ascii_lowercase();
        if t.protected || lower.len() < 2 || !lower.bytes().all(|c| c.is_ascii_lowercase()) {
            return;
        }
        let oov = self.engine.lx.id(&lower).is_none() && !self.ignore.contains(&lower);
        self.recent_oov.push_back(oov);
        if self.recent_oov.len() > cfg.sloppy_window {
            self.recent_oov.pop_front();
        }
        if self.recent_oov.len() < 8 {
            return;
        }
        let rate = self.recent_oov.iter().filter(|&&x| x).count() as f32 / self.recent_oov.len() as f32;
        if !self.sloppy && rate >= cfg.sloppy_on {
            self.sloppy = true;
        } else if self.sloppy && rate <= cfg.sloppy_off {
            self.sloppy = false;
        }
    }

    /// Keep the document cache in sync with the latest outputs, and learn
    /// unknown words the user has now typed the same way twice.
    fn update_memory(&mut self) {
        let n = self.words.len();
        self.counted.resize(n, Vec::new());
        let lx = &self.engine.lx;
        let cache = Arc::make_mut(&mut self.cache);
        // Only the last few chunks can still change (look-backs, boundary repair).
        for k in n.saturating_sub(4)..n {
            let now: Vec<_> = self.words[k]
                .output
                .split(' ')
                .filter_map(|p| lx.id(&parse_token(p, true).core.to_ascii_lowercase()))
                .collect();
            if now != self.counted[k] {
                for &id in &self.counted[k] {
                    cache.add(id, -1);
                }
                for &id in &now {
                    cache.add(id, 1);
                }
                self.counted[k] = now;
            }
        }
        if self.engine.cfg.learned_weight <= 0.0 {
            return;
        }
        // Chunks older than the second look-back are final.
        while self.learned_upto + 3 < n {
            let w = &self.words[self.learned_upto];
            self.learned_upto += 1;
            let lower = w.tok.core.to_ascii_lowercase();
            let plain = lower.len() >= 3 && lower.bytes().all(|c| c.is_ascii_lowercase());
            if w.output != w.typed || !plain || w.tok.protected && !w.tok.name_like || lx.id(&lower).is_some() {
                continue;
            }
            let c = self.typed_oov.entry(lower.clone()).or_insert(0);
            *c += 1;
            if *c == 2 {
                // Typed the same way twice and never corrected: it's a word.
                self.ignore.insert(lower.clone());
                cache.learned.push((lower, 2));
            } else if *c > 2 {
                if let Some(e) = cache.learned.iter_mut().find(|e| e.0 == lower) {
                    e.1 = *c;
                }
            }
        }
    }

    fn push_inner(&mut self, raw: &str, times: Option<Vec<f32>>) -> Vec<Edit> {
        let cache = self.cache.clone();
        let i = self.words.len();
        let sentence_start = self.prev_index(i).is_none_or(|p| self.words[p].tok.ends_sentence);
        let tok = parse_token(raw, sentence_start);
        let lower = tok.core.to_ascii_lowercase();
        let protected = tok.protected || self.ignore.contains(&lower);
        let mut w = Word {
            typed: raw.to_string(),
            output: raw.to_string(),
            out_id: None,
            tok,
            times,
            changed: false,
            resegmented: false,
            protected,
            sentence_start,
        };
        w.out_id = self.id_of(raw);
        self.words.push(w);
        let mut edits = Vec::new();
        if !self.enabled {
            return edits;
        }
        if let Some(e) = self.try_hand_shift(i) {
            edits.push(e);
            return edits;
        }
        if self.engine.cfg.lattice {
            // Special tokens (names, hyphenated, shift slips, "i", "dont") keep
            // their dedicated handling; plain words are decoded jointly.
            if !self.lattice_open(i) {
                let ctx = Ctx {
            prev3: self.ctx_prev3(i),
            prev2: self.ctx_prev2(i),
            prev: self.ctx_prev(i),
            cache: Some(&cache),
            ..Default::default()
        };
                if let Some(e) = self.try_word(i, ctx, None, false) {
                    edits.push(e);
                }
            }
            edits.extend(self.lattice_step(i));
            // Two words after a real word, let the neural LM judge it with what follows.
            if self.engine.cfg.nn_lookahead > 0.0 && i >= 2 {
                if let Some(e) = self.nn_lookahead(i - 2, i) {
                    edits.push(e);
                }
            }
            return edits;
        }
        let reseg = self.try_boundary(i);
        if !reseg.is_empty() {
            return reseg;
        }
        let ctx = Ctx {
            prev3: self.ctx_prev3(i),
            prev2: self.ctx_prev2(i),
            prev: self.ctx_prev(i),
            cache: Some(&cache),
            ..Default::default()
        };
        if let Some(e) = self.try_word(i, ctx, None, false) {
            edits.push(e);
        }
        if self.engine.cfg.right_context {
            if let Some(p) = self.prev_index(i) {
                let pw = &self.words[p];
                if !pw.changed && !pw.protected && !pw.tok.ends_sentence {
                    let next = self.words[i].out_id;
                    if next.is_some() {
                        let ctx = Ctx {
                            prev2: self.ctx_prev2(p),
                            prev: self.ctx_prev(p),
                            next,
                            cache: Some(&cache),
                            ..Default::default()
                        };
                        // Real words need tau_revise; a non-word just needs the usual bar.
                        let known = self.engine.lx.id(&pw.tok.core.to_ascii_lowercase()).is_some();
                        let tau = if known { Some(self.engine.cfg.tau_revise) } else { None };
                        if let Some(e) = self.try_word(p, ctx, tau, true) {
                            edits.push(e);
                        }
                    }
                }
                // Second look-back: two words of context on each side.
                if self.engine.cfg.lookback2 {
                    if let Some(pp) = self.prev_index(p) {
                        let w2 = &self.words[pp];
                        let (n1, n2) = (self.words[p].out_id, self.words[i].out_id);
                        if !w2.changed
                            && !w2.protected
                            && !w2.tok.ends_sentence
                            && !self.words[p].tok.ends_sentence
                            && n1.is_some()
                            && n2.is_some()
                        {
                            let ctx = Ctx {
                                prev2: self.ctx_prev2(pp),
                                prev: self.ctx_prev(pp),
                                next: n1,
                                next2: n2,
                                cache: Some(&cache),
                                ..Default::default()
                            };
                            let known = self.engine.lx.id(&w2.tok.core.to_ascii_lowercase()).is_some();
                            let tau = if known { Some(self.engine.cfg.tau_revise) } else { None };
                            if let Some(e) = self.try_word(pp, ctx, tau, true) {
                                edits.push(e);
                            }
                        }
                    }
                }
            }
        }
        edits
    }

    fn set_output(&mut self, i: usize, new: String) -> String {
        let id = if new.is_empty() { None } else { self.id_of(&new) };
        let w = &mut self.words[i];
        w.changed = true;
        w.out_id = id;
        std::mem::replace(&mut w.output, new)
    }

    fn try_word(&mut self, i: usize, ctx: Ctx, tau: Option<f32>, revision: bool) -> Option<Edit> {
        let w = &self.words[i];
        if w.protected || w.changed {
            return None;
        }
        let lower = w.tok.core.to_ascii_lowercase();
        if w.tok.core == "i" {
            // Lone lowercase "i" is always "I".
            let new = format!("{}I{}", w.tok.lead, w.tok.trail);
            let old = self.set_output(i, new.clone());
            return Some(Edit { index: i, old, new, confidence: 1.0, revision });
        }
        let tau = if w.tok.name_like { Some(tau.unwrap_or(0.0).max(self.engine.cfg.tau_capital)) } else { tau };
        let ctx = Ctx { name_like: w.tok.name_like, ..ctx };
        if lower.contains('-') {
            // Hyphenated: fix each non-word part on its own ("out-fo-doors").
            if revision {
                return None;
            }
            let mut changed = false;
            let parts: Vec<String> = lower
                .split('-')
                .map(|part| {
                    if part.len() >= 2 && self.engine.lx.id(part).is_none() {
                        if let Some(s) = self.engine.decide(part, None, &Ctx::default(), tau) {
                            if !s.text.contains(' ') {
                                changed = true;
                                return s.text;
                            }
                        }
                    }
                    part.to_string()
                })
                .collect();
            if !changed {
                return None;
            }
            let new = format!("{}{}{}", w.tok.lead, recase_lx(&self.engine.lx, &parts.join("-"), w.capital()), w.tok.trail);
            let old = self.set_output(i, new.clone());
            return Some(Edit { index: i, old, new, confidence: 1.0, revision });
        }
        let slip = shift_slip(&w.tok.core);
        // "uSre": the Shift went with the second key, so the first two keys
        // rolled over. If swapping them gives a word, that is the reading.
        let rolled = (slip && w.tok.core.as_bytes()[1].is_ascii_uppercase())
            .then(|| {
                let mut b = lower.clone().into_bytes();
                b.swap(0, 1);
                String::from_utf8(b).ok()
            })
            .flatten()
            .filter(|r| self.engine.lx.id(r).is_some());
        let lx = &self.engine.lx;
        let fixed_case = (slip && lx.id(&lower).is_some()).then(|| lower.clone());
        // "london" typed in lowercase: just capitalise it.
        let cap_fix = (!slip && w.tok.core == lower && lx.cap_share(&lower) >= CAP_FIX_SHARE).then(|| lower.clone());
        // A capitalised word mid-sentence may be a name, so it normally needs
        // near-certainty. But if the fix is itself a known proper noun
        // ("Calicornia" -> California), the usual bar is enough.
        let proper_fix = if w.tok.name_like {
            let plain = Ctx { name_like: false, ..ctx };
            self.engine
                .decide(&lower, w.core_times(), &plain, None)
                .filter(|s| {
                    // A name learned in this session ("Hasnat" typed twice) is
                    // as good as a dictionary proper noun.
                    if ctx.cache.is_some_and(|c| c.learned.iter().any(|(w, _)| *w == s.text)) {
                        return true;
                    }
                    if s.text.contains(' ') || lx.cap_share(&s.text) < 0.9 {
                        return false;
                    }
                    // Only a single clean keyboard slip (adjacent key, roll-over swap,
                    // doubled/dropped repeat), never in the first letter: names
                    // differ from each other by spelling variants (Colman/Coleman).
                    let id = lx.id(&s.text);
                    self.engine
                        .candidates(&lower, w.core_times())
                        .iter()
                        .any(|c| Some(c.word) == id && c.channel < self.engine.cfg.proper_fix_max_channel)
                        && s.text.as_bytes().first() == lower.as_bytes().first()
                })
        } else {
            None
        };
        let s = match rolled {
            Some(r) => crate::decoder::Suggestion { text: r, confidence: 1.0 },
            None => match proper_fix.or_else(|| self.engine.decide(&lower, w.core_times(), &ctx, tau)) {
                Some(s) => s,
                None => crate::decoder::Suggestion { text: fixed_case.or(cap_fix)?, confidence: 1.0 },
            },
        };
        let new = format!("{}{}{}", w.tok.lead, recase_lx(&self.engine.lx, &s.text, w.capital()), w.tok.trail);
        if new == w.output {
            return None;
        }
        let old = self.set_output(i, new.clone());
        Some(Edit { index: i, old, new, confidence: s.confidence, revision })
    }

    /// Whole hand one key off: "jr;;p" -> "hello".
    fn try_hand_shift(&mut self, i: usize) -> Option<Edit> {
        let w = self.words[i].clone();
        let raw = w.typed.as_str();
        if self.ignore.contains(&raw.to_ascii_lowercase()) {
            return None;
        }
        let body = raw.trim_end_matches(['!', '?']);
        let capital = raw.as_bytes().first().is_some_and(|c| c.is_ascii_uppercase()) && w.sentence_start;
        if raw.bytes().skip(1).any(|c| c.is_ascii_uppercase()) || (!w.sentence_start && raw.as_bytes()[0].is_ascii_uppercase()) {
            return None;
        }
        if self.engine.lx.id(&w.tok.core.to_ascii_lowercase()).is_some() {
            return None;
        }
        // Try the whole token first ('.' and ',' are keys too), then without
        // trailing punctuation.
        for b in [body, body.trim_end_matches(['.', ','])] {
            if b.len() < 4 {
                continue;
            }
            if let Some(word) = self.engine.hand_shift(b) {
                let new = format!("{}{}", recase_lx(&self.engine.lx, &word, capital), &raw[b.len()..]);
                let old = self.set_output(i, new.clone());
                return Some(Edit { index: i, old, new, confidence: 1.0, revision: false });
            }
        }
        None
    }

    /// Reconsider the space between the previous chunk and chunk `i`.
    fn try_boundary(&mut self, i: usize) -> Vec<Edit> {
        let Some(p) = self.prev_index(i) else { return Vec::new() };
        if p + 1 != i || !self.engine.cfg.boundary {
            return Vec::new();
        }
        let (pw, cw) = (&self.words[p], &self.words[i]);
        let eligible = |w: &Word| {
            !w.protected && !w.resegmented && !w.tok.name_like && w.tok.core.bytes().all(|c| c.is_ascii_alphabetic())
        };
        if !eligible(pw) || !eligible(cw) || !pw.tok.trail.is_empty() || !cw.tok.lead.is_empty() {
            return Vec::new();
        }
        let a = pw.tok.core.to_ascii_lowercase();
        let b = cw.tok.core.to_ascii_lowercase();
        let lx = &self.engine.lx;
        // Only when something is off: clean text never has its spaces moved.
        if lx.id(&a).is_some() && lx.id(&b).is_some() {
            return Vec::new();
        }
        let e = self.engine;
        let prev = self.ctx_prev(p);
        let prev2 = self.ctx_prev2(p);
        let joined = format!("{a}{b}");
        let joined_times: Option<Vec<f32>> = match (pw.core_times(), cw.core_times()) {
            (Some(x), Some(y)) => Some(x.iter().chain(y).copied().collect()),
            _ => None,
        };
        let jt = joined_times.as_deref();
        let cache = self.cache.clone();
        let ctx = |prev2, prev| Ctx { prev2, prev, cache: Some(&cache), ..Default::default() };
        // Score of reading `x` as word(s); an unknown word can never be the
        // *result* of moving a space, so it costs infinity there.
        let known = |text: &str| text.split(' ').all(|w| lx.id(w).is_some());
        let pair = |k: usize, strict: bool| -> (String, String, f32) {
            let (x, y) = joined.split_at(k);
            let (tx, ty) = match jt {
                Some(t) => (Some(&t[..k]), Some(&t[k..])),
                None => (None, None),
            };
            let (bx, sx) = e.best(x, tx, &ctx(prev2, prev));
            let (by, sy) = e.best(y, ty, &ctx(prev, lx.id(bx.rsplit(' ').next().unwrap_or(&bx))));
            let single = |t: &str| !t.contains(' ');
            let ok = !strict || (known(&bx) && known(&by) && single(&bx) && single(&by));
            (bx, by, if ok { sx + sy } else { f32::INFINITY })
        };
        // As typed (each side may still be spell-corrected).
        let (base_a, base_b, base) = pair(a.len(), false);
        let mut alts: Vec<(String, String, f32)> = Vec::new();
        let (m, sm) = e.best(&joined, jt, &ctx(prev2, prev));
        let base_known = known(&base_a) && known(&base_b);
        // Gluing two real words together is how compounds get mangled
        // ("soy beans" -> "soybeans"): demand much stronger evidence.
        let merge_extra = if base_known { e.cfg.merge_margin } else { 0.0 };
        let sm = if known(&m) { sm + merge_extra } else { f32::INFINITY };
        alts.push((m, String::new(), sm + e.cfg.space_cost));
        for d in [-1i32, 1] {
            let k = a.len() as i32 + d;
            if k < 1 || k >= joined.len() as i32 {
                continue;
            }
            let (x, y, s) = pair(k as usize, true);
            // Moving a space through a correctly typed word costs extra.
            let touches_clean = (lx.id(&a).is_some() && x != base_a) || (lx.id(&b).is_some() && y != base_b);
            let extra = if touches_clean { e.cfg.shift_margin } else { 0.0 };
            alts.push((x, y, s + e.cfg.space_cost + extra));
        }
        let best_alt = alts.iter().min_by(|x, y| x.2.partial_cmp(&y.2).unwrap()).unwrap().clone();
        if !best_alt.2.is_finite() || best_alt.2 >= base {
            return Vec::new();
        }
        // Posterior of the best re-segmentation against "as typed" and the rest.
        let min = best_alt.2;
        let z: f32 = std::iter::once(base).chain(alts.iter().map(|x| x.2)).map(|s| (min - s).exp()).sum();
        let conf = 1.0 / z;
        if conf < e.cfg.tau_reseg || (best_alt.0 == base_a && best_alt.1 == base_b) {
            return Vec::new();
        }
        let (cap_p, lead_p) = (self.words[p].capital(), self.words[p].tok.lead.clone());
        let trail_c = self.words[i].tok.trail.clone();
        let (new_p, new_c) = if best_alt.1.is_empty() {
            (format!("{lead_p}{}{trail_c}", recase_lx(lx, &best_alt.0, cap_p)), String::new())
        } else {
            (format!("{lead_p}{}", recase_lx(lx, &best_alt.0, cap_p)), format!("{}{trail_c}", recase_lx(lx, &best_alt.1, false)))
        };
        let mut edits = Vec::new();
        for (k, new) in [(p, new_p), (i, new_c)] {
            self.words[k].resegmented = true;
            if self.words[k].output != new {
                let old = self.set_output(k, new.clone());
                edits.push(Edit { index: k, old, new, confidence: conf, revision: k == p });
            } else {
                self.words[k].changed = true;
            }
        }
        edits
    }

    /// The user undid the correction of chunk `index`: restore it, protect it,
    /// and never correct that spelling again this session.
    pub fn reject(&mut self, index: usize) -> Option<Edit> {
        let w = self.words.get_mut(index)?;
        if !w.changed {
            return None;
        }
        let old = std::mem::replace(&mut w.output, w.typed.clone());
        w.protected = true;
        let lower = w.tok.core.to_ascii_lowercase();
        self.ignore.insert(lower.clone());
        self.ignore.insert(w.typed.to_ascii_lowercase());
        // An undo is the strongest signal there is: learn the word.
        if self.engine.lx.id(&lower).is_none() && lower.bytes().all(|c| c.is_ascii_lowercase()) {
            Arc::make_mut(&mut self.cache).learned.push((lower, 2));
        }
        let new = w.typed.clone();
        self.words[index].out_id = self.id_of(&new);
        Some(Edit { index, old, new, confidence: 1.0, revision: false })
    }

    pub fn text(&self) -> String {
        self.words.iter().map(|w| w.output.as_str()).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ")
    }
}

impl Engine {
    /// Correct a whole line exactly as the streaming path would, word by word.
    pub fn correct_line(&self, line: &str) -> String {
        let mut s = Stream::new(self);
        for t in line.split_whitespace() {
            s.push(t, None);
        }
        s.text()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_protection() {
        for raw in [
            "fooBar", "snake_case", "src/main.rs", "v2", "NASA", "e.g.", "http://x.io", "--force", "a@b.com", "countin'",
            "'em", "Mr.",
        ] {
            assert!(parse_token(raw, false).protected, "{raw}");
        }
        assert!(parse_token("Smith", false).name_like);
        assert!(!parse_token("THe", true).protected);
        assert!(!parse_token("hTe", false).protected);
        assert!(parse_token("fooBar", false).protected);
        assert!(!parse_token("out-fo-doors,", false).protected);
        assert!(parse_token("--force", false).protected);
        assert!(!parse_token("Thsi", true).protected);
        assert!(!parse_token("stpre.", false).protected);
        assert!(!parse_token("(relaly)", false).protected);
        assert!(!parse_token("Mr.", true).ends_sentence);
        let t = parse_token("\"wamt,\"", false);
        assert_eq!((t.lead.as_str(), t.core.as_str(), t.trail.as_str()), ("\"", "wamt", ",\""));
        let t = parse_token("drivr's", false);
        assert_eq!((t.core.as_str(), t.trail.as_str()), ("drivr", "'s"));
    }
}
