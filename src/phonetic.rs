//! Sounds-like matching: spelling by ear ("fone", "nollij", "edukashun").
//!
//! Every dictionary word gets one or more *sound codes*: its pronunciation
//! from the CMU Pronouncing Dictionary (data/en_phon.txt), plus a code from
//! simple letter-to-sound rules for words CMU lacks. A typed word gets a
//! code from the same rules, which read letters the plain way a phonetic
//! speller writes them. Words whose codes are close become candidates, with
//! a channel cost, alongside the keyboard-slip candidates; the sentence
//! decoder then lets context choose between sound-alikes.
//!
//! Sound code alphabet (one byte per sound):
//!   consonants  p b t d k g f v s m n l r w y h, q = th, x = sh/zh,
//!               c = ch, j = j/dge, N = ng (s and z are one sound here)
//!   vowels      1 ee, 2 i, 3 ay, 4 e, 5 a, 6 o/aw, 7 oh, 8 oo, 9 uh/er,
//!               I eye, W ow, Y oy

use std::collections::HashMap;

use crate::lexicon::{Lexicon, WordId};

const CMU: &str = include_str!("../data/en_phon.txt");

fn is_vowel_sound(c: u8) -> bool {
    matches!(c, b'1'..=b'9' | b'I' | b'W' | b'Y')
}

fn is_vowel_letter(c: u8) -> bool {
    matches!(c, b'a' | b'e' | b'i' | b'o' | b'u' | b'y')
}

/// Map one CMU phoneme (stress digits stripped) to sound-code bytes.
fn cmu_phone(p: &str, out: &mut Vec<u8>) {
    let s: &[u8] = match p {
        "P" => b"p", "B" => b"b", "T" => b"t", "D" => b"d", "K" => b"k", "G" => b"g",
        "F" => b"f", "V" => b"v", "TH" | "DH" => b"q", "S" | "Z" => b"s", "SH" | "ZH" => b"x",
        "CH" => b"c", "JH" => b"j", "M" => b"m", "N" => b"n", "NG" => b"N", "L" => b"l",
        "R" => b"r", "W" => b"w", "Y" => b"y", "HH" => b"h",
        "IY" => b"1", "IH" => b"2", "EY" => b"3", "EH" => b"4", "AE" => b"5", "AA" | "AO" => b"6",
        "OW" => b"7", "UH" | "UW" => b"8", "AH" => b"9", "ER" => b"9r", "AY" => b"I", "AW" => b"W",
        "OY" => b"Y",
        _ => b"",
    };
    out.extend_from_slice(s);
}

