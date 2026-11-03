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
        TypedCosts { keys: keys.to_vec(), ins, trans }
    }
}
