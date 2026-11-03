//! Whole-sentence decoding over a sliding window of typed chunks.
//!
//! Every chunk gets a set of *readings* (keep as typed, spelling candidates,
//! splits into several words, learned words) and adjacent chunk pairs get
//! boundary readings (merge, space moved one letter). A beam search finds the
//! best path through the window with three-word context along the path, so
//! neighbouring uncertain words are decided together instead of one by one.
//!
//! A change is only made if it clears the usual confidence gate. Confidence is
//! the posterior probability of the new text at that chunk: the forward sum
//! over every path producing it, divided by the sum over all paths. Every
//! changed chunk is then locked (hysteresis).

use std::collections::HashMap;

use super::{parse_token, recase_lx, Edit, Stream};
use crate::decoder::restore_apostrophe;
use crate::lexicon::WordId;

#[derive(Clone, Debug)]
struct Reading {
    start: usize,
    span: usize,
    /// Words emitted, for the language model (None = unknown word).
    words: Vec<Option<WordId>>,
    /// New lowercase core text per covered chunk ("" if merged away).
    outs: Vec<String>,
    /// Error-model / prior cost, without the language model.
    cost: f32,
    /// Reproduces what is currently shown (no change).
    keep: bool,
}

#[derive(Clone, Copy)]
struct Node {
    score: f32,
    p3: Option<WordId>,
    p2: Option<WordId>,
    p1: Option<WordId>,
    parent: usize,
    reading: usize,
}

type State = (Option<WordId>, Option<WordId>, Option<WordId>);
type Memo = HashMap<(Option<WordId>, Option<WordId>, Option<WordId>, WordId), f32>;

impl<'e> Stream<'e> {
    /// A chunk the lattice may change: a plain word, untouched so far.
    pub(super) fn lattice_open(&self, k: usize) -> bool {
        let w = &self.words[k];
        let core = &w.tok.core;
        !w.changed
            && !w.protected
            && !w.tok.name_like
            && !w.resegmented
            && !w.output.is_empty()
            && core.len() >= 1
            && core.bytes().all(|c| c.is_ascii_alphabetic() || c == b'\'')
            && (core.bytes().all(|c| c.is_ascii_lowercase() || c == b'\'')
                || (w.sentence_start && core.bytes().skip(1).all(|c| c.is_ascii_lowercase() || c == b'\'')))
            && core != "i"
            && restore_apostrophe(&core.to_ascii_lowercase()).is_none()
            && self.engine.lx.cap_share(&core.to_ascii_lowercase()) < super::CAP_FIX_SHARE
    }

    /// May take part in a boundary reading: plain letters, not yet re-spaced.
    /// A chunk already spell-fixed on its own still qualifies (the one allowed
    /// override): "th" -> "the" must be able to become "the friend" once
    /// "efriend" arrives.
    fn pair_eligible(&self, k: usize) -> bool {
        let w = &self.words[k];
        !w.protected
            && !w.tok.name_like
            && !w.resegmented
            && !w.output.is_empty()
            && w.tok.core.bytes().all(|c| c.is_ascii_alphabetic())
    }

    /// The chunk's current text without its punctuation, lowercased.
    fn shown_core(&self, k: usize) -> String {
        let w = &self.words[k];
        let o = w.output.strip_prefix(w.tok.lead.as_str()).unwrap_or(&w.output);
        let o = o.strip_suffix(w.tok.trail.as_str()).unwrap_or(o);
        o.to_ascii_lowercase()
    }

    fn fixed_reading(&self, k: usize) -> Reading {
        let w = &self.words[k];
        let words = w
            .output
            .split(' ')
            .filter(|p| !p.is_empty())
            .map(|p| self.engine.lx.id(&parse_token(p, true).core.to_ascii_lowercase()))
            .collect();
        Reading { start: k, span: 1, words, outs: vec![self.shown_core(k)], cost: 0.0, keep: true }
    }

