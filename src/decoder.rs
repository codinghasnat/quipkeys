//! The intent decoder: candidate generation + scoring + confidence gate for a
//! single word, given whatever context is available.

use crate::channel::{Channel, ChannelParams};
use crate::lexicon::{Lexicon, LmParams, WordId};
use crate::search::{search, Candidate};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    static CACHE: RefCell<HashMap<String, Rc<Vec<Candidate>>>> = RefCell::new(HashMap::new());
}

#[derive(Clone, Debug)]
pub struct Config {
    pub channel: ChannelParams,
    /// Use the previous word (bigram) context.
    pub context: bool,
    /// Allow revising the previous word once the next word is known.
    pub right_context: bool,
    /// Decode a sliding window of chunks jointly (whole-sentence decoder)
    /// instead of word-by-word decisions with look-backs.
    pub lattice: bool,
    pub lattice_window: usize,
    pub lattice_beam: usize,
    pub lattice_cands: usize,
    /// Second look-back with two words of right context.
    pub lookback2: bool,
    /// Repair misplaced spaces between consecutive chunks.
    pub boundary: bool,
    /// Try splitting a run-together word ("thecat" -> "the cat").
    pub splits: bool,
    /// Bigram interpolation weight.
    pub lambda: f64,
    /// Use corpus trigrams (three-word context).
    pub trigrams: bool,
    /// Max trigram weight, and its confidence constant.
    pub lambda3: f64,
    pub k3: f64,
    /// Weight of the language model relative to the error model.
    pub lm_weight: f32,
    /// Largest channel cost considered (nats). Roughly "two slips".
    pub bound: f32,
    /// Extra search radius per letter beyond 5 (long words absorb more slips).
    pub bound_per_char: f32,
    /// Hard cap on the search radius (keeps the worst case fast).
    pub bound_max: f32,
    /// Confidence needed to correct a capitalised word mid-sentence (could be a name).
    pub tau_capital: f32,
    /// A capitalised word may be fixed to a known proper noun with the usual
    /// bar only if the slip costs less than this (one clean keyboard slip).
    pub proper_fix_max_channel: f32,
    /// Weight of the document cache in the context model (0 = off).
    pub doc_cache: f32,
    /// Prior weight of session-learned words.
    pub learned_weight: f32,
    /// Score of "this unknown word is exactly what the user meant".
    pub oov_keep_base: f32,
    pub oov_keep_per_char: f32,
    /// Letters beyond this don't make an unknown word less plausible
    /// (long unknown words are usually real technical terms).
    pub oov_len_cap: usize,
    /// Cost of an inflected form of a known word being exactly what was meant.
    pub morph_cost: f32,
    /// Extra cost for replacing a word that is already in the dictionary.
    pub real_word_extra: f32,
    /// Posterior probability the best candidate needs before we act.
    pub tau_nonword: f32,
    pub tau_realword: f32,
    pub tau_revise: f32,
    pub split_cost: f32,
    /// Cost of a misplaced space (pressed too early / late / extra).
    pub space_cost: f32,
    /// Extra evidence (nats) needed to merge two chunks that already read as real words.
    pub merge_margin: f32,
    /// Extra evidence needed to move a space through a correctly typed word.
    pub shift_margin: f32,
    /// Confidence needed to move word boundaries.
    pub tau_reseg: f32,
    /// A hand-shifted word must be at least this common (LM cost, nats).
    pub hand_shift_max_cost: f32,
    /// Shortest typed word we will touch.
    pub min_len: usize,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            channel: ChannelParams::default(),
            context: true,
            right_context: true,
            splits: true,
            boundary: true,
            lookback2: true,
            lattice: true,
            lattice_window: 4,
            lattice_beam: 8,
            lattice_cands: 8,
            lambda: 0.6,
            trigrams: true,
            lambda3: 0.7,
            k3: 5.0,
            lm_weight: 1.0,
            bound: 13.0,
            bound_per_char: 2.5,
            bound_max: 19.0,
            tau_capital: 0.999,
            proper_fix_max_channel: 6.0,
            doc_cache: 0.0,
            learned_weight: 0.2,
            oov_keep_base: 12.0,
            oov_keep_per_char: 2.0,
            oov_len_cap: 10,
            morph_cost: 3.0,
            real_word_extra: 1.5,
            tau_nonword: 0.9,
            tau_realword: 0.98,
            tau_revise: 0.98,
            split_cost: 6.0,
            space_cost: 5.0,
            tau_reseg: 0.9,
            merge_margin: 6.0,
            shift_margin: 2.0,
            hand_shift_max_cost: 14.0,
            min_len: 1,
        }
    }
}