/// Sound codes for a spelling, read the plain phonetic way. Ambiguous letters
/// (c/g before e, i, y) give up to a few alternative codes.
pub fn spell_codes(word: &str) -> Vec<Vec<u8>> {
    let w: Vec<u8> = word.bytes().filter(|c| c.is_ascii_alphabetic()).map(|c| c.to_ascii_lowercase()).collect();
    let mut outs: Vec<Vec<u8>> = vec![Vec::new()];
    let n = w.len();
    let at = |i: usize| -> u8 { if i < n { w[i] } else { 0 } };
    let front = |c: u8| matches!(c, b'e' | b'i' | b'y');
    let starts = |i: usize, pat: &str| w[i..].starts_with(pat.as_bytes());
    let push_all = |outs: &mut Vec<Vec<u8>>, s: &[u8]| {
        for o in outs.iter_mut() {
            o.extend_from_slice(s);
        }
    };
    let branch = |outs: &mut Vec<Vec<u8>>, a: &[u8], b: &[u8]| {
        if outs.len() >= 4 {
            push_all(outs, a);
            return;
        }
        let mut more = outs.clone();
        for o in outs.iter_mut() {
            o.extend_from_slice(a);
        }
        for o in more.iter_mut() {
            o.extend_from_slice(b);
        }
        outs.extend(more);
    };
    let mut i = 0;
    while i < n {
        let c = w[i];
        let next = at(i + 1);
        // Silent starts.
        if i == 0 && (starts(0, "kn") || starts(0, "gn") || starts(0, "pn")) {
            push_all(&mut outs, b"n");
            i += 2;
            continue;
        }
        if i == 0 && starts(0, "wr") {
            push_all(&mut outs, b"r");
            i += 2;
            continue;
        }
        if i == 0 && starts(0, "ps") {
            push_all(&mut outs, b"s");
            i += 2;
            continue;
        }
        // Multi-letter consonant spellings.
        let multi: &[(&str, &[u8])] = &[
            ("tch", b"c"), ("dge", b"j"), ("sch", b"sk"), ("tion", b"x9n"), ("sion", b"x9n"), ("cian", b"x9n"),
            ("shun", b"x9n"), ("ture", b"c9r"), ("ph", b"f"), ("sh", b"x"), ("ch", b"c"), ("th", b"q"),
            ("ck", b"k"), ("zh", b"x"), ("qu", b"kw"), ("wh", b"w"), ("ng", b"N"), ("nk", b"Nk"), ("gh", b""), ("dj", b"j"),
            ("kw", b"kw"),
        ];
        if let Some((pat, out)) = multi.iter().find(|(p, _)| starts(i, p)) {
            // "gh" is silent after a vowel ("night"), a "g" otherwise ("ghost").
            if *pat == "gh" && (i == 0 || !is_vowel_letter(w[i - 1])) {
                push_all(&mut outs, b"g");
            } else {
                push_all(&mut outs, out);
            }
            i += pat.len();
            continue;
        }
        if is_vowel_letter(c) && !(c == b'y' && (i == 0 || (i + 1 < n && is_vowel_letter(next) && next != b'y'))) {
            // A vowel run, read as one vowel sound.
            let mut j = i;
            while j < n && is_vowel_letter(w[j]) && !(j > i && w[j] == b'y' && j + 1 < n && is_vowel_letter(w[j + 1])) {
                j += 1;
            }
            let run = &w[i..j];
            // Silent final e ("fone", "nite"), but not in "be", "the".
            if run == b"e" && j == n && i >= 2 {
                i = j;
                continue;
            }
            // Vowel + consonant + final e: the long vowel ("nite" -> eye, "fone" -> oh).
            let magic = run.len() == 1 && j + 1 == n - 1 && at(j + 1) == b'e' && !is_vowel_letter(at(j));
            let v: &[u8] = match run {
                b"ee" | b"ea" | b"ie" | b"ei" | b"ey" => b"1",
                b"oo" | b"ew" | b"ue" | b"ui" => b"8",
                b"ai" | b"ay" | b"ae" => b"3",
                b"oa" | b"oe" => b"7",
                b"aw" | b"au" | b"augh" => b"6",
                b"oi" | b"oy" => b"Y",
                b"ou" | b"ow" => b"W",
                b"igh" => b"I",
                b"y" if j == n => if n <= 3 { b"I" } else { b"1" },
                b"y" => b"2",
                b"a" if magic => b"3",
                b"e" if magic => b"1",
                b"i" if magic => b"I",
                b"o" if magic => b"7",
                b"u" if magic => b"8",
                b"a" => b"5",
                b"e" => b"4",
                b"i" => b"2",
                b"o" => b"6",
                b"u" => b"9",
                _ => b"9",
            };
            // Spellings with two common readings: try both.
            let alt: Option<&[u8]> = match run {
                b"ie" => Some(b"I9"),
                b"ea" => Some(b"4"),
                b"ou" => Some(b"8"),
                b"ow" => Some(b"7"),
                b"ei" => Some(b"3"),
                _ => None,
            };
            match alt {
                Some(a) if !magic => branch(&mut outs, v, a),
                _ => push_all(&mut outs, v),
            }
            i = j;
            continue;
        }
        // Doubled consonants sound once.
        if i > 0 && w[i - 1] == c && !is_vowel_letter(c) {
            i += 1;
            continue;
        }
        match c {
            b'c' if front(next) => branch(&mut outs, b"s", b"k"),
            b'c' | b'k' | b'q' => push_all(&mut outs, b"k"),
            b'g' if front(next) => branch(&mut outs, b"j", b"g"),
            b'x' => push_all(&mut outs, if i == 0 { b"s" } else { b"ks" }),
            b'z' => push_all(&mut outs, b"s"),
            b'y' => push_all(&mut outs, b"y"),
            _ => push_all(&mut outs, &[c]),
        }
        i += 1;
    }
    outs.sort();
    outs.dedup();
    outs
}