    fn single_readings(&self, k: usize, out: &mut Vec<Reading>) {
        let e = self.engine;
        let (lx, cfg) = (&e.lx, &e.cfg);
        let w = &self.words[k];
        let core = w.tok.core.to_ascii_lowercase();
        let times = w.core_times();
        let typed_id = lx.id(&core);
        let morph = if typed_id.is_none() { e.morph_stem(&core) } else { None };
        // Keep as typed.
        let (kw, kc) = match (typed_id, morph) {
            (Some(t), _) => (Some(t), 0.0),
            (None, Some(m)) => (Some(m), cfg.morph_cost),
            (None, None) => (None, cfg.oov_keep_base + cfg.oov_keep_per_char * core.len().min(cfg.oov_len_cap) as f32),
        };
        out.push(Reading { start: k, span: 1, words: vec![kw], outs: vec![core.clone()], cost: kc, keep: true });
        if core.len() < cfg.min_len {
            return;
        }
        let extra = if typed_id.is_some() || morph.is_some() { cfg.real_word_extra } else { 0.0 };
        // Spelling candidates: the best few by error cost + word frequency.
        let cands = e.candidates(&core, times);
        let mut scored: Vec<(f32, WordId, f32)> = cands
            .iter()
            .filter(|c| Some(c.word) != typed_id)
            .map(|c| (c.channel + e.lm3(None, None, c.word), c.word, c.channel))
            .collect();
        scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for &(_, wid, ch) in scored.iter().take(cfg.lattice_cands) {
            out.push(Reading {
                start: k,
                span: 1,
                words: vec![Some(wid)],
                outs: vec![lx.word(wid).to_string()],
                cost: ch + extra + e.variety_cost(wid),
                keep: false,
            });
        }
        if typed_id.is_some() || morph.is_some() || !cfg.splits {
            return;
        }
        // Run-together words, exact pieces.
        if core.len() >= 3 {
            if let Some((ws, _)) = e.segment(&core, None, None) {
                let text = ws.iter().map(|&x| lx.word(x)).collect::<Vec<_>>().join(" ");
                out.push(Reading {
                    start: k,
                    span: 1,
                    words: ws.iter().map(|&x| Some(x)).collect(),
                    outs: vec![text],
                    cost: cfg.split_cost * (ws.len() - 1) as f32,
                    keep: false,
                });
            }
        }
        // Run-together with a slip in one piece.
        if core.len() >= 5 {
            for cut in 2..core.len() - 1 {
                let (x, y) = core.split_at(cut);
                let (ix, iy) = (lx.id(x), lx.id(y));
                if ix.is_some() == iy.is_some() {
                    continue;
                }
                let fuzzy = if ix.is_some() { y } else { x };
                if fuzzy.len() < 3 {
                    continue;
                }
                let mut best: Option<(f32, WordId, WordId)> = None;
                for c in e.candidates_within(fuzzy, None, cfg.bound).iter() {
                    let (a, b) = if let Some(a) = ix { (a, c.word) } else { (c.word, iy.unwrap()) };
                    let unseen = if lx.seen_bigram(a, b) { 0.0 } else { 3.0 };
                    let s = c.channel + unseen + e.lm3(None, None, c.word);
                    if best.is_none_or(|bb| s < bb.0) {
                        best = Some((s, a, b));
                    }
                }
                if let Some((s, a, b)) = best {
                    let fz = if ix.is_some() { b } else { a };
                    let ch = s - e.lm3(None, None, fz);
                    out.push(Reading {
                        start: k,
                        span: 1,
                        words: vec![Some(a), Some(b)],
                        outs: vec![format!("{} {}", lx.word(a), lx.word(b))],
                        cost: cfg.split_cost + ch,
                        keep: false,
                    });
                }
            }
        }
        // Words learned in this session.
        if !self.cache.learned.is_empty() {
            let tc = e.chan().typed_costs(core.as_bytes(), times);
            let total = self.cache.total.max(10) as f32;
            for (lw, n) in &self.cache.learned {
                if *lw == core || lw.len().abs_diff(core.len()) > 2 {
                    continue;
                }
                let d = e.chan().distance(lw.as_bytes(), &tc);
                if d <= cfg.bound {
                    let prior = -(cfg.learned_weight * *n as f32 / total).ln();
                    out.push(Reading { start: k, span: 1, words: vec![None], outs: vec![lw.clone()], cost: d + prior, keep: false });
                }
            }
        }
    }