/// Context for one decision. `prev`/`next` are dictionary ids of the
/// neighbouring words (already corrected), when known and in the dictionary.
/// What has been typed so far in this document/session.
#[derive(Clone, Default, Debug)]
pub struct DocCache {
    /// Output word counts (dictionary words).
    pub counts: HashMap<WordId, u32>,
    pub total: u32,
    /// Words the user evidently means even though the dictionary lacks them
    /// (typed identically twice, or restored with undo): (lowercase, count).
    pub learned: Vec<(String, u32)>,
}

impl DocCache {
    pub fn add(&mut self, id: WordId, delta: i32) {
        let c = self.counts.entry(id).or_insert(0);
        *c = (*c as i32 + delta).max(0) as u32;
        self.total = (self.total as i32 + delta).max(0) as u32;
    }
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Ctx<'a> {
    /// Document memory, if any.
    pub cache: Option<&'a DocCache>,
    /// Two words back (for trigram context).
    pub prev2: Option<WordId>,
    pub prev: Option<WordId>,
    /// Two words ahead (only known when revising).
    pub next2: Option<WordId>,
    pub next: Option<WordId>,
    /// There is a following word, but it is not in the dictionary.
    pub next_unknown: bool,
    /// Capitalised mid-sentence (likely a name): keeping it unknown is cheap.
    pub name_like: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    /// Lowercase replacement (may contain a space for splits).
    pub text: String,
    /// Posterior probability of `text` among all hypotheses (incl. "keep").
    pub confidence: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Hyp {
    Keep,
    Word(WordId),
    /// Run-together words: "nicetomeetyou" -> nice to meet you.
    Seg(Vec<WordId>),
    /// A word learned in this session (not in the dictionary).
    Learned(String),
}

pub struct Engine {
    pub lx: Lexicon,
    pub ch: Channel,
    pub cfg: Config,
}

impl Engine {
    pub fn new(cfg: Config) -> Self {
        Self::with_lexicon(Lexicon::english(), cfg)
    }

    pub fn with_lexicon(lx: Lexicon, cfg: Config) -> Self {
        Engine { ch: Channel::new(cfg.channel.clone()), lx, cfg }
    }

    fn lm_params(&self) -> LmParams {
        LmParams { lambda: self.cfg.lambda, lambda3: self.cfg.lambda3, k3: self.cfg.k3, trigrams: self.cfg.trigrams }
    }

    /// Context cost of `w` after `prev2 prev`.
    pub fn lm3(&self, prev2: Option<WordId>, prev: Option<WordId>, w: WordId) -> f32 {
        if !self.cfg.context {
            return self.lx.cost3(None, None, w, &self.lm_params()) as f32;
        }
        let prev2 = if prev.is_some() { prev2 } else { None };
        self.lx.cost3(prev2, prev, w, &self.lm_params()) as f32
    }

    /// Left-context cost of `w`, mixed with how often it already appeared in
    /// this document (a cache language model).
    fn lm_doc(&self, ctx: &Ctx, w: WordId) -> f32 {
        let base = self.lm3(ctx.prev2, ctx.prev, w);
        match ctx.cache {
            Some(c) if self.cfg.doc_cache > 0.0 && c.total >= 10 => {
                let n = c.counts.get(&w).copied().unwrap_or(0);
                if n == 0 {
                    return base - (1.0 - self.cfg.doc_cache).ln();
                }
                let pc = n as f32 / c.total as f32;
                let p = (1.0 - self.cfg.doc_cache) * (-base).exp() + self.cfg.doc_cache * pc;
                -p.ln()
            }
            _ => base,
        }
    }

