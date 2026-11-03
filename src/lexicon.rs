//! Dictionary, word frequencies, word-pair frequencies and the trie used for
//! candidate search.

use std::collections::HashMap;

pub use crate::ngrams::{write_ngrams, NgramCounts};
use crate::ngrams::Ngrams;

pub type WordId = u32;
pub const NO_WORD: u32 = u32::MAX;

/// Compact trie in CSR form: node `n`'s children are
/// `child_chars[child_start[n]..child_start[n+1]]` (sorted) with matching
/// `child_nodes`.
pub struct Trie {
    pub child_start: Vec<u32>,
    pub child_chars: Vec<u8>,
    pub child_nodes: Vec<u32>,
    pub node_word: Vec<WordId>,
}

impl Trie {
    #[inline]
    pub fn children(&self, n: u32) -> impl Iterator<Item = (u8, u32)> + '_ {
        let s = self.child_start[n as usize] as usize;
        let e = self.child_start[n as usize + 1] as usize;
        self.child_chars[s..e].iter().copied().zip(self.child_nodes[s..e].iter().copied())
    }
}

pub struct Lexicon {
    pub words: Vec<String>,
    pub counts: Vec<f64>,
    pub total: f64,
    index: HashMap<String, WordId>,
    bigrams: HashMap<(WordId, WordId), f64>,
    /// Bigram counts in the source data are on a ~60x larger scale than the
    /// unigram counts (different corpus snapshot). Estimated from pairs like
    /// "able to", where nearly every "able" is followed by "to".
    bigram_scale: f64,
    pub trie: Trie,
    /// Corpus bigram/trigram counts (Wikipedia + conversational text).
    pub ngrams: Option<Ngrams>,
    /// Words normally written capitalised (London, Monday): share of
    /// mid-sentence uses that are capitalised.
    caps: HashMap<WordId, f32>,
    /// Letter-trigram model over dictionary word types: how "English-looking"
    /// a string is. log P(c | two previous letters), add-one smoothed.
    chars: HashMap<[u8; 3], f32>,
    char_ctx: HashMap<[u8; 2], f32>,
}

/// Knobs for the context model.
#[derive(Clone, Copy, Debug)]
pub struct LmParams {
    /// Weight of the bigram estimate vs. the unigram prior.
    pub lambda: f64,
    /// Max weight of the trigram estimate.
    pub lambda3: f64,
    /// Trigram confidence: weight = lambda3 * c(u,v) / (c(u,v) + k3).
    pub k3: f64,
    /// Use corpus trigrams at all.
    pub trigrams: bool,
    /// Max 4-gram weight and its confidence constant (0 = off).
    pub lambda4: f64,
    pub k4: f64,
}

const UNIGRAMS: &str = include_str!("../data/en_unigrams.txt");
const BIGRAMS: &str = include_str!("../data/en_bigrams.txt");
const WORDFREQ: &str = include_str!("../data/en_wordfreq.txt");
const SUPPLEMENT: &str = include_str!("../data/en_supplement.txt");
const NGRAMS: &[u8] = include_bytes!("../data/en_ngrams.bin");
const CAPITALIZED: &str = include_str!("../data/en_capitalized.txt");

fn valid_word(w: &str) -> bool {
    let b = w.as_bytes();
    // Elongations like "hellll" / "sooo" are web noise, and they would compete
    // with the real word they stretch.
    let tripled = b.windows(3).any(|t| t[0] == t[1] && t[1] == t[2]);
    // Single letters other than "a" and "i" are not words people type on purpose.
    let stray_letter = b.len() == 1 && w != "a" && w != "i";
    !w.is_empty() && !tripled && !stray_letter && b.iter().all(|&c| c.is_ascii_lowercase() || c == b'\'')
}

impl Lexicon {
    /// Built-in English data.
    pub fn english() -> Self {
        let mut lx = Self::english_without_ngrams();
        lx.ngrams = Ngrams::load(NGRAMS, lx.len(), |w| lx.id(w));
        for line in CAPITALIZED.lines().filter(|l| !l.starts_with('#')) {
            if let Some((w, r)) = line.split_once(' ') {
                if let (Some(id), Ok(r)) = (lx.id(w), r.parse::<f32>()) {
                    lx.caps.insert(id, r);
                }
            }
        }
        lx
    }

    /// The dictionary and Google pair counts only (used to build the n-grams).
    pub fn english_without_ngrams() -> Self {
        let extra = format!("{WORDFREQ}\n{SUPPLEMENT}");
        Self::from_sources(UNIGRAMS, BIGRAMS, &extra)
    }