    /// Merge / moved-space readings over chunks k, k+1 (only when one of them
    /// is not a word: clean text never has its spaces moved).
    fn pair_readings(&self, k: usize, out: &mut Vec<Reading>) {
        let e = self.engine;
        let (lx, cfg) = (&e.lx, &e.cfg);
        if !cfg.boundary {
            return;
        }
        let (pw, cw) = (&self.words[k], &self.words[k + 1]);
        if !pw.tok.trail.is_empty() || !cw.tok.lead.is_empty() {
            return;
        }
        let a = pw.tok.core.to_ascii_lowercase();
        let b = cw.tok.core.to_ascii_lowercase();
        if !a.bytes().chain(b.bytes()).all(|c| c.is_ascii_lowercase()) {
            return;
        }
        let (ka, kb) = (lx.id(&a).is_some(), lx.id(&b).is_some());
        if ka && kb {
            return;
        }
        // Gluing a real word onto its neighbour ("m kept" -> "kept",
        // "soy beans" -> "soybeans") needs much stronger evidence.
        let merge_margin = if ka || kb { cfg.merge_margin } else { 0.0 };
        let joined = format!("{a}{b}");
        let jt: Option<Vec<f32>> = match (pw.core_times(), cw.core_times()) {
            (Some(x), Some(y)) => Some(x.iter().chain(y).copied().collect()),
            _ => None,
        };
        let top = |s: &str, t: Option<&[f32]>, n: usize| -> Vec<(WordId, f32)> {
            let mut v: Vec<(f32, WordId, f32)> = e
                .candidates_within(s, t, cfg.bound)
                .iter()
                .map(|c| (c.channel + e.lm3(None, None, c.word), c.word, c.channel))
                .collect();
            if let Some(id) = lx.id(s) {
                v.push((e.lm3(None, None, id), id, 0.0));
            }
            v.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
            v.into_iter().take(n).map(|x| (x.1, x.2)).collect()
        };
        for (wid, ch) in top(&joined, jt.as_deref(), 3) {
            out.push(Reading {
                start: k,
                span: 2,
                words: vec![Some(wid)],
                outs: vec![lx.word(wid).to_string(), String::new()],
                cost: ch + cfg.space_cost + merge_margin + e.variety_cost(wid),
                keep: false,
            });
        }
        for d in [-1i32, 1] {
            let cut = a.len() as i32 + d;
            if cut < 1 || cut >= joined.len() as i32 {
                continue;
            }
            let cut = cut as usize;
            let (x, y) = joined.split_at(cut);
            let (tx, ty) = match &jt {
                Some(t) => (Some(&t[..cut]), Some(&t[cut..])),
                None => (None, None),
            };
            for (ix, cx) in top(x, tx, 2) {
                for (iy, cy) in top(y, ty, 2) {
                    let (sx, sy) = (lx.word(ix).to_string(), lx.word(iy).to_string());
                    let touches_clean = (ka && sx != a) || (kb && sy != b);
                    let margin = if touches_clean { cfg.shift_margin } else { 0.0 };
                    out.push(Reading {
                        start: k,
                        span: 2,
                        words: vec![Some(ix), Some(iy)],
                        outs: vec![sx, sy],
                        cost: cx + cy + cfg.space_cost + margin,
                        keep: false,
                    });
                }
            }
        }
    }