    fn lm(&self, prev: Option<WordId>, w: WordId) -> f32 {
        self.lm3(None, prev, w)
    }

    /// Cost of the following word given hypothesis `w` (right context).
    /// Cost of the following word(s) given hypothesis `w` (right context).
    fn right(&self, w: Option<WordId>, ctx: &Ctx) -> f32 {
        match ctx.next {
            Some(n) if self.cfg.context => {
                let mut c = self.lm3(ctx.prev, w, n);
                if let Some(n2) = ctx.next2 {
                    c += self.lm3(w, Some(n), n2);
                }
                c
            }
            _ => 0.0,
        }
    }

    /// Dictionary candidates for `core`, memoised: while typing, each chunk is
    /// searched again at the next space (boundary repair, look-back), so a
    /// small per-thread cache saves most repeat work.
    pub fn candidates(&self, core: &str, times: Option<&[f32]>) -> Rc<Vec<Candidate>> {
        let bound = (self.cfg.bound + self.cfg.bound_per_char * core.len().saturating_sub(5) as f32).min(self.cfg.bound_max);
        self.candidates_within(core, times, bound)
    }

    pub fn candidates_within(&self, core: &str, times: Option<&[f32]>, bound: f32) -> Rc<Vec<Candidate>> {
        let tc = self.ch.typed_costs(core.as_bytes(), times);
        // Timing only matters through the transposition costs, so key on those.
        let mut key = core.to_string();
        key.push_str(&format!("|{bound}|"));
        for t in &tc.trans {
            key.push(char::from(b'0' + (*t as u8).min(9)));
        }
        if let Some(c) = CACHE.with(|m| m.borrow().get(&key).cloned()) {
            return c;
        }
        let c = Rc::new(search(&self.lx, &self.ch, &tc, bound));
        CACHE.with(|m| {
            let mut m = m.borrow_mut();
            if m.len() > 256 {
                m.clear();
            }
            m.insert(key, c.clone());
        });
        c
    }

    /// Score every hypothesis for `core`. Index 0 is always "keep as typed".
    /// Lower score = more likely (nats).
    pub fn hypotheses(&self, core: &str, times: Option<&[f32]>, ctx: &Ctx) -> Vec<(Hyp, f32)> {
        self.hypotheses_within(core, times, ctx, None)
    }

    pub fn hypotheses_within(&self, core: &str, times: Option<&[f32]>, ctx: &Ctx, bound: Option<f32>) -> Vec<(Hyp, f32)> {
        let typed_id = self.lx.id(core);
        let morph = if typed_id.is_none() { self.morph_stem(core) } else { None };
        let cands = match bound {
            Some(b) => self.candidates_within(core, times, b),
            None => self.candidates(core, times),
        };
        let w = self.cfg.lm_weight;
        let mut hyps = Vec::with_capacity(cands.len() + 4);
        let keep = match typed_id.or(morph) {
            // An inflection of a known word ("freewheelers") is scored like
            // the stem plus a small cost: it is almost certainly a real word.
            Some(t) if typed_id.is_none() => self.cfg.morph_cost + w * (self.lm_doc(ctx, t) + self.right(Some(t), ctx)),
            Some(t) => w * (self.lm_doc(ctx, t) + self.right(Some(t), ctx)),
            None => {
                let len = if ctx.name_like { 0 } else { core.len().min(self.cfg.oov_len_cap) };
                self.cfg.oov_keep_base + self.cfg.oov_keep_per_char * len as f32 + w * self.right(None, ctx)
            }
        };
        hyps.push((Hyp::Keep, keep));
        for c in cands.iter() {
            if Some(c.word) == typed_id {
                continue;
            }
            let extra = if typed_id.is_some() { self.cfg.real_word_extra } else { 0.0 };
            let s = c.channel + extra + w * (self.lm_doc(ctx, c.word) + self.right(Some(c.word), ctx));
            hyps.push((Hyp::Word(c.word), s));
        }
        // Words learned in this session compete like dictionary words, with a
        // prior from how often they've been used here.
        if let Some(cache) = ctx.cache {
            if !cache.learned.is_empty() && typed_id.is_none() {
                let tc = self.ch.typed_costs(core.as_bytes(), times);
                let total = cache.total.max(10) as f32;
                for (lw, n) in &cache.learned {
                    if lw == core || lw.len().abs_diff(core.len()) > 2 {
                        continue;
                    }
                    let d = self.ch.distance(lw.as_bytes(), &tc);
                    if d <= self.cfg.bound {
                        let prior = -(self.cfg.learned_weight * *n as f32 / total).ln();
                        hyps.push((Hyp::Learned(lw.clone()), d + w * (prior + self.right(None, ctx))));
                    }
                }
            }
        }
        if self.cfg.splits && typed_id.is_none() && core.len() >= 3 {
            if let Some((ws, lm)) = self.segment(core, ctx.prev2, ctx.prev) {
                let s = self.cfg.split_cost * (ws.len() - 1) as f32
                    + w * (lm + self.right(ws.last().copied(), ctx));
                hyps.push((Hyp::Seg(ws), s));
            }
            if core.len() >= 5 && bound.is_none() {
                self.fuzzy_splits(core, ctx, &mut hyps);
            }
        }
        hyps
    }

