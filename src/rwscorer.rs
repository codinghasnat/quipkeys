//! The real-word scorer: was "form" in "a letter form the bank" a slip of
//! "from"? A small network (trained by scripts/nn/rw_train.py on real English
//! with slips swapped in) reads the 5 tokens on each side of a typed real word
//! and gives each of its look-alikes (sound-alikes and one-key slips, listed in
//! the model file) a probability of being what was meant. It was trained on
//! clean text as much as on slips, so "Please fill out the form" stays "form".
//!
//! Pure Rust, about 3 MB of weights, a few microseconds per word.

use std::collections::HashMap;

/// The model the engine uses (scripts/nn/rw_train.py; about 3 MB).
static MODEL: &[u8] = include_bytes!("../data/en_rw.bin");

/// The built-in English scorer, loaded on first use.
pub fn english() -> Option<&'static RwScorer> {
    static S: std::sync::OnceLock<Option<RwScorer>> = std::sync::OnceLock::new();
    S.get_or_init(|| RwScorer::load(MODEL)).as_ref()
}

/// Relation code for pairs from the hand-made list (rw_data.py REL_LIST).
const REL_LIST: u8 = 1;

/// US/UK spelling pairs ("color", "colour"): never a slip.
fn variants() -> &'static std::collections::HashSet<(String, String)> {
    static V: std::sync::OnceLock<std::collections::HashSet<(String, String)>> = std::sync::OnceLock::new();
    V.get_or_init(|| {
        let mut set = std::collections::HashSet::new();
        for line in include_str!("../data/en_variants.txt").lines().filter(|l| !l.starts_with('#')) {
            let mut it = line.split_whitespace();
            if let (Some(a), Some(b)) = (it.next(), it.next()) {
                set.insert((a.to_string(), b.to_string()));
                set.insert((b.to_string(), a.to_string()));
            }
        }
        set
    })
}

/// The words differ in their first letter only by one being added or
/// dropped ("other"/"mother", "at"/"sat"): rare as a slip, common as two
/// different words, so never offered.
fn first_letter_edit(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() < b.len() { (a, b) } else { (b, a) };
    long.len() == short.len() + 1 && long[1..] == *short
}

/// Relation codes (rw_data.py): which kind of slip turns the meant word into the typed one.
pub const REL_SUB: u8 = 2;
pub const REL_DROP: u8 = 3;
pub const REL_EXTRA: u8 = 4;
pub const REL_SWAP: u8 = 5;

/// One word is the other plus a final "s" or "d".
fn inflection(a: &str, b: &str) -> bool {
    let (short, long) = if a.len() < b.len() { (a, b) } else { (b, a) };
    long.len() == short.len() + 1 && long.starts_with(short) && (long.ends_with('s') || long.ends_with('d'))
}

pub const PAD: u16 = 0;
pub const UNK: u16 = 1;

pub struct RwScorer {
    pub vocab: Vec<String>,
    index: HashMap<String, u16>,
    d: usize,
    pub ctx: usize,
    hidden: usize,
    k: usize,
    emb_scale: Vec<f32>,
    emb_q: Vec<i8>,
    w1: Vec<f32>,
    b1: Vec<f32>,
    w2: Vec<f32>,
    b2: Vec<f32>,
    bias: Vec<f32>,
    keep: Vec<f32>,
    rel: Vec<f32>,
    count: Vec<u8>,
    table: Vec<u16>,
    trel: Vec<u8>,
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.p..self.p + n)?;
        self.p += n;
        Some(s)
    }
    fn u32(&mut self) -> Option<usize> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize)
    }
    fn f32s(&mut self, n: usize) -> Option<Vec<f32>> {
        Some(self.take(n * 4)?.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect())
    }
}

/// erf, to within 1.5e-7 (Abramowitz and Stegun 7.1.26), for the exact GELU.
fn erf(x: f32) -> f32 {
    let s = x.signum();
    let x = x.abs() as f64;
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0 - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t + 0.254_829_592) * t * (-x * x).exp();
    s * y as f32
}