    /// Beam search over chunks `lo..hi` of the window. Returns the best score
    /// and the chosen reading indices (into `rs`).
    fn viterbi(&self, lo: usize, hi: usize, rs: &[Reading], by_start: &[Vec<usize>], ctx: State, memo: &mut Memo) -> Option<(f32, Vec<usize>)> {
        let e = self.engine;
        let m = hi - lo;
        let mut beams: Vec<Vec<Node>> = vec![Vec::new(); m + 1];
        beams[0].push(Node { score: 0.0, p3: ctx.0, p2: ctx.1, p1: ctx.2, parent: usize::MAX, reading: usize::MAX });
        let mut arena: Vec<Node> = Vec::new();
        for pos in 0..m {
            // Prune: best per LM state, then top-B.
            let mut cur = std::mem::take(&mut beams[pos]);
            cur.sort_by(|a, b| a.score.partial_cmp(&b.score).unwrap());
            let mut seen = std::collections::HashSet::new();
            cur.retain(|n| seen.insert((n.p3, n.p2, n.p1)));
            cur.truncate(e.cfg.lattice_beam);
            for node in cur {
                let id = arena.len();
                arena.push(node);
                for &ri in &by_start[pos] {
                    let r = &rs[ri];
                    if pos + r.span > m {
                        continue;
                    }
                    let (mut p3, mut p2, mut p1, mut s) = (node.p3, node.p2, node.p1, node.score + r.cost);
                    for &w in &r.words {
                        if let Some(wid) = w {
                            let c = *memo.entry((p3, p2, p1, wid)).or_insert_with(|| e.lm4(p3, p2, p1, wid));
                            s += e.cfg.lm_weight * c;
                        }
                        (p3, p2, p1) = (p2, p1, w);
                    }
                    if self.words[lo + pos + r.span - 1].tok.ends_sentence {
                        (p3, p2, p1) = (None, None, None);
                    }
                    beams[pos + r.span].push(Node { score: s, p3, p2, p1, parent: id, reading: ri });
                }
            }
        }
        let best = beams[m].iter().min_by(|a, b| a.score.partial_cmp(&b.score).unwrap())?;
        let mut path = vec![best.reading];
        let mut p = best.parent;
        while p != usize::MAX && arena[p].reading != usize::MAX {
            path.push(arena[p].reading);
            p = arena[p].parent;
        }
        path.reverse();
        Some((best.score, path))
    }

    /// Neural re-ranking: each open chunk's single readings get
    /// `nn_weight * (logsumexp - logit)` added, where logits come from the
    /// neural LM given everything shown before that chunk.
    fn nn_rerank(&mut self, nn: &crate::nnlm::NnLm, lo: usize, i: usize, rs: &mut [Reading]) {
        let vocab_words = nn.vocab - 2;
        let to_nn = |w: Option<WordId>| match w {
            Some(id) if (id as usize) < vocab_words => id as u16,
            _ => nn.unk(),
        };
        // Token history up to each chunk in the window.
        let mut toks = vec![nn.bos()];
        let mut before = vec![0usize; i + 1 - lo];
        for c in 0..=i {
            if c >= lo {
                before[c - lo] = toks.len();
            }
            if c == i {
                break;
            }
            let w = &self.words[c];
            for p in w.output.split(' ').filter(|p| !p.is_empty()) {
                toks.push(to_nn(self.engine.lx.id(&parse_token(p, true).core.to_ascii_lowercase())));
            }
            if w.tok.ends_sentence {
                toks.push(nn.bos());
            }
        }
        let alpha = self.engine.cfg.nn_weight;
        for k in lo..=i {
            let idx: Vec<usize> = (0..rs.len()).filter(|&r| rs[r].start == k && rs[r].span == 1).collect();
            if idx.len() < 2 {
                continue;
            }
            let mut st = std::mem::take(&mut self.nn_state);
            nn.sync(&mut st, &toks[..before[k - lo]]);
            let logits: Vec<f32> = idx.iter().map(|&r| nn.logit(&st, to_nn(rs[r].words.first().copied().flatten()))).collect();
            self.nn_state = st;
            let m = logits.iter().cloned().fold(f32::MIN, f32::max);
            let lse = m + logits.iter().map(|l| (l - m).exp()).sum::<f32>().ln();
            for (&r, l) in idx.iter().zip(&logits) {
                rs[r].cost += alpha * (lse - l);
            }
        }
    }