    /// If `core` is a known word plus a regular prefix/suffix
    /// ("banquetings", "overnighters", "unfriendly"), return the known word.
    pub fn morph_stem(&self, core: &str) -> Option<WordId> {
        if core.len() < 6 {
            return None;
        }
        let known = |w: &str| if w.len() >= 3 { self.lx.id(w) } else { None };
        const SUFFIXES: &[(&str, &[&str])] = &[
            ("ings", &["", "e"]), ("ers", &["", "e"]), ("ships", &[""]), ("ship", &[""]), ("ments", &[""]),
            ("ment", &[""]), ("ness", &[""]), ("less", &[""]), ("ful", &[""]), ("ably", &["able"]),
            ("ally", &["", "al"]), ("ily", &["y"]), ("ly", &["", "le"]), ("ies", &["y"]), ("ied", &["y"]),
            ("ing", &["", "e"]), ("ed", &["", "e"]), ("er", &["", "e"]), ("es", &[""]), ("s", &[""]),
            ("ism", &[""]), ("ist", &[""]), ("ists", &[""]), ("ization", &["ize"]), ("isation", &["ise"]),
            ("izer", &["ize"]), ("izers", &["ize"]), ("iser", &["ise"]), ("isers", &["ise"]),
        ];
        for (suf, adds) in SUFFIXES {
            if let Some(stem) = core.strip_suffix(suf) {
                for add in *adds {
                    let cand = format!("{stem}{add}");
                    if let Some(id) = known(&cand) {
                        return Some(id);
                    }
                }
                // Doubled final consonant: "stopped" -> stop.
                let b = stem.as_bytes();
                if b.len() >= 4 && b[b.len() - 1] == b[b.len() - 2] && (*suf == "ed" || *suf == "ing" || *suf == "er") {
                    if let Some(id) = known(&stem[..stem.len() - 1]) {
                        return Some(id);
                    }
                }
            }
        }
        const PREFIXES: &[&str] =
            &["over", "under", "inter", "super", "counter", "non", "un", "re", "pre", "mis", "out", "sub", "anti", "semi", "multi"];
        for pre in PREFIXES {
            if let Some(rest) = core.strip_prefix(pre) {
                if rest.len() >= 4 {
                    if let Some(id) = known(rest).or_else(|| self.morph_stem(rest)) {
                        return Some(id);
                    }
                }
            }
        }
        None
    }

