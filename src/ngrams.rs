//! Corpus bigram/trigram counts (built by `qk-ngrams`), stored as sorted
//! arrays and looked up by binary search: compact, fast to load, no hashing.
//!
//! File format (little-endian):
//!   "QKNG1\n", u32 V, V words each followed by '\n', V varint unigram counts,
//!   u32 N2, N2 x (varint key-delta, varint count)   key = a*V + b
//!   u32 N3, N3 x (varint key-delta, varint count)   key = (a*V + b)*V + c

use std::collections::HashMap;

use crate::lexicon::{Lexicon, WordId};

pub struct NgramCounts {
    /// Indexed by lexicon id.
    pub unigrams: Vec<u64>,
    /// Keys in lexicon-id space with V = lexicon size.
    pub bigrams: HashMap<u64, u32>,
    pub trigrams: HashMap<u64, u32>,
}

pub struct Ngrams {
    /// Lexicon id -> n-gram vocabulary id (u32::MAX if absent).
    map: Vec<u32>,
    uni: Vec<u32>,
    v: u64,
    bi_keys: Vec<u64>,
    bi_counts: Vec<u32>,
    tri_keys: Vec<u64>,
    tri_counts: Vec<u32>,
}

fn put_varint(out: &mut Vec<u8>, mut x: u64) {
    loop {
        let b = (x & 0x7f) as u8;
        x >>= 7;
        if x == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn get_varint(b: &[u8], pos: &mut usize) -> u64 {
    let mut x = 0u64;
    let mut shift = 0;
    loop {
        let c = b[*pos];
        *pos += 1;
        x |= ((c & 0x7f) as u64) << shift;
        if c & 0x80 == 0 {
            return x;
        }
        shift += 7;
    }
}

fn get_u32(b: &[u8], pos: &mut usize) -> u32 {
    let v = u32::from_le_bytes(b[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    v
}

pub fn write_ngrams(lx: &Lexicon, c: &NgramCounts) -> Vec<u8> {
    let lv = lx.len() as u64;
    // Vocabulary: every lexicon word seen in the corpus.
    let mut remap = vec![u32::MAX; lx.len()];
    let mut vocab = Vec::new();
    for (id, &n) in c.unigrams.iter().enumerate() {
        if n > 0 {
            remap[id] = vocab.len() as u32;
            vocab.push(id);
        }
    }
    let v = vocab.len() as u64;
    let mut out = b"QKNG1\n".to_vec();
    out.extend((v as u32).to_le_bytes());
    for &id in &vocab {
        out.extend(lx.word(id as WordId).as_bytes());
        out.push(b'\n');
    }
    for &id in &vocab {
        put_varint(&mut out, c.unigrams[id].min(u32::MAX as u64));
    }
    let mut bi: Vec<(u64, u32)> = c
        .bigrams
        .iter()
        .map(|(&k, &n)| ((remap[(k / lv) as usize] as u64) * v + remap[(k % lv) as usize] as u64, n))
        .collect();
    bi.sort_unstable();
    out.extend((bi.len() as u32).to_le_bytes());
    let mut last = 0;
    for (k, n) in bi {
        put_varint(&mut out, k - last);
        put_varint(&mut out, n as u64);
        last = k;
    }
    let mut tri: Vec<(u64, u32)> = c
        .trigrams
        .iter()
        .map(|(&k, &n)| {
            let (ab, w) = (k / lv, k % lv);
            let (a, b) = (ab / lv, ab % lv);
            let r = |x: u64| remap[x as usize] as u64;
            ((r(a) * v + r(b)) * v + r(w), n)
        })
        .collect();
    tri.sort_unstable();
    out.extend((tri.len() as u32).to_le_bytes());
    let mut last = 0;
    for (k, n) in tri {
        put_varint(&mut out, k - last);
        put_varint(&mut out, n as u64);
        last = k;
    }
    out
}

impl Ngrams {
    /// Parse the binary file; `id_of` maps a word to its lexicon id.
    pub fn load(bytes: &[u8], lex_len: usize, id_of: impl Fn(&str) -> Option<WordId>) -> Option<Ngrams> {
        if bytes.len() < 10 || &bytes[..6] != b"QKNG1\n" {
            return None;
        }
        let mut pos = 6;
        let v = get_u32(bytes, &mut pos) as usize;
        let mut map = vec![u32::MAX; lex_len];
        for i in 0..v {
            let end = pos + bytes[pos..].iter().position(|&c| c == b'\n')?;
            if let Some(id) = std::str::from_utf8(&bytes[pos..end]).ok().and_then(&id_of) {
                map[id as usize] = i as u32;
            }
            pos = end + 1;
        }
        let uni = (0..v).map(|_| get_varint(bytes, &mut pos) as u32).collect();
        let read = |pos: &mut usize| {
            let n = get_u32(bytes, pos) as usize;
            let (mut keys, mut counts) = (Vec::with_capacity(n), Vec::with_capacity(n));
            let mut k = 0u64;
            for _ in 0..n {
                k += get_varint(bytes, pos);
                keys.push(k);
                counts.push(get_varint(bytes, pos) as u32);
            }
            (keys, counts)
        };
        let (bi_keys, bi_counts) = read(&mut pos);
        let (tri_keys, tri_counts) = read(&mut pos);
        Some(Ngrams { map, uni, v: v as u64, bi_keys, bi_counts, tri_keys, tri_counts })
    }

    #[inline]
    fn m(&self, id: WordId) -> Option<u64> {
        let x = *self.map.get(id as usize)?;
        (x != u32::MAX).then_some(x as u64)
    }

    pub fn unigram(&self, a: WordId) -> u32 {
        self.m(a).map_or(0, |x| self.uni[x as usize])
    }

    pub fn bigram(&self, a: WordId, b: WordId) -> u32 {
        let (Some(a), Some(b)) = (self.m(a), self.m(b)) else { return 0 };
        let k = a * self.v + b;
        self.bi_keys.binary_search(&k).map_or(0, |i| self.bi_counts[i])
    }

    pub fn trigram(&self, a: WordId, b: WordId, c: WordId) -> u32 {
        let (Some(a), Some(b), Some(c)) = (self.m(a), self.m(b), self.m(c)) else { return 0 };
        let k = (a * self.v + b) * self.v + c;
        self.tri_keys.binary_search(&k).map_or(0, |i| self.tri_counts[i])
    }

    pub fn sizes(&self) -> (usize, usize) {
        (self.bi_keys.len(), self.tri_keys.len())
    }
}
