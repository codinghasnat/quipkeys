//! Learning one typist's habits, locally.
//!
//! A [`Tallies`] holds counts of what kinds of slips a typist makes: how often
//! a letter is dropped, doubled, added or swapped with its neighbour, and
//! which key was hit for which. It holds no words and no text. The app feeds
//! it pairs of (what was typed, what was meant) from its own corrections that
//! the user kept, and from correctly typed dictionary words; an undo takes
//! its pair back out.
//!
//! [`Tallies::apply`] turns the counts into channel costs, blended with the
//! built-in ones so little data means little change:
//!
//!   cost = -ln( (count + K * exp(-default cost)) / (opportunities + K) )
//!
//! Measured on 50 real runs of one typist, learning from the engine's own
//! fixes on one set and testing on the other: the six slip rates gave +1.4
//! and +0.6 points of word accuracy; the per-key confusions gave about
//! nothing at that amount of data. So the slip rates are applied, and the
//! key tallies are kept but not used yet (see `KEYS_MIN_LETTERS`).

use crate::channel::ChannelParams;
use crate::keyboard::Keyboard;

/// How much the average typist is trusted over the data (pseudo-count).
const K: f64 = 150.0;
/// Letters of evidence needed before the slip rates are applied (~400 words).
pub const MIN_LETTERS: u64 = 2000;
/// Letters needed before per-key confusions are applied: far more than any
/// data they have been tested on, so in practice not yet.
pub const KEYS_MIN_LETTERS: u64 = 500_000;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tallies {
    /// Intended letters seen (each one an opportunity for a slip).
    pub letters: u64,
    /// Intended letters that repeat the one before ("ll" in hello).
    pub doubles: u64,
    pub del_double: u64,
    pub del_other: u64,
    pub ins_double: u64,
    pub ins_adjacent: u64,
    pub ins_other: u64,
    pub transpose: u64,
    /// aimed[a]: times letter a was intended. sub[a][b]: aimed at a, hit b.
    pub aimed: [u64; 26],
    pub sub: [[u32; 26]; 26],
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    Match(u8),
    Sub(u8, u8),
    /// Intended letter left out (with its neighbours in the intended word).
    Del(u8, u8, u8),
    /// Extra typed key (with its neighbours in what was typed).
    Ins(u8, u8, u8),
    Swap(u8, u8),
}

/// Cheapest letter alignment of `typed` against `intended` (substitutions are
/// cheaper between adjacent keys; adjacent swaps allowed).
fn align(kb: &Keyboard, intended: &[u8], typed: &[u8]) -> Vec<Op> {
    let (n, m) = (intended.len(), typed.len());
    let sub = |a: u8, b: u8| if a == b { 0.0 } else if kb.adjacent(a, b) { 0.6 } else { 1.0 };
    let swap = |i: usize, j: usize| i >= 2 && j >= 2 && intended[i - 2] == typed[j - 1] && intended[i - 1] == typed[j - 2] && intended[i - 2] != intended[i - 1];
    let mut d = vec![vec![f32::INFINITY; m + 1]; n + 1];
    d[0][0] = 0.0;
    for i in 0..=n {
        for j in 0..=m {
            let mut v = d[i][j];
            if i > 0 && j > 0 {
                v = v.min(d[i - 1][j - 1] + sub(intended[i - 1], typed[j - 1]));
            }
            if i > 0 {
                v = v.min(d[i - 1][j] + 1.0);
            }
            if j > 0 {
                v = v.min(d[i][j - 1] + 1.0);
            }
            if swap(i, j) {
                v = v.min(d[i - 2][j - 2] + 0.8);
            }
            d[i][j] = v;
        }
    }
    let (mut i, mut j, mut ops) = (n, m, Vec::new());
    let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
    while i > 0 || j > 0 {
        let c = d[i][j];
        if swap(i, j) && near(d[i - 2][j - 2] + 0.8, c) {
            ops.push(Op::Swap(intended[i - 2], intended[i - 1]));
            i -= 2;
            j -= 2;
        } else if i > 0 && j > 0 && near(d[i - 1][j - 1] + sub(intended[i - 1], typed[j - 1]), c) {
            let (a, b) = (intended[i - 1], typed[j - 1]);
            ops.push(if a == b { Op::Match(a) } else { Op::Sub(a, b) });
            i -= 1;
            j -= 1;
        } else if i > 0 && near(d[i - 1][j] + 1.0, c) {
            ops.push(Op::Del(intended[i - 1], if i > 1 { intended[i - 2] } else { 0 }, if i < n { intended[i] } else { 0 }));
            i -= 1;
        } else {
            ops.push(Op::Ins(typed[j - 1], if j > 1 { typed[j - 2] } else { 0 }, if j < m { typed[j] } else { 0 }));
            j -= 1;
        }
    }
    ops.reverse();
    ops
}

fn ix(c: u8) -> Option<usize> {
    c.is_ascii_lowercase().then(|| (c - b'a') as usize)
}

