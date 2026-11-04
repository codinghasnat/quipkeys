//! The error ("noisy channel") model: how expensive (in nats, i.e. -ln p) it
//! is for a finger aiming at the intended word to produce the typed keys.
//!
//! The candidate search runs a weighted Damerau-Levenshtein alignment over
//! the dictionary trie with these costs, so geometry, double-letter slips and
//! roll-over transpositions are all part of the search itself rather than a
//! re-ranking afterwards.

use crate::keyboard::Keyboard;

#[derive(Clone, Debug)]
pub struct ChannelParams {
    /// Use key distances. When false, every substitution / insertion is
    /// equally likely (classic edit-distance spellchecker behaviour).
    pub geometry: bool,
    /// Use key timing to judge transpositions.
    pub timing: bool,
    pub sub_adjacent: f32,
    pub sub_per_key: f32,
    pub sub_max: f32,
    pub sub_flat: f32,
    pub ins_double: f32,
    pub ins_adjacent: f32,
    pub ins_other: f32,
    pub del_double: f32,
    pub del_apostrophe: f32,
    pub del_other: f32,
    pub transpose: f32,
    pub transpose_fast: f32,
    pub transpose_slow: f32,
    /// Inter-key interval (ms) below which a swapped pair looks like roll-over.
    pub fast_ms: f32,
    pub slow_ms: f32,
    /// Extra cost for an error in the first letter (people rarely get it wrong).
    pub first_letter: f32,
    /// Use typing rhythm: errors break rhythm (a pause before a wrong key, a
    /// pause after an extra key, a doubled gap where a letter went missing,
    /// a very fast overlapping pair for a swap). Needs key times and the
    /// typist's typical gap (set by the stream, see RHYTHM).
    pub rhythm: bool,
}