    /// Sum over all paths (forward algorithm, beam-pruned) whose text at the
    /// chunks in `req` equals the required text. Returns -ln(total probability).
    #[allow(clippy::too_many_arguments)]
    fn forward(&self, lo: usize, hi: usize, rs: &[Reading], by_start: &[Vec<usize>], ctx: State, req: &[(usize, &str)], memo: &mut Memo) -> f32 {
        let e = self.engine;
        let m = hi - lo;
        let lse = |a: f32, b: f32| {
            let (lo_, hi_) = if a < b { (a, b) } else { (b, a) };
            lo_ - (-(hi_ - lo_)).exp().ln_1p()
        };
        let mut beams: Vec<HashMap<State, f32>> = vec![HashMap::new(); m + 1];
        beams[0].insert(ctx, 0.0);
        for pos in 0..m {
            let mut cur: Vec<_> = beams[pos].drain().collect();
            cur.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            cur.truncate(e.cfg.lattice_beam * 2);
            for ((p3s, p2s, p1s), score) in cur {
                'reading: for &ri in &by_start[pos] {
                    let r = &rs[ri];
                    if pos + r.span > m {
                        continue;
                    }
                    for &(c, t) in req {
                        if r.start <= c && c < r.start + r.span && r.outs[c - r.start] != t {
                            continue 'reading;
                        }
                    }
                    let (mut p3, mut p2, mut p1, mut s) = (p3s, p2s, p1s, score + r.cost);
                    for &w in &r.words {
                        if let Some(wid) = w {
                            let c = *memo.entry((p3, p2, p1, wid)).or_insert_with(|| e.lm4(p3, p2, p1, wid));
                            s += e.cfg.lm_weight * c;
                        }
                        (p3, p2, p1) = (p2, p1, w);
                    }
                    if self.words[lo + pos + r.span - 1].tok.ends_sentence {
                        (p3, p2, p1) = (None, None, None);
                    }
                    let slot = beams[pos + r.span].entry((p3, p2, p1)).or_insert(f32::INFINITY);
                    *slot = if slot.is_finite() { lse(*slot, s) } else { s };
                }
            }
        }
        beams[m].values().fold(f32::INFINITY, |acc, &v| if acc.is_finite() { lse(acc, v) } else { v })
    }

    /// Decode the window ending at chunk `i` and apply confident changes.
    pub(super) fn lattice_step(&mut self, i: usize) -> Vec<Edit> {
        let e = self.engine;
        let lo = i.saturating_sub(e.cfg.lattice_window - 1);
        // Don't let the window straddle a sentence start: context resets there.
        let lo = (lo..=i).rev().find(|&k| k == lo || self.words[k - 1].tok.ends_sentence).unwrap_or(lo);
        if !(lo..=i).any(|k| self.lattice_open(k) || (k < i && self.pair_eligible(k) && self.pair_eligible(k + 1))) {
            return Vec::new();
        }
        let mut rs: Vec<Reading> = Vec::new();
        for k in lo..=i {
            let w = &self.words[k];
            // A chunk already spell-fixed keeps its honest error costs (so a
            // later boundary reading is compared fairly), but its readings are
            // never applied on their own: it is locked.
            let relock = w.changed && self.pair_eligible(k) && w.tok.core.bytes().all(|c| c.is_ascii_alphabetic());
            if self.lattice_open(k) || relock {
                let from = rs.len();
                self.single_readings(k, &mut rs);
                if relock {
                    let shown = self.shown_core(k);
                    let mut any = false;
                    for r in &mut rs[from..] {
                        r.keep = r.outs[0] == shown;
                        any |= r.keep;
                    }
                    if !any {
                        rs.truncate(from);
                        rs.push(self.fixed_reading(k));
                    }
                }
            } else {
                rs.push(self.fixed_reading(k));
            }
            if k < i && self.pair_eligible(k) && self.pair_eligible(k + 1) {
                self.pair_readings(k, &mut rs);
            }
        }
        if let Some(nn) = &e.nn {
            if e.cfg.nn_weight > 0.0 {
                self.nn_rerank(nn, lo, i, &mut rs);
            }
        }
        let m = i + 1 - lo;
        let mut by_start: Vec<Vec<usize>> = vec![Vec::new(); m];
        for (ri, r) in rs.iter().enumerate() {
            by_start[r.start - lo].push(ri);
        }
        let ctx = (self.ctx_prev3(lo), self.ctx_prev2(lo), self.ctx_prev(lo));
        let mut memo = HashMap::new();
        let Some((_, path)) = self.viterbi(lo, i + 1, &rs, &by_start, ctx, &mut memo) else {
            return Vec::new();
        };
        let total = self.forward(lo, i + 1, &rs, &by_start, ctx, &[], &mut memo);
        let mut edits = Vec::new();
        for &ri in &path {
            let r = &rs[ri];
            if r.keep {
                continue;
            }
            let k = r.start;
            if r.span == 1 && self.words[k].changed {
                continue; // locked: only a boundary reading may still change it
            }
            // Confidence: probability mass of all paths giving this text here.
            let req: Vec<(usize, &str)> = r.outs.iter().enumerate().map(|(j, t)| (k + j, t.as_str())).collect();
            let conf = (total - self.forward(lo, i + 1, &rs, &by_start, ctx, &req, &mut memo)).exp();
            let core = self.words[k].tok.core.to_ascii_lowercase();
            let typed_known = e.lx.id(&core).is_some();
            let morph = !typed_known && e.morph_stem(&core).is_some();
            let known = typed_known || morph;
            let tau = if r.span == 2 {
                e.cfg.tau_reseg
            } else if known {
                if k == i { e.cfg.tau_realword } else { e.cfg.tau_revise }
            } else {
                e.cfg.tau_nonword
            };
            // Features for the learned gate (and for training it).
            let keep_cost = rs.iter().find(|x| x.start == k && x.span == 1 && x.keep).map_or(0.0, |x| x.cost);
            let n_here = rs.iter().filter(|x| x.start == k && x.span == 1).count();
            let cand_freq = r.words.first().copied().flatten().map_or(25.0, |w| e.lm3(None, None, w));
            let feats = crate::decoder::gate_features(&crate::decoder::GateInput {
                conf,
                known: typed_known,
                morph,
                span2: r.span == 2,
                newest: k == i,
                back: i - k,
                cost: r.cost,
                keep_cost: if known { 0.0 } else { keep_cost },
                cand_freq,
                typed_len: core.len(),
                split: r.span == 1 && r.words.len() > 1,
                learned: r.words.len() == 1 && r.words[0].is_none(),
                n_readings: n_here,
                capital: self.words[k].sentence_start,
            });
            // The learned gate decides non-word fixes. Changing a real word is
            // the riskiest move there is, so it keeps the strict fixed bar.
            let apply = match &e.cfg.gate {
                Some(g) if !known && r.span == 1 => g.prob(&feats) >= g.threshold,
                _ => conf >= tau,
            };
            if let Some(log) = &mut self.log {
                log.push(super::GateExample { chunk: k, span: r.span, outs: r.outs.clone(), feats, applied: apply });
            }
            if !apply {
                continue;
            }
            for (j, out) in r.outs.iter().enumerate() {
                let c = k + j;
                let w = &self.words[c];
                let new = if out.is_empty() {
                    String::new()
                } else if r.span == 2 {
                    let pw = &self.words[k];
                    let cw = &self.words[k + 1];
                    if r.outs[1].is_empty() {
                        format!("{}{}{}", pw.tok.lead, recase_lx(&e.lx, out, pw.capital()), cw.tok.trail)
                    } else if j == 0 {
                        format!("{}{}", pw.tok.lead, recase_lx(&e.lx, out, pw.capital()))
                    } else {
                        format!("{}{}", recase_lx(&e.lx, out, false), cw.tok.trail)
                    }
                } else {
                    format!("{}{}{}", w.tok.lead, recase_lx(&e.lx, out, w.capital()), w.tok.trail)
                };
                if r.span == 2 {
                    self.words[c].resegmented = true;
                }
                if new != self.words[c].output {
                    let old = self.set_output(c, new.clone());
                    edits.push(Edit { index: c, old, new, confidence: conf, revision: c != i });
                } else {
                    self.words[c].changed = true;
                }
            }
        }
        edits
    }
}