impl Tallies {
    /// Count the slips in one word: `typed` is what the keys produced,
    /// `intended` what was meant (both are lower-cased here; words with
    /// anything but letters and apostrophes are skipped). `add` false takes
    /// a previously counted pair back out (an undo).
    pub fn observe(&mut self, kb: &Keyboard, typed: &str, intended: &str, add: bool) {
        let (t, w) = (typed.to_ascii_lowercase(), intended.to_ascii_lowercase());
        let ok = |s: &str| !s.is_empty() && s.len() <= 32 && s.bytes().all(|c| c.is_ascii_lowercase() || c == b'\'');
        if !ok(&t) || !ok(&w) {
            return;
        }
        let bump = |x: &mut u64| *x = if add { *x + 1 } else { x.saturating_sub(1) };
        let mut prev = 0u8;
        for op in align(kb, w.as_bytes(), t.as_bytes()) {
            let aim = |s: &mut Tallies, a: u8, prev: u8| {
                if let Some(i) = ix(a) {
                    bump(&mut s.aimed[i]);
                    bump(&mut s.letters);
                    if a == prev {
                        bump(&mut s.doubles);
                    }
                }
            };
            match op {
                Op::Match(a) => {
                    aim(self, a, prev);
                    prev = a;
                }
                Op::Sub(a, b) => {
                    aim(self, a, prev);
                    if let (Some(i), Some(j)) = (ix(a), ix(b)) {
                        let x = &mut self.sub[i][j];
                        *x = if add { x.saturating_add(1) } else { x.saturating_sub(1) };
                    }
                    prev = a;
                }
                Op::Del(a, before, after) => {
                    aim(self, a, prev);
                    bump(if a == before || a == after { &mut self.del_double } else { &mut self.del_other });
                    prev = a;
                }
                Op::Ins(g, before, after) => {
                    if ix(g).is_some() {
                        bump(if g == before || g == after {
                            &mut self.ins_double
                        } else if kb.adjacent(g, before) || kb.adjacent(g, after) {
                            &mut self.ins_adjacent
                        } else {
                            &mut self.ins_other
                        });
                    }
                }
                Op::Swap(a, b) => {
                    aim(self, a, prev);
                    aim(self, b, a);
                    bump(&mut self.transpose);
                    prev = b;
                }
            }
        }
    }

    /// Learn from the word a stream has just taken in (`edits` is what
    /// `Stream::push` returned). Two kinds of evidence are trusted: a known
    /// word typed and left unchanged (every letter was right), and a single
    /// fix of that word itself. Anything else (unknown words, splits, fixes
    /// reaching back to earlier words) teaches nothing. Returns the
    /// (typed, intended) pair when a fix was counted, so that an undo can
    /// take it back out with `observe(.., false)`.
    pub fn learn(&mut self, engine: &crate::Engine, stream: &crate::Stream, edits: &[crate::stream::Edit]) -> Option<(String, String)> {
        let last = stream.words.len().checked_sub(1)?;
        let word = |s: &str| s.trim_matches(|c: char| !c.is_ascii_alphabetic()).to_ascii_lowercase();
        let typed = word(&stream.words[last].typed);
        match edits {
            [] => {
                if engine.lx.id(&typed).is_some() {
                    self.observe(&engine.ch.kb, &typed, &typed, true);
                }
                None
            }
            [e] if e.index == last => {
                let meant = word(&e.new);
                if meant.contains(' ') || meant == typed {
                    return None;
                }
                self.observe(&engine.ch.kb, &typed, &meant, true);
                Some((typed, meant))
            }
            _ => None,
        }
    }

    /// Put what has been learned into the channel costs. Does nothing until
    /// there is enough evidence.
    pub fn apply(&self, p: &mut ChannelParams) {
        self.apply_with_min(p, MIN_LETTERS)
    }

    pub fn apply_with_min(&self, p: &mut ChannelParams, min_letters: u64) {
        if self.letters < min_letters {
            return;
        }
        let cost = |count: u64, opportunities: u64, c0: f32| -> f32 {
            let c = -(((count as f64) + K * (-(c0 as f64)).exp()) / (opportunities as f64 + K)).ln();
            (c as f32).clamp(1.5, 13.0)
        };
        let l = self.letters;
        p.ins_double = cost(self.ins_double, l, p.ins_double);
        p.ins_adjacent = cost(self.ins_adjacent, l, p.ins_adjacent);
        p.ins_other = cost(self.ins_other, l, p.ins_other);
        p.del_double = cost(self.del_double, self.doubles, p.del_double);
        p.del_other = cost(self.del_other, l, p.del_other);
        p.transpose = cost(self.transpose, l, p.transpose);
        if self.letters >= KEYS_MIN_LETTERS {
            let kb = Keyboard::qwerty();
            for a in 0..26u8 {
                for b in 0..26u8 {
                    if a == b {
                        continue;
                    }
                    let (ca, cb) = (b'a' + a, b'a' + b);
                    let d = kb.distance(ca, cb);
                    let c0 = (p.sub_adjacent + p.sub_per_key * (d - 1.0).max(0.0)).min(p.sub_max);
                    let c = cost(self.sub[a as usize][b as usize] as u64, self.aimed[a as usize], c0);
                    if (c - c0).abs() > 0.15 {
                        p.sub_overrides.push((ca, cb, c));
                    }
                }
            }
        }
    }