    pub fn from_sources(unigrams: &str, bigrams: &str, supplement: &str) -> Self {
        let mut counts_map: HashMap<String, f64> = HashMap::new();
        for line in unigrams.lines().chain(supplement.lines()) {
            if line.starts_with('#') {
                continue;
            }
            let mut it = line.split_whitespace();
            if let (Some(w), Some(c)) = (it.next(), it.next()) {
                if valid_word(w) {
                    if let Ok(c) = c.parse::<f64>() {
                        counts_map.insert(w.to_string(), c);
                    }
                }
            }
        }
        let mut entries: Vec<(String, f64)> = counts_map.into_iter().collect();
        entries.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap().then(a.0.cmp(&b.0)));
        let words: Vec<String> = entries.iter().map(|e| e.0.clone()).collect();
        let counts: Vec<f64> = entries.iter().map(|e| e.1).collect();
        let total = counts.iter().sum();
        let index: HashMap<String, WordId> =
            words.iter().enumerate().map(|(i, w)| (w.clone(), i as WordId)).collect();

        let mut bigram_map = HashMap::new();
        for line in bigrams.lines() {
            let mut it = line.split_whitespace();
            if let (Some(a), Some(b), Some(c)) = (it.next(), it.next(), it.next()) {
                if let (Some(&ia), Some(&ib), Ok(c)) = (index.get(a), index.get(b), c.parse::<f64>()) {
                    bigram_map.insert((ia, ib), c);
                }
            }
        }