thread_local! {
    /// The typist's typical gap between keys (ms), kept current by the stream.
    pub static RHYTHM: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// Gap bins, as multiples of the typist's typical gap.
const RHYTHM_BINS: [f32; 4] = [0.5, 0.8, 1.3, 2.0];
/// Likelihood ratios P(gap bin | error) / P(gap bin | correct key), fitted on
/// 33 real runs (scripts/real/timing_lr.py). Rows: wrong key, extra key,
/// swapped pair, letter missing just before this key.
const LR_BEFORE: [[f32; 5]; 4] = [
    [1.75, 0.47, 0.53, 1.08, 3.66],
    [0.78, 0.44, 0.65, 1.27, 2.07],
    [3.15, 0.78, 0.87, 1.00, 2.05],
    [0.67, 0.46, 0.30, 1.96, 3.94],
];
const LR_AFTER: [[f32; 5]; 4] = [
    [1.12, 0.33, 0.75, 1.14, 7.21],
    [0.76, 0.62, 0.58, 1.14, 12.56],
    [3.08, 0.90, 0.73, 1.32, 1.67],
    [0.66, 0.65, 0.77, 1.14, 8.91],
];

fn rhythm_bin(r: f32) -> usize {
    RHYTHM_BINS.iter().position(|&b| r < b).unwrap_or(4)
}

impl Default for ChannelParams {
    fn default() -> Self {
        ChannelParams {
            geometry: true,
            timing: true,
            sub_adjacent: 5.5,
            sub_per_key: 3.0,
            sub_max: 13.0,
            sub_flat: 7.0,
            ins_double: 5.5,
            ins_adjacent: 6.0,
            ins_other: 11.0,
            del_double: 4.0,
            del_apostrophe: 2.5,
            del_other: 6.5,
            transpose: 5.0,
            transpose_fast: 3.0,
            transpose_slow: 6.5,
            fast_ms: 45.0,
            slow_ms: 120.0,
            first_letter: 1.5,
            rhythm: false,
        }
    }
}

/// Per-typed-word costs that only depend on the typed keys and their timing,
/// precomputed once before the search.
pub struct TypedCosts {
    pub keys: Vec<u8>,
    /// ins[j]: cost of typed key j being spurious.
    pub ins: Vec<f32>,
    /// trans[j]: cost of typed keys j-1, j being a swapped pair (j >= 1).
    pub trans: Vec<f32>,
    /// Rhythm adjustments (nats): wrong key at typed j; a letter missing just
    /// before typed key j (index n = at the end).
    pub sub_adj: Vec<f32>,
    pub del_adj: Vec<f32>,
}

pub struct Channel {
    pub kb: Keyboard,
    pub p: ChannelParams,
    sub_table: Box<[[f32; 128]; 128]>,
}

impl Channel {
    pub fn new(p: ChannelParams) -> Self {
        let kb = Keyboard::qwerty();
        let mut sub_table = Box::new([[p.sub_max; 128]; 128]);
        for a in 0..128u8 {
            for b in 0..128u8 {
                sub_table[a as usize][b as usize] = if a == b {
                    0.0
                } else if !p.geometry {
                    p.sub_flat
                } else {
                    let d = kb.distance(a, b);
                    (p.sub_adjacent + p.sub_per_key * (d - 1.0).max(0.0)).min(p.sub_max)
                };
            }
        }
        Channel { kb, p, sub_table }
    }

    /// Intended `want` but typed `got`.
    #[inline]
    pub fn sub(&self, want: u8, got: u8) -> f32 {
        self.sub_table[(want & 127) as usize][(got & 127) as usize]
    }

    /// Intended `want` was never typed. `prev` is the intended letter before it.
    #[inline]
    pub fn del(&self, want: u8, prev: u8) -> f32 {
        if want == prev {
            self.p.del_double
        } else if want == b'\'' {
            self.p.del_apostrophe
        } else {
            self.p.del_other
        }
    }

    /// `times[j]` is the key-down time (ms) of typed key j, if known.
    pub fn typed_costs(&self, keys: &[u8], times: Option<&[f32]>) -> TypedCosts {
        let n = keys.len();
        let mut ins = vec![0.0; n];
        let mut trans = vec![self.p.transpose; n];
        for j in 0..n {
            let c = keys[j];
            let prev = if j > 0 { Some(keys[j - 1]) } else { None };
            let next = keys.get(j + 1).copied();
            ins[j] = if prev == Some(c) {
                self.p.ins_double
            } else if !self.p.geometry {
                self.p.sub_flat
            } else if prev.is_some_and(|p| self.kb.adjacent(p, c))
                || next.is_some_and(|q| self.kb.adjacent(q, c))
            {
                self.p.ins_adjacent
            } else {
                self.p.ins_other
            };
            if j > 0 && self.p.timing {
                if let Some(t) = times {
                    let iki = t[j] - t[j - 1];
                    trans[j] = if iki < self.p.fast_ms {
                        self.p.transpose_fast
                    } else if iki > self.p.slow_ms {
                        self.p.transpose_slow
                    } else {
                        self.p.transpose
                    };
                }
            }
        }
        let mut sub_adj = vec![0.0; n];
        let mut del_adj = vec![0.0; n + 1];
        let med = RHYTHM.with(|r| r.get());
        if let (true, Some(t), true) = (self.p.rhythm, times, med > 0.0) {
            let adj = |lr: f32| (-lr.ln()).clamp(-2.5, 1.0);
            for j in 0..n {
                let bb = (j > 0).then(|| rhythm_bin((t[j] - t[j - 1]) / med));
                let ba = (j + 1 < n).then(|| rhythm_bin((t[j + 1] - t[j]) / med));
                let both = |row: usize| {
                    let a = bb.map_or(0.0, |b| adj(LR_BEFORE[row][b])) + ba.map_or(0.0, |b| adj(LR_AFTER[row][b]));
                    a.clamp(-3.0, 1.5)
                };
                sub_adj[j] = both(0);
                ins[j] = (ins[j] + both(1)).max(1.0);
                del_adj[j] = both(3);
                if let Some(b) = bb {
                    trans[j] = (self.p.transpose + adj(LR_BEFORE[2][b])).max(1.0);
                }
            }
        }
        TypedCosts { keys: keys.to_vec(), ins, trans, sub_adj, del_adj }
    }
}

impl Channel {
    /// Weighted alignment cost of `intended` against already-prepared typed
    /// costs: the same recurrence as the trie search, for one word.
    pub fn distance(&self, intended: &[u8], tc: &TypedCosts) -> f32 {
        let (t, n, m) = (&tc.keys, tc.keys.len(), intended.len());
        let fl = self.p.first_letter;
        let mut rows = vec![vec![0f32; n + 1]; m + 1];
        for j in 1..=n {
            rows[0][j] = rows[0][j - 1] + tc.ins[j - 1] + if j == 1 { fl } else { 0.0 };
        }
        for d in 1..=m {
            let c = intended[d - 1];
            let pc = if d >= 2 { intended[d - 2] } else { 0 };
            let first = if d == 1 { fl } else { 0.0 };
            rows[d][0] = rows[d - 1][0] + self.del(c, pc) + first + tc.del_adj[0];
            for j in 1..=n {
                let tj = t[j - 1];
                let mut v = rows[d - 1][j - 1] + if c == tj { 0.0 } else { (self.sub(c, tj) + tc.sub_adj[j - 1]).max(0.5) + first };
                v = v.min(rows[d - 1][j] + (self.del(c, pc) + tc.del_adj[j]).max(0.5) + first);
                v = v.min(rows[d][j - 1] + tc.ins[j - 1]);
                if d >= 2 && j >= 2 && c == t[j - 2] && pc == tj && c != pc {
                    v = v.min(rows[d - 2][j - 2] + tc.trans[j - 1] + if d == 2 { fl } else { 0.0 });
                }
                rows[d][j] = v;
            }
        }
        rows[m][n]
    }
}