    /// The tallies as text (the profile file). Counts only.
    pub fn to_text(&self) -> String {
        let mut s = String::from(
            "# QuipKeys typing profile: tallies of the kinds of slips you make, so corrections fit\n\
             # your typing. Counts only: no words, no text. Delete this file to forget them.\n",
        );
        for (name, v) in [
            ("letters", self.letters), ("doubles", self.doubles), ("del_double", self.del_double), ("del_other", self.del_other),
            ("ins_double", self.ins_double), ("ins_adjacent", self.ins_adjacent), ("ins_other", self.ins_other), ("transpose", self.transpose),
        ] {
            s.push_str(&format!("n {name} {v}\n"));
        }
        for a in 0..26 {
            if self.aimed[a] > 0 {
                s.push_str(&format!("a {} {}\n", (b'a' + a as u8) as char, self.aimed[a]));
            }
            for b in 0..26 {
                if self.sub[a][b] > 0 {
                    s.push_str(&format!("k {} {} {}\n", (b'a' + a as u8) as char, (b'a' + b as u8) as char, self.sub[a][b]));
                }
            }
        }
        s
    }

    pub fn from_text(text: &str) -> Tallies {
        let mut t = Tallies::default();
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let letter = |s: &str| (s.len() == 1).then(|| ix(s.as_bytes()[0])).flatten();
            match f.as_slice() {
                ["n", name, v] => {
                    let Ok(v) = v.parse::<u64>() else { continue };
                    match *name {
                        "letters" => t.letters = v,
                        "doubles" => t.doubles = v,
                        "del_double" => t.del_double = v,
                        "del_other" => t.del_other = v,
                        "ins_double" => t.ins_double = v,
                        "ins_adjacent" => t.ins_adjacent = v,
                        "ins_other" => t.ins_other = v,
                        "transpose" => t.transpose = v,
                        _ => {}
                    }
                }
                ["a", a, v] => {
                    if let (Some(i), Ok(v)) = (letter(a), v.parse()) {
                        t.aimed[i] = v;
                    }
                }
                ["k", a, b, v] => {
                    if let (Some(i), Some(j), Ok(v)) = (letter(a), letter(b), v.parse()) {
                        t.sub[i][j] = v;
                    }
                }
                _ => {}
            }
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_each_kind_of_slip_and_undoes() {
        let kb = Keyboard::qwerty();
        let mut t = Tallies::default();
        t.observe(&kb, "wamt", "want", true); // n -> m
        t.observe(&kb, "teh", "the", true); // swap
        t.observe(&kb, "helo", "hello", true); // one of a double dropped
        t.observe(&kb, "frend", "friend", true); // a letter dropped
        t.observe(&kb, "thhe", "the", true); // doubled key
        t.observe(&kb, "thge", "the", true); // extra neighbouring key
        assert_eq!(t.sub[(b'n' - b'a') as usize][(b'm' - b'a') as usize], 1);
        assert_eq!((t.transpose, t.del_double, t.del_other, t.ins_double, t.ins_adjacent), (1, 1, 1, 1, 1));
        assert_eq!(t.letters, 4 + 3 + 5 + 6 + 3 + 3);
        let before = t.clone();
        t.observe(&kb, "wrok", "work", true);
        t.observe(&kb, "wrok", "work", false);
        assert_eq!(t, before);
        // Round trip through the file format.
        assert_eq!(Tallies::from_text(&t.to_text()), t);
        // Anything that isn't a plain word is ignored.
        t.observe(&kb, "p4ss", "pass", true);
        assert_eq!(t, before);
    }

    #[test]
    fn little_data_changes_nothing_and_habits_shift_costs() {
        let kb = Keyboard::qwerty();
        let mut t = Tallies::default();
        t.observe(&kb, "teh", "the", true);
        let mut p = ChannelParams::default();
        t.apply(&mut p);
        assert_eq!(p.transpose, ChannelParams::default().transpose);
        // A typist who swaps letters a lot, and never types a stray far key.
        for _ in 0..700 {
            t.observe(&kb, "the", "the", true);
        }
        for _ in 0..80 {
            t.observe(&kb, "teh", "the", true);
        }
        t.apply(&mut p);
        let d = ChannelParams::default();
        assert!(p.transpose < d.transpose - 1.0, "{}", p.transpose);
        assert!(p.ins_other > d.ins_other - 0.01);
        assert!(p.sub_overrides.is_empty());
    }
}