        let trie = build_trie(&words);
        // Letter trigrams over word types (each word counted once).
        let mut chars: HashMap<[u8; 3], f32> = HashMap::new();
        let mut char_ctx: HashMap<[u8; 2], f32> = HashMap::new();
        for w in &words {
            let b: Vec<u8> = [b'^', b'^'].into_iter().chain(w.bytes()).chain([b'$']).collect();
            for t in b.windows(3) {
                *chars.entry([t[0], t[1], t[2]]).or_insert(0.0) += 1.0;
                *char_ctx.entry([t[0], t[1]]).or_insert(0.0) += 1.0;
            }
        }
        Lexicon { words, counts, total, index, bigrams: bigram_map, bigram_scale: 60.0, trie, ngrams: None, caps: HashMap::new(), chars, char_ctx }
    }

    /// Lowest log-probability of any letter in `w` under the letter-trigram
    /// model (nats). Real words and names have no "impossible" spots; typos
    /// often do ("chjna": h-j-n).
    pub fn worst_letter(&self, w: &str) -> f32 {
        let b: Vec<u8> = [b'^', b'^'].into_iter().chain(w.bytes().map(|c| c.to_ascii_lowercase())).chain([b'$']).collect();
        b.windows(3)
            .map(|t| {
                let c = self.chars.get(&[t[0], t[1], t[2]]).copied().unwrap_or(0.0);
                let n = self.char_ctx.get(&[t[0], t[1]]).copied().unwrap_or(0.0);
                ((c + 0.1) / (n + 2.8)).ln()
            })
            .fold(0.0, f32::min)
    }

    /// Share of mid-sentence uses written capitalised (0 if not a proper noun).
    pub fn cap_share(&self, w: &str) -> f32 {
        self.id(w).and_then(|id| self.caps.get(&id).copied()).unwrap_or(0.0)
    }

    pub fn id(&self, w: &str) -> Option<WordId> {
        self.index.get(w).copied()
    }

    pub fn word(&self, id: WordId) -> &str {
        &self.words[id as usize]
    }

    pub fn len(&self) -> usize {
        self.words.len()
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Is "a b" a frequent pair in the bigram data?
    pub fn has_bigram(&self, a: WordId, b: WordId) -> bool {
        self.bigrams.contains_key(&(a, b)) || self.ngrams.as_ref().is_some_and(|n| n.bigram(a, b) >= 20)
    }

    /// Has "a b" been seen at all (either source)?
    pub fn seen_bigram(&self, a: WordId, b: WordId) -> bool {
        self.bigrams.contains_key(&(a, b)) || self.ngrams.as_ref().is_some_and(|n| n.bigram(a, b) > 0)
    }

    /// P(w | prev) from the pair data: the larger of the Google Books estimate
    /// and the corpus estimate (each list misses many pairs the other has).
    fn p_bigram(&self, prev: WordId, w: WordId) -> f64 {
        let g = self
            .bigrams
            .get(&(prev, w))
            .map(|c| (c / (self.bigram_scale * self.counts[prev as usize])).min(1.0))
            .unwrap_or(0.0);
        let c = match &self.ngrams {
            Some(n) => {
                let cp = n.unigram(prev);
                if cp >= 50 {
                    n.bigram(prev, w) as f64 / cp as f64
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        g.max(c)
    }

    /// Negative log probability (nats) of `w` after `prev2 prev`.
    /// Interpolated trigram -> bigram -> unigram.
    /// Negative log probability of `w` after `prev3 prev2 prev`:
    /// interpolated 4-gram -> trigram -> bigram -> unigram.
    pub fn cost4(&self, prev3: Option<WordId>, prev2: Option<WordId>, prev: Option<WordId>, w: WordId, p: &LmParams) -> f64 {
        if let (Some(p3), Some(p2), Some(p1), Some(n)) = (prev3, prev2, prev, &self.ngrams) {
            if p.lambda4 > 0.0 && p.trigrams {
                let ctx = n.trigram(p3, p2, p1) as f64;
                if ctx > 0.0 {
                    let l4 = p.lambda4 * ctx / (ctx + p.k4);
                    let p4 = (n.fourgram(p3, p2, p1, w) as f64 / ctx).min(1.0);
                    let p3g = (-self.cost3(prev2, prev, w, p)).exp();
                    return -(l4 * p4 + (1.0 - l4) * p3g).ln();
                }
            }
        }
        self.cost3(prev2, prev, w, p)
    }

    pub fn cost3(&self, prev2: Option<WordId>, prev: Option<WordId>, w: WordId, p: &LmParams) -> f64 {
        let pu = self.unigram_p(w);
        let Some(p1) = prev else { return -pu.ln() };
        let p2gram = p.lambda * self.p_bigram(p1, w) + (1.0 - p.lambda) * pu;
        let pr = match (prev2, &self.ngrams) {
            (Some(p0), Some(n)) if p.trigrams => {
                let ctx = n.bigram(p0, p1) as f64;
                if ctx > 0.0 {
                    let l3 = p.lambda3 * ctx / (ctx + p.k3);
                    let pt = n.trigram(p0, p1, w) as f64 / ctx;
                    l3 * pt.min(1.0) + (1.0 - l3) * p2gram
                } else {
                    p2gram
                }
            }
            _ => p2gram,
        };
        -pr.ln()
    }

    pub fn unigram_p(&self, w: WordId) -> f64 {
        self.counts[w as usize] / self.total
    }

    /// Negative log probability (nats) of `w` given the previous word.
    /// Interpolates a scaled bigram estimate with the unigram prior, so a pair
    /// missing from the (frequent-pairs-only) bigram list falls back gracefully.
    pub fn cost(&self, prev: Option<WordId>, w: WordId, lambda: f64) -> f64 {
        let pu = self.unigram_p(w);
        let p = match prev {
            Some(p) if lambda > 0.0 => {
                let pb = self
                    .bigrams
                    .get(&(p, w))
                    .map(|c| (c / (self.bigram_scale * self.counts[p as usize])).min(1.0))
                    .unwrap_or(0.0);
                lambda * pb + (1.0 - lambda) * pu
            }
            _ => pu,
        };
        -p.ln()
    }
}

fn build_trie(words: &[String]) -> Trie {
    // Build with a simple pointer trie, then flatten.
    struct N {
        kids: Vec<(u8, usize)>,
        word: WordId,
    }
    let mut nodes = vec![N { kids: Vec::new(), word: NO_WORD }];
    for (id, w) in words.iter().enumerate() {
        let mut cur = 0usize;
        for &b in w.as_bytes() {
            cur = match nodes[cur].kids.iter().find(|k| k.0 == b) {
                Some(&(_, n)) => n,
                None => {
                    nodes.push(N { kids: Vec::new(), word: NO_WORD });
                    let n = nodes.len() - 1;
                    nodes[cur].kids.push((b, n));
                    n
                }
            };
        }
        nodes[cur].word = id as WordId;
    }
    let mut child_start = Vec::with_capacity(nodes.len() + 1);
    let mut child_chars = Vec::new();
    let mut child_nodes = Vec::new();
    let mut node_word = Vec::with_capacity(nodes.len());
    for n in nodes.iter_mut() {
        n.kids.sort();
        child_start.push(child_chars.len() as u32);
        for &(c, k) in &n.kids {
            child_chars.push(c);
            child_nodes.push(k as u32);
        }
        node_word.push(n.word);
    }
    child_start.push(child_chars.len() as u32);
    Trie { child_start, child_chars, child_nodes, node_word }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_prefers_plausible_pairs() {
        let lx = Lexicon::english();
        let to = lx.id("to");
        let bed = lx.id("bed").unwrap();
        let bad = lx.id("bad").unwrap();
        // "went to bed" vs "went to bad"
        assert!(lx.cost(to, bed, 0.6) < lx.cost(to, bad, 0.6));
        // Without context, "bad" is the more common word.
        assert!(lx.cost(None, bad, 0.6) < lx.cost(None, bed, 0.6));
        assert!(lx.id("don't").is_some());
    }
}