/// Consonant skeleton of a code (vowels and the weak h/w/y dropped).
fn skeleton(code: &[u8]) -> Vec<u8> {
    code.iter().copied().filter(|&c| !is_vowel_sound(c) && !matches!(c, b'h' | b'w' | b'y')).collect()
}

fn group(c: u8) -> u8 {
    match c {
        b's' | b'x' | b'c' | b'j' => 1,
        b't' | b'd' => 2,
        b'p' | b'b' => 3,
        b'k' | b'g' => 4,
        b'f' | b'v' | b'q' => 5,
        b'm' | b'n' | b'N' => 6,
        _ => 0,
    }
}

fn sub_cost(a: u8, b: u8) -> f32 {
    if a == b {
        0.0
    } else if is_vowel_sound(a) && is_vowel_sound(b) {
        vowel_sub(a, b)
    } else if is_vowel_sound(a) || is_vowel_sound(b) {
        1.5
    } else if group(a) != 0 && group(a) == group(b) {
        0.7
    } else {
        1.3
    }
}

/// Vowel confusions: the neutral "uh" is close to everything, neighbouring
/// vowel sounds are close, distant ones less so.
fn vowel_sub(a: u8, b: u8) -> f32 {
    if a == b'9' || b == b'9' {
        return 0.3;
    }
    const NEAR: &[(u8, u8)] = &[
        (b'1', b'2'), (b'2', b'4'), (b'4', b'5'), (b'3', b'4'), (b'1', b'3'), (b'5', b'6'), (b'6', b'7'),
        (b'7', b'8'), (b'2', b'I'), (b'1', b'I'), (b'6', b'W'), (b'7', b'W'), (b'6', b'Y'), (b'2', b'8'),
    ];
    if NEAR.iter().any(|&(x, y)| (x, y) == (a, b) || (y, x) == (a, b)) {
        0.4
    } else {
        0.8
    }
}

fn indel_cost(c: u8) -> f32 {
    if is_vowel_sound(c) {
        0.4
    } else if matches!(c, b'h' | b'w' | b'y' | b'r') {
        0.5
    } else {
        1.0
    }
}