fn gelu(x: f32) -> f32 {
    0.5 * x * (1.0 + erf(x / std::f32::consts::SQRT_2))
}

/// Split text into the scorer's tokens: lower-case words (apostrophes kept
/// inside) and the punctuation marks . , ! ? ; : (as in rw_data.py).
pub fn tokens(text: &str) -> Vec<String> {
    let t = text.to_lowercase().replace('\u{2019}', "'");
    let b: Vec<char> = t.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_lowercase() {
            let mut j = i;
            while j < b.len() && b[j].is_ascii_lowercase() {
                j += 1;
            }
            // One apostrophe followed by letters stays in the word ("you're").
            if j + 1 < b.len() && b[j] == '\'' && b[j + 1].is_ascii_lowercase() {
                j += 1;
                while j < b.len() && b[j].is_ascii_lowercase() {
                    j += 1;
                }
            }
            out.push(b[i..j].iter().collect());
            i = j;
        } else {
            if ".,!?;:".contains(c) {
                out.push(c.to_string());
            }
            i += 1;
        }
    }
    out
}

impl RwScorer {
    pub fn load(bytes: &[u8]) -> Option<RwScorer> {
        let mut r = Reader { b: bytes, p: 0 };
        if r.take(8)? != b"QKRW1\0\0\0" {
            return None;
        }
        let (v, d, ctx, hidden, k, nrel) = (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
        let mut vocab = Vec::with_capacity(v);
        for _ in 0..v {
            let n = r.take(1)?[0] as usize;
            vocab.push(String::from_utf8_lossy(r.take(n)?).into_owned());
        }
        let emb_scale = r.f32s(v)?;
        let emb_q = r.take(v * d)?.iter().map(|&x| x as i8).collect();
        let w1 = r.f32s(hidden * 2 * ctx * d)?;
        let b1 = r.f32s(hidden)?;
        let w2 = r.f32s(d * hidden)?;
        let b2 = r.f32s(d)?;
        let bias = r.f32s(v)?;
        let keep = r.f32s(v)?;
        let rel = r.f32s(nrel)?;
        let count = r.take(v)?.to_vec();
        let table = r.take(v * k * 2)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let trel = r.take(v * k)?.to_vec();
        let index = vocab.iter().enumerate().map(|(i, w)| (w.clone(), i as u16)).collect();
        Some(RwScorer { vocab, index, d, ctx, hidden, k, emb_scale, emb_q, w1, b1, w2, b2, bias, keep, rel, count, table, trel })
    }

    pub fn id(&self, token: &str) -> u16 {
        self.index.get(token).copied().unwrap_or(UNK)
    }

    /// Does this typed word have look-alikes the scorer can judge?
    pub fn judges(&self, typed: &str) -> bool {
        self.index.get(typed).is_some_and(|&i| self.count[i as usize] > 1)
    }

    fn emb(&self, id: u16) -> impl Iterator<Item = f32> + '_ {
        let i = id as usize;
        let s = self.emb_scale[i];
        self.emb_q[i * self.d..(i + 1) * self.d].iter().map(move |&q| q as f32 * s)
    }