    /// Run-together words with a slip in one of them ("andfharts" -> and charts):
    /// cut where one side is an exact word and spell-correct the other side.
    fn fuzzy_splits(&self, core: &str, ctx: &Ctx, hyps: &mut Vec<(Hyp, f32)>) {
        let w = self.cfg.lm_weight;
        let ok_piece = |p: &str| p.len() >= 2 || p == "a" || p == "i";
        for k in 1..core.len() {
            let (x, y) = core.split_at(k);
            if !ok_piece(x) || !ok_piece(y) {
                continue;
            }
            let (ix, iy) = (self.lx.id(x), self.lx.id(y));
            if ix.is_some() == iy.is_some() {
                continue; // both exact: plain segmentation covers it; neither: too costly
            }
            let fuzzy = if ix.is_some() { y } else { x };
            if fuzzy.len() < 3 {
                continue;
            }
            for c in self.candidates_within(fuzzy, None, self.cfg.bound).iter() {
                let (a, b) = if let Some(a) = ix { (a, c.word) } else { (c.word, iy.unwrap()) };
                let unseen = if self.lx.seen_bigram(a, b) { 0.0 } else { 3.0 };
                let s = self.cfg.split_cost
                    + unseen
                    + c.channel
                    + w * (self.lm3(ctx.prev2, ctx.prev, a) + self.lm3(ctx.prev, Some(a), b) + self.right(Some(b), ctx));
                hyps.push((Hyp::Seg(vec![a, b]), s));
            }
        }
    }

    /// Best way to cut `core` into 2+ dictionary words (exact pieces),
    /// returning the words and their summed language-model cost.
    pub fn segment(&self, core: &str, prev2: Option<WordId>, prev: Option<WordId>) -> Option<(Vec<WordId>, f32)> {
        let n = core.len();
        let mut best: Vec<Option<(f32, Vec<WordId>)>> = vec![None; n + 1];
        best[0] = Some((0.0, Vec::new()));
        for j in 1..=n {
            let mut cur: Option<(f32, Vec<WordId>)> = None;
            for i in j.saturating_sub(20)..j {
                let piece = &core[i..j];
                if piece.len() == 1 && piece != "a" && piece != "i" {
                    continue;
                }
                let (Some(id), Some((s0, ws))) = (self.lx.id(piece), best[i].as_ref()) else { continue };
                let (q2, q1) = match ws.len() {
                    0 => (prev2, prev),
                    1 => (prev, Some(ws[0])),
                    k => (Some(ws[k - 2]), Some(ws[k - 1])),
                };
                let s = s0 + self.lm3(q2, q1, id) * self.cfg.lm_weight + if i > 0 { self.cfg.split_cost } else { 0.0 };
                if cur.as_ref().is_none_or(|c| s < c.0) {
                    let mut v = ws.clone();
                    v.push(id);
                    cur = Some((s, v));
                }
            }
            best[j] = cur;
        }
        let (s, ws) = best[n].take()?;
        if ws.len() < 2 {
            return None;
        }
        // Real run-together typing joins words people write next to each other
        // ("nice to meet you"); an unknown compound ("autofluorescence") rarely
        // splits into a frequent pair. Require every junction to be one.
        if !ws.windows(2).all(|p| self.lx.has_bigram(p[0], p[1])) {
            return None;
        }
        // Return the pure LM part; the caller adds split costs.
        let lm = (s - self.cfg.split_cost * (ws.len() - 1) as f32) / self.cfg.lm_weight.max(1e-6);
        Some((ws, lm))
    }

    pub fn hyp_text(&self, h: &Hyp, core: &str) -> String {
        match h {
            Hyp::Keep => core.to_string(),
            Hyp::Word(id) => self.lx.word(*id).to_string(),
            Hyp::Learned(w) => w.clone(),
            Hyp::Seg(ws) => ws.iter().map(|&w| self.lx.word(w)).collect::<Vec<_>>().join(" "),
        }
    }

    /// The single most likely reading of `core` and its score, with no
    /// confidence gate. Used to compare alternative word boundaries.
    pub fn best(&self, core: &str, times: Option<&[f32]>, ctx: &Ctx) -> (String, f32) {
        // Boundary alternatives use the base radius: keeps repair cheap.
        let hyps = self.hypotheses_within(core, times, ctx, Some(self.cfg.bound));
        let (h, s) = hyps.into_iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap();
        (self.hyp_text(&h, core), s)
    }

