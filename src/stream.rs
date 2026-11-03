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

use crate::decoder::{Ctx, Engine};
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
        || core.bytes().any(|c| !(is_letter(c) || c == b'\''))
        || lead.bytes().any(|c| c == b'.' || c == b'-' || c == b':')
        // Dialect elisions: "countin'", "'em".
        || (trail.starts_with('\'') && !trail.starts_with("'s"))
        || lead.ends_with('\'')
        || trail.contains("..") && !trail.ends_with("...")
        || trail.contains(['.', ':']) && trail.len() > 1 && trail.bytes().skip(1).any(is_letter);
    match case_of(core) {
        None => protected = true, // ALL CAPS / camelCase
        Some(Case::Capital) if !sentence_start && !(lower == "i" || lower.starts_with("i'")) => protected = true,
        _ => {}
    }
    Token {
        lead: lead.to_string(),
        core: core.to_string(),
        trail: trail.to_string(),
        core_start: s,
        protected,
        ends_sentence,
    }
}

fn case_of(core: &str) -> Option<Case> {
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

/// Apply the typed word's capitalisation to a lowercase replacement.
fn recase(word: &str, capital: bool) -> String {
    let mut out = String::with_capacity(word.len());
    for (i, w) in word.split(' ').enumerate() {
        if i > 0 {
            out.push(' ');
        }
        let make_cap = (capital && i == 0) || w == "i" || w.starts_with("i'");
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

pub struct Stream<'e> {
    pub engine: &'e Engine,
    pub words: Vec<Word>,
    pub enabled: bool,
    ignore: HashSet<String>,
}

impl<'e> Stream<'e> {
    pub fn new(engine: &'e Engine) -> Self {
        Stream { engine, words: Vec::new(), enabled: true, ignore: HashSet::new() }
    }

    /// Index of the last non-empty chunk before `i`.
    fn prev_index(&self, i: usize) -> Option<usize> {
        (0..i).rev().find(|&k| !self.words[k].output.is_empty())
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
        let reseg = self.try_boundary(i);
        if !reseg.is_empty() {
            return reseg;
        }
        let ctx = Ctx { prev: self.ctx_prev(i), next: None, next_unknown: false };
        if let Some(e) = self.try_word(i, ctx, None, false) {
            edits.push(e);
        }
        if self.engine.cfg.right_context {
            if let Some(p) = self.prev_index(i) {
                let pw = &self.words[p];
                if !pw.changed && !pw.protected && !pw.tok.ends_sentence {
                    let next = self.words[i].out_id;
                    if next.is_some() {
                        let ctx = Ctx { prev: self.ctx_prev(p), next, next_unknown: false };
                        // Real words need tau_revise; a non-word just needs the usual bar.
                        let known = self.engine.lx.id(&pw.tok.core.to_ascii_lowercase()).is_some();
                        let tau = if known { Some(self.engine.cfg.tau_revise) } else { None };
                        if let Some(e) = self.try_word(p, ctx, tau, true) {
                            edits.push(e);
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
        let s = self.engine.decide(&lower, w.core_times(), &ctx, tau)?;
        let new = format!("{}{}{}", w.tok.lead, recase(&s.text, w.capital()), w.tok.trail);
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
                let new = format!("{}{}", recase(&word, capital), &raw[b.len()..]);
                let old = self.set_output(i, new.clone());
                return Some(Edit { index: i, old, new, confidence: 1.0, revision: false });
            }
        }
        None
    }

    /// Reconsider the space between the previous chunk and chunk `i`.
    fn try_boundary(&mut self, i: usize) -> Vec<Edit> {
        let Some(p) = self.prev_index(i) else { return Vec::new() };
        if p + 1 != i {
            return Vec::new();
        }
        let (pw, cw) = (&self.words[p], &self.words[i]);
        let eligible = |w: &Word| !w.protected && !w.resegmented && w.tok.core.bytes().all(|c| c.is_ascii_alphabetic());
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
        let joined = format!("{a}{b}");
        let joined_times: Option<Vec<f32>> = match (pw.core_times(), cw.core_times()) {
            (Some(x), Some(y)) => Some(x.iter().chain(y).copied().collect()),
            _ => None,
        };
        let jt = joined_times.as_deref();
        let ctx = |prev| Ctx { prev, next: None, next_unknown: false };
        // Score of reading `x` as word(s); an unknown word can never be the
        // *result* of moving a space, so it costs infinity there.
        let known = |text: &str| text.split(' ').all(|w| lx.id(w).is_some());
        let pair = |k: usize, strict: bool| -> (String, String, f32) {
            let (x, y) = joined.split_at(k);
            let (tx, ty) = match jt {
                Some(t) => (Some(&t[..k]), Some(&t[k..])),
                None => (None, None),
            };
            let (bx, sx) = e.best(x, tx, &ctx(prev));
            let (by, sy) = e.best(y, ty, &ctx(lx.id(bx.rsplit(' ').next().unwrap_or(&bx))));
            let single = |t: &str| !t.contains(' ');
            let ok = !strict || (known(&bx) && known(&by) && single(&bx) && single(&by));
            (bx, by, if ok { sx + sy } else { f32::INFINITY })
        };
        // As typed (each side may still be spell-corrected).
        let (base_a, base_b, base) = pair(a.len(), false);
        let mut alts: Vec<(String, String, f32)> = Vec::new();
        let (m, sm) = e.best(&joined, jt, &ctx(prev));
        let base_known = known(&base_a) && known(&base_b);
        // Gluing two real words together is how compounds get mangled
        // ("soy beans" -> "soybeans"): demand much stronger evidence.
        let merge_extra = if base_known { e.cfg.merge_margin } else { 0.0 };
        let sm = if known(&m) { sm + merge_extra } else { f32::INFINITY };
        alts.push((m, String::new(), sm + e.cfg.space_cost));
        for d in [-2i32, -1, 1, 2] {
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
            (format!("{lead_p}{}{trail_c}", recase(&best_alt.0, cap_p)), String::new())
        } else {
            (format!("{lead_p}{}", recase(&best_alt.0, cap_p)), format!("{}{trail_c}", recase(&best_alt.1, false)))
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
        self.ignore.insert(lower);
        self.ignore.insert(w.typed.to_ascii_lowercase());
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
        assert!(parse_token("Smith", false).protected);
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