/// Weighted edit distance between two sound codes.
pub fn distance(a: &[u8], b: &[u8]) -> f32 {
    let m = b.len();
    let mut prev: Vec<f32> = (0..=m).map(|j| b[..j].iter().map(|&c| indel_cost(c)).sum()).collect();
    let mut cur = vec![0f32; m + 1];
    for i in 1..=a.len() {
        cur[0] = prev[0] + indel_cost(a[i - 1]);
        for j in 1..=m {
            cur[j] = (prev[j - 1] + sub_cost(a[i - 1], b[j - 1]))
                .min(prev[j] + indel_cost(a[i - 1]))
                .min(cur[j - 1] + indel_cost(b[j - 1]));
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[m]
}

fn hash(s: &[u8]) -> u64 {
    // FNV-1a
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &c in s {
        h ^= c as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

pub struct Phonetic {
    /// Sound codes of each word id.
    codes: Vec<Vec<Vec<u8>>>,
    /// Skeleton (and its one-deletions) -> word ids.
    index: HashMap<u64, Vec<WordId>>,
    /// Exact CMU pronunciation -> word ids (homophones).
    by_sound: HashMap<u64, Vec<WordId>>,
}

impl Phonetic {
    pub fn build(lx: &Lexicon) -> Phonetic {
        let n = lx.len();
        let mut codes: Vec<Vec<Vec<u8>>> = vec![Vec::new(); n];
        let mut by_sound: HashMap<u64, Vec<WordId>> = HashMap::new();
        for line in CMU.lines() {
            let mut parts = line.split_whitespace();
            let Some(word) = parts.next() else { continue };
            if word.starts_with('#') {
                continue;
            }
            let Some(id) = lx.id(word) else { continue };
            // Abbreviations (tv, mr, st) are pronounced as the full word:
            // they must not compete with it.
            if !word.bytes().any(is_vowel_letter) {
                continue;
            }
            let mut code = Vec::new();
            for p in parts {
                cmu_phone(p.trim_end_matches(|c: char| c.is_ascii_digit()), &mut code);
            }
            if !code.is_empty() && code.len() <= word.len() * 2 {
                by_sound.entry(hash(&code)).or_default().push(id);
                codes[id as usize].push(code);
            }
        }
        let mut index: HashMap<u64, Vec<WordId>> = HashMap::new();
        for (id, cs) in codes.iter_mut().enumerate() {
            let word = lx.word(id as WordId);
            if word.len() < 2 || !word.bytes().all(|c| c.is_ascii_lowercase()) {
                continue;
            }
            // Words CMU lacks get their spelling read by the rules.
            if cs.is_empty() {
                *cs = spell_codes(word);
            }
            let mut keys: Vec<u64> = Vec::new();
            for c in cs.iter() {
                let sk = skeleton(c);
                keys.push(hash(&sk));
                for k in 0..sk.len() {
                    let mut d = sk.clone();
                    d.remove(k);
                    keys.push(hash(&d));
                }
            }
            keys.sort_unstable();
            keys.dedup();
            for k in keys {
                index.entry(k).or_default().push(id as WordId);
            }
        }
        for ids in by_sound.values_mut() {
            ids.sort_unstable();
            ids.dedup();
        }
        Phonetic { codes, index, by_sound }
    }

    /// Dictionary words that sound like `typed`, with their sound distance
    /// (at most `max`).
    pub fn candidates(&self, typed: &str, max: f32) -> Vec<(WordId, f32)> {
        let mut best: HashMap<WordId, f32> = HashMap::new();
        for tc in spell_codes(typed) {
            let sk = skeleton(&tc);
            if sk.is_empty() {
                continue;
            }
            let mut keys = vec![hash(&sk)];
            // One-deletions only for skeletons long enough to stay specific
            // (a one-consonant skeleton matches thousands of words).
            if sk.len() >= 3 {
                for k in 0..sk.len() {
                    let mut d = sk.clone();
                    d.remove(k);
                    keys.push(hash(&d));
                }
            }
            for k in keys {
                let Some(ids) = self.index.get(&k) else { continue };
                for &id in ids {
                    if best.contains_key(&id) && best[&id] == 0.0 {
                        continue;
                    }
                    // Codes much longer or shorter can't be within `max`.
                    let d = self.codes[id as usize]
                        .iter()
                        .filter(|c| (c.len() as f32 - tc.len() as f32).abs() * 0.3 <= max)
                        .map(|c| distance(&tc, c))
                        .fold(f32::INFINITY, f32::min);
                    if d <= max {
                        let e = best.entry(id).or_insert(f32::INFINITY);
                        *e = e.min(d);
                    }
                }
            }
        }
        best.into_iter().collect()
    }

    /// Words pronounced exactly like `id` in some reading (sun/son, our/hour).
    pub fn homophones(&self, id: WordId) -> Vec<WordId> {
        let mut out = Vec::new();
        for c in &self.codes[id as usize] {
            if let Some(ids) = self.by_sound.get(&hash(c)) {
                out.extend(ids.iter().copied().filter(|&o| o != id));
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    pub fn codes(&self, id: WordId) -> &[Vec<u8>] {
        &self.codes[id as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(s: &str) -> String {
        String::from_utf8(spell_codes(s)[0].clone()).unwrap()
    }

    #[test]
    fn plain_spellings_read_as_they_sound() {
        assert_eq!(code("fone"), "f7n");
        assert_eq!(code("nite"), "nIt");
        assert_eq!(code("skool"), "sk8l");
        assert_eq!(code("enuf"), "4n9f");
    }

    #[test]
    fn close_sounds_are_close() {
        let fone = &spell_codes("fone")[0];
        let mut phone = Vec::new();
        for p in ["F", "OW", "N"] {
            cmu_phone(p, &mut phone);
        }
        assert_eq!(distance(fone, &phone), 0.0);
        assert!(distance(b"f7n", b"f5n") < distance(b"f7n", b"d7n"));
    }
}