    /// Whole hand shifted one key left or right ("jr;;p" for "hello").
    /// Returns the intended word if the shifted reading is a common word and
    /// the typed text is not a word at all.
    pub fn hand_shift(&self, typed: &str) -> Option<String> {
        const ROWS: [&[u8]; 3] = [b"qwertyuiop[]", b"asdfghjkl;'", b"zxcvbnm,./"];
        let t = typed.to_ascii_lowercase();
        if t.len() < 3 || self.lx.id(&t).is_some() {
            return None;
        }
        let mut best: Option<(f32, String)> = None;
        for dir in [-1i32, 1] {
            let mut out = String::with_capacity(t.len());
            for c in t.bytes() {
                let mapped = ROWS.iter().find_map(|r| {
                    let i = r.iter().position(|&k| k == c)? as i32 + dir;
                    r.get(usize::try_from(i).ok()?).copied()
                });
                match mapped {
                    Some(m) if m.is_ascii_lowercase() || m == b'\'' => out.push(m as char),
                    _ => {
                        out.clear();
                        break;
                    }
                }
            }
            if let Some(id) = self.lx.id(&out) {
                let c = self.lm(None, id);
                if c < self.cfg.hand_shift_max_cost && best.as_ref().is_none_or(|b| c < b.0) {
                    best = Some((c, out));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Decide what the user meant by `core` (lowercase letters/apostrophes).
    /// Returns `Some` only when a *different* word wins with enough confidence.
    /// `tau` overrides the confidence threshold (used for revisions).
    pub fn decide(&self, core: &str, times: Option<&[f32]>, ctx: &Ctx, tau: Option<f32>) -> Option<Suggestion> {
        if core.len() < self.cfg.min_len {
            return None;
        }
        if let Some(full) = restore_apostrophe(core) {
            return Some(Suggestion { text: full.to_string(), confidence: 1.0 });
        }
        let hyps = self.hypotheses(core, times, ctx);
        if hyps.len() == 1 {
            return None;
        }
        let min = hyps.iter().map(|h| h.1).fold(f32::INFINITY, f32::min);
        let z: f32 = hyps.iter().map(|h| (min - h.1).exp()).sum();
        let best = hyps.iter().min_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap().clone();
        if best.0 == Hyp::Keep {
            return None;
        }
        let conf = 1.0 / z;
        let known = self.lx.id(core).is_some() || self.morph_stem(core).is_some();
        let tau = tau.unwrap_or(if known { self.cfg.tau_realword } else { self.cfg.tau_nonword });
        if conf < tau {
            return None;
        }
        Some(Suggestion { text: self.hyp_text(&best.0, core), confidence: conf })
    }
}

/// Contractions typed without the apostrophe. Forms that are also real words
/// ("wont", "cant", "ill", "were", "its", "shed", "hell", "well", "id") are
/// deliberately absent: those go through the normal real-word path.
pub fn restore_apostrophe(core: &str) -> Option<&'static str> {
    const TABLE: &[(&str, &str)] = &[
        ("dont", "don't"), ("im", "i'm"), ("ive", "i've"), ("didnt", "didn't"), ("doesnt", "doesn't"),
        ("isnt", "isn't"), ("wasnt", "wasn't"), ("werent", "weren't"), ("arent", "aren't"),
        ("havent", "haven't"), ("hasnt", "hasn't"), ("hadnt", "hadn't"), ("couldnt", "couldn't"),
        ("wouldnt", "wouldn't"), ("shouldnt", "shouldn't"), ("thats", "that's"), ("whats", "what's"),
        ("youre", "you're"), ("theyre", "they're"), ("youve", "you've"), ("theyve", "they've"),
        ("weve", "we've"), ("youll", "you'll"), ("theyll", "they'll"), ("itll", "it'll"),
        ("didn", "didn't"), ("doesn", "doesn't"), ("couldve", "could've"), ("wouldve", "would've"),
        ("shouldve", "should've"), ("theres", "there's"), ("heres", "here's"),
        ("aint", "ain't"), ("mustnt", "mustn't"), ("neednt", "needn't"),
    ];
    TABLE.iter().find(|(k, _)| *k == core).map(|(_, v)| *v)
}
