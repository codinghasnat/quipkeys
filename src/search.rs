//! Geometry-weighted dictionary search: a weighted Damerau-Levenshtein
//! alignment run directly over the trie, pruning any branch whose cheapest
//! partial alignment already exceeds the cost bound.

use crate::channel::{Channel, TypedCosts};
use crate::lexicon::{Lexicon, WordId, NO_WORD};

#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub word: WordId,
    /// Channel cost in nats: -ln P(typed | intended word), up to a constant.
    pub channel: f32,
}

const MAX_DEPTH: usize = 32;

pub fn search(lx: &Lexicon, ch: &Channel, tc: &TypedCosts, bound: f32) -> Vec<Candidate> {
    let n = tc.keys.len();
    let w = n + 1;
    if n == 0 || n + 3 >= MAX_DEPTH {
        return Vec::new();
    }
    let fl = ch.p.first_letter;
    let t = &tc.keys;
    // rows[d * w + j] = best cost aligning the first d intended letters
    // (the current trie path) with the first j typed keys.
    let mut rows = vec![0f32; MAX_DEPTH * w];
    let mut path = [0u8; MAX_DEPTH];
    for j in 1..=n {
        rows[j] = rows[j - 1] + tc.ins[j - 1] + if j == 1 { fl } else { 0.0 };
    }
    let mut out = Vec::new();
    // Explicit DFS stack of (node, depth, char).
    let mut stack: Vec<(u32, usize)> = Vec::with_capacity(256);
    for (c, k) in lx.trie.children(0) {
        path[1] = c;
        stack.push((k, 1 | ((c as usize) << 8)));
    }
    while let Some((node, packed)) = stack.pop() {
        let d = packed & 0xff;
        let c = (packed >> 8) as u8;
        path[d] = c;
        let pc = if d >= 2 { path[d - 1] } else { 0 };
        let (prev_rows, cur_rows) = rows.split_at_mut(d * w);
        let prev = &prev_rows[(d - 1) * w..d * w];
        let cur = &mut cur_rows[..w];
        let first = if d == 1 { fl } else { 0.0 };
        cur[0] = prev[0] + (ch.del(c, pc) + tc.del_adj[0]).max(0.5) + first;
        let mut best = cur[0];
        for j in 1..=n {
            let tj = t[j - 1];
            let mut v = prev[j - 1] + if c == tj { 0.0 } else { (ch.sub(c, tj) + tc.sub_adj[j - 1]).max(0.5) + first };
            v = v.min(prev[j] + (ch.del(c, pc) + tc.del_adj[j]).max(0.5) + first);
            v = v.min(cur[j - 1] + tc.ins[j - 1]);
            if d >= 2 && j >= 2 && c == t[j - 2] && pc == tj && c != pc {
                let pp = &prev_rows[(d - 2) * w..(d - 1) * w];
                v = v.min(pp[j - 2] + tc.trans[j - 1] + if d == 2 { fl } else { 0.0 });
            }
            cur[j] = v;
            best = best.min(v);
        }
        if best > bound {
            continue;
        }
        let wid = lx.trie.node_word[node as usize];
        if wid != NO_WORD && cur[n] <= bound {
            out.push(Candidate { word: wid, channel: cur[n] });
        }
        if d + 1 < MAX_DEPTH && d < n + 3 {
            for (c2, k2) in lx.trie.children(node) {
                stack.push((k2, (d + 1) | ((c2 as usize) << 8)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::ChannelParams;

    fn best(lx: &Lexicon, ch: &Channel, typed: &str) -> Vec<(String, f32)> {
        let tc = ch.typed_costs(typed.as_bytes(), None);
        let mut v: Vec<_> = search(lx, ch, &tc, 12.0)
            .into_iter()
            .map(|c| (lx.word(c.word).to_string(), c.channel))
            .collect();
        v.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        v
    }

    #[test]
    fn finds_geometric_neighbours() {
        let lx = Lexicon::english();
        let ch = Channel::new(ChannelParams::default());
        let r = best(&lx, &ch, "wamt");
        assert!(r.iter().any(|(w, _)| w == "want"), "{r:?}");
        let r = best(&lx, &ch, "thsi");
        assert!(r.iter().any(|(w, _)| w == "this"), "{r:?}");
        let r = best(&lx, &ch, "the");
        assert_eq!(r[0], ("the".to_string(), 0.0));
        // An adjacent-key slip is cheaper than a far one.
        let near = best(&lx, &ch, "stpre").into_iter().find(|x| x.0 == "store").unwrap().1;
        let far = best(&lx, &ch, "stqre").into_iter().find(|x| x.0 == "store");
        assert!(far.is_none() || far.unwrap().1 > near);
    }
}