    /// What the typed word (lower case) probably was, given the tokens before
    /// it (`left`, nearest last) and after it (`right`, nearest first): each
    /// look-alike with its probability, the typed word first. `keep_bonus`
    /// is added to the typed word's score (more = changes words less often).
    /// None if the word isn't one the scorer knows look-alikes for.
    /// `rels` is a bit mask of the relations (slip kinds) offered: bit r for code r.
    pub fn judge(&self, left: &[String], typed: &str, right: &[String], keep_bonus: f32, rels: u8) -> Option<Vec<(String, f32)>> {
        let t = *self.index.get(typed)?;
        let n = self.count[t as usize] as usize;
        if n < 2 {
            return None;
        }
        // Context: CTX tokens before (padded at the start), CTX after (padded at the end).
        let mut ids = vec![PAD; 2 * self.ctx];
        for (j, tok) in left.iter().rev().take(self.ctx).enumerate() {
            ids[self.ctx - 1 - j] = self.id(tok);
        }
        for (j, tok) in right.iter().take(self.ctx).enumerate() {
            ids[self.ctx + j] = self.id(tok);
        }
        let x: Vec<f32> = ids.iter().flat_map(|&i| self.emb(i)).collect();
        let cols = 2 * self.ctx * self.d;
        let hid: Vec<f32> = (0..self.hidden)
            .map(|r| gelu(self.w1[r * cols..(r + 1) * cols].iter().zip(&x).map(|(a, b)| a * b).sum::<f32>() + self.b1[r]))
            .collect();
        let h: Vec<f32> =
            (0..self.d).map(|r| self.w2[r * self.hidden..(r + 1) * self.hidden].iter().zip(&hid).map(|(a, b)| a * b).sum::<f32>() + self.b2[r]).collect();
        let row = t as usize * self.k;
        let typed_word = &self.vocab[t as usize];
        let mut scored: Vec<(String, f32)> = (0..n)
            // A plural "s" or past-tense "d" added or dropped ("letter" ->
            // "letters", "use" -> "used") is a real slip, but context rarely
            // settles it and a wrong change reads badly: never offered
            // (unless the pair is on the hand-made list).
            .filter(|&j| {
                let (r, c) = (self.trel[row + j], &self.vocab[self.table[row + j] as usize]);
                j == 0
                    || r == REL_LIST
                    || (rels >> r) & 1 == 1
                        && !inflection(typed_word, c)
                        && !first_letter_edit(typed_word, c)
                        && !variants().contains(&(typed_word.clone(), c.clone()))
            })
            .map(|j| {
                let c = self.table[row + j];
                let mut s = self.emb(c).zip(&h).map(|(a, b)| a * b).sum::<f32>() + self.bias[c as usize] + self.rel[self.trel[row + j] as usize];
                if j == 0 {
                    s += self.keep[t as usize] + keep_bonus;
                }
                (self.vocab[c as usize].clone(), s)
            })
            .collect();
        let max = scored.iter().map(|s| s.1).fold(f32::NEG_INFINITY, f32::max);
        let z: f32 = scored.iter().map(|s| (s.1 - max).exp()).sum();
        for s in &mut scored {
            s.1 = (s.1 - max).exp() / z;
        }
        Some(scored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_match_the_training_script() {
        assert_eq!(tokens("I got a letter form the bank. Their is"), ["i", "got", "a", "letter", "form", "the", "bank", ".", "their", "is"]);
        assert_eq!(tokens("You\u{2019}re \"done\", aren't-you?"), ["you're", "done", ",", "aren't", "you", "?"]);
        assert_eq!(tokens("rock'n'roll 3pm"), ["rock'n", "roll", "pm"]);
    }

    #[test]
    fn inflections_are_spotted() {
        assert!(inflection("letter", "letters") && inflection("used", "use"));
        assert!(!inflection("the", "they") && !inflection("form", "from"));
        // "an"/"and" counts too, but it's on the hand-made list, which is exempt.
        assert!(inflection("an", "and"));
        assert!(first_letter_edit("other", "mother") && first_letter_edit("sat", "at"));
        assert!(!first_letter_edit("form", "from") && !first_letter_edit("the", "them"));
        assert!(variants().contains(&("color".into(), "colour".into())));
    }

    #[test]
    fn erf_is_close() {
        assert!((erf(0.5) - 0.520_499_9).abs() < 1e-5);
        assert!((erf(-1.2) + 0.910_314).abs() < 1e-5);
        assert!((gelu(1.0) - 0.841_344_7).abs() < 1e-5);
    }
}
