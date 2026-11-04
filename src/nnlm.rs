//! Tiny neural word language model (inference only), trained by
//! `scripts/nn/train_lm.py`. Pure Rust, int8 weights, incremental KV cache.
//!
//! The decoder uses it only to re-rank the few candidate readings of a chunk:
//! it needs logits for a handful of words, never a full-vocabulary softmax.

pub struct QMat {
    rows: usize,
    cols: usize,
    scale: Vec<f32>,
    q: Vec<i8>,
    /// Dequantised copy for the per-token layer matrices (fast path).
    f: Vec<f32>,
}

/// Dot product with 8 independent accumulators so it vectorises.
#[inline]
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut acc = [0f32; 8];
    let (ca, cb) = (a.chunks_exact(8), b.chunks_exact(8));
    let (ra, rb) = (ca.remainder(), cb.remainder());
    for (x, y) in ca.zip(cb) {
        for k in 0..8 {
            acc[k] += x[k] * y[k];
        }
    }
    let mut s: f32 = acc.iter().sum();
    for (x, y) in ra.iter().zip(rb) {
        s += x * y;
    }
    s
}

impl QMat {
    /// y = W x (+ bias)
    fn matvec(&self, x: &[f32], bias: &[f32], y: &mut Vec<f32>) {
        y.clear();
        for r in 0..self.rows {
            y.push(dot(&self.f[r * self.cols..(r + 1) * self.cols], x) + bias[r]);
        }
    }

    fn dequantise(&mut self) {
        self.f = (0..self.rows * self.cols).map(|i| self.q[i] as f32 * self.scale[i / self.cols]).collect();
    }

    fn row_dot(&self, r: usize, x: &[f32]) -> f32 {
        let row = &self.q[r * self.cols..(r + 1) * self.cols];
        row.iter().zip(x).map(|(a, b)| *a as f32 * b).sum::<f32>() * self.scale[r]
    }

    fn row(&self, r: usize, out: &mut [f32]) {
        let row = &self.q[r * self.cols..(r + 1) * self.cols];
        for (o, a) in out.iter_mut().zip(row) {
            *o = *a as f32 * self.scale[r];
        }
    }
}

struct Block {
    ln1: (Vec<f32>, Vec<f32>),
    qkv: (QMat, Vec<f32>),
    proj: (QMat, Vec<f32>),
    ln2: (Vec<f32>, Vec<f32>),
    fc1: (QMat, Vec<f32>),
    fc2: (QMat, Vec<f32>),
}

pub struct NnLm {
    pub vocab: usize,
    d: usize,
    heads: usize,
    pub ctx: usize,
    tok: QMat,
    pos: Vec<f32>,
    blocks: Vec<Block>,
    lnf: (Vec<f32>, Vec<f32>),
}

/// Running state for one token sequence.
#[derive(Default, Clone)]
pub struct NnState {
    /// Index in the full history where this state's tokens begin.
    base: usize,
    pub tokens: Vec<u16>,
    k: Vec<Vec<f32>>,
    v: Vec<Vec<f32>>,
    /// Final hidden vector after each position.
    hidden: Vec<Vec<f32>>,
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn u32(&mut self) -> usize {
        let v = u32::from_le_bytes(self.b[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        v as usize
    }
    fn f32s(&mut self, n: usize) -> Vec<f32> {
        let v = self.b[self.pos..self.pos + 4 * n].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        self.pos += 4 * n;
        v
    }
    fn qmat(&mut self, rows: usize, cols: usize) -> QMat {
        let scale = self.f32s(rows);
        let q = self.b[self.pos..self.pos + rows * cols].iter().map(|&x| x as i8).collect();
        self.pos += rows * cols;
        QMat { rows, cols, scale, q, f: Vec::new() }
    }
}

fn layernorm(x: &[f32], p: &(Vec<f32>, Vec<f32>)) -> Vec<f32> {
    let n = x.len() as f32;
    let mean = x.iter().sum::<f32>() / n;
    let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f32>() / n;
    let inv = 1.0 / (var + 1e-5).sqrt();
    x.iter().zip(&p.0).zip(&p.1).map(|((v, w), b)| (v - mean) * inv * w + b).collect()
}

fn gelu(x: f32) -> f32 {
    0.5 * x * (1.0 + (0.797_884_6 * (x + 0.044715 * x * x * x)).tanh())
}

impl NnLm {
    pub fn load(bytes: &[u8]) -> Option<NnLm> {
        if bytes.len() < 30 || &bytes[..6] != b"QKNN1\n" {
            return None;
        }
        let mut r = Reader { b: bytes, pos: 6 };
        let (vocab, d, layers, heads, ctx) = (r.u32(), r.u32(), r.u32(), r.u32(), r.u32());
        let tok = r.qmat(vocab, d);
        let pos = r.f32s(ctx * d);
        let mut blocks = Vec::new();
        for _ in 0..layers {
            let ln1 = (r.f32s(d), r.f32s(d));
            let mut qkv = (r.qmat(3 * d, d), r.f32s(3 * d));
            let mut proj = (r.qmat(d, d), r.f32s(d));
            let ln2 = (r.f32s(d), r.f32s(d));
            let mut fc1 = (r.qmat(4 * d, d), r.f32s(4 * d));
            let mut fc2 = (r.qmat(d, 4 * d), r.f32s(d));
            for m in [&mut qkv.0, &mut proj.0, &mut fc1.0, &mut fc2.0] {
                m.dequantise();
            }
            blocks.push(Block { ln1, qkv, proj, ln2, fc1, fc2 });
        }
        let lnf = (r.f32s(d), r.f32s(d));
        Some(NnLm { vocab, d, heads, ctx, tok, pos, blocks, lnf })
    }

    pub fn unk(&self) -> u16 {
        (self.vocab - 2) as u16
    }
    pub fn bos(&self) -> u16 {
        (self.vocab - 1) as u16
    }

    /// Append one token, updating the cache.
    pub fn step(&self, st: &mut NnState, tok: u16) {
        let (d, h) = (self.d, self.heads);
        let hd = d / h;
        let t = st.tokens.len();
        if st.k.is_empty() {
            st.k = vec![Vec::new(); self.blocks.len()];
            st.v = vec![Vec::new(); self.blocks.len()];
        }
        let mut x = vec![0f32; d];
        self.tok.row(tok as usize, &mut x);
        for (xi, p) in x.iter_mut().zip(&self.pos[(t % self.ctx) * d..(t % self.ctx + 1) * d]) {
            *xi += p;
        }
        let mut buf = Vec::with_capacity(4 * d);
        for (l, b) in self.blocks.iter().enumerate() {
            let a = layernorm(&x, &b.ln1);
            b.qkv.0.matvec(&a, &b.qkv.1, &mut buf);
            let (q, kv) = buf.split_at(d);
            st.k[l].extend_from_slice(&kv[..d]);
            st.v[l].extend_from_slice(&kv[d..]);
            let n = t + 1;
            let mut att = vec![0f32; d];
            for hh in 0..h {
                let qh = &q[hh * hd..(hh + 1) * hd];
                let mut w: Vec<f32> = (0..n)
                    .map(|j| dot(qh, &st.k[l][j * d + hh * hd..j * d + (hh + 1) * hd]) / (hd as f32).sqrt())
                    .collect();
                let m = w.iter().cloned().fold(f32::MIN, f32::max);
                let mut z = 0.0;
                for v in &mut w {
                    *v = (*v - m).exp();
                    z += *v;
                }
                for (j, wj) in w.iter().enumerate() {
                    let vj = &st.v[l][j * d + hh * hd..j * d + (hh + 1) * hd];
                    for (o, vv) in att[hh * hd..(hh + 1) * hd].iter_mut().zip(vj) {
                        *o += wj / z * vv;
                    }
                }
            }
            b.proj.0.matvec(&att, &b.proj.1, &mut buf);
            for (xi, a) in x.iter_mut().zip(&buf) {
                *xi += a;
            }
            let a = layernorm(&x, &b.ln2);
            b.fc1.0.matvec(&a, &b.fc1.1, &mut buf);
            let g: Vec<f32> = buf.iter().map(|&v| gelu(v)).collect();
            b.fc2.0.matvec(&g, &b.fc2.1, &mut buf);
            for (xi, a) in x.iter_mut().zip(&buf) {
                *xi += a;
            }
        }
        st.hidden.push(layernorm(&x, &self.lnf));
        st.tokens.push(tok);
    }

    /// Make `st` represent the end of the full history `all` (reusing what
    /// it already has). At most `ctx` tokens are kept; when full, the state is
    /// rebuilt from the last half.
    pub fn sync(&self, st: &mut NnState, all: &[u16]) {
        let reuse = st.base <= all.len()
            && all.len() - st.base <= self.ctx
            && st.tokens.len() <= all.len() - st.base;
        let common = if reuse {
            st.tokens.iter().zip(&all[st.base..]).take_while(|(a, b)| a == b).count()
        } else {
            0
        };
        if !reuse || common == 0 && !st.tokens.is_empty() {
            let base = if all.len() > self.ctx { all.len() - self.ctx / 2 } else { 0 };
            *st = NnState { base, ..Default::default() };
        } else if common < st.tokens.len() {
            st.tokens.truncate(common);
            st.hidden.truncate(common);
            for l in 0..st.k.len() {
                st.k[l].truncate(common * self.d);
                st.v[l].truncate(common * self.d);
            }
        }
        let from = st.base + st.tokens.len();
        for &t in &all[from..] {
            self.step(st, t);
        }
    }

    /// log P(tokens | state), with a full softmax at each position. Expensive
    /// (a pass over the whole vocabulary per token); used only to rescore a
    /// few ambiguous candidates with the words that follow them.
    pub fn seq_logprob(&self, st: &NnState, tokens: &[u16]) -> f32 {
        let mut s = st.clone();
        let mut total = 0f32;
        for &t in tokens {
            let Some(h) = s.hidden.last() else { return f32::NEG_INFINITY };
            let logits: Vec<f32> = (0..self.vocab).map(|r| self.tok.row_dot(r, h)).collect();
            let m = logits.iter().cloned().fold(f32::MIN, f32::max);
            let lse = m + logits.iter().map(|l| (l - m).exp()).sum::<f32>().ln();
            total += logits[t as usize] - lse;
            self.step(&mut s, t);
        }
        total
    }

    /// Logit of `tok` coming next after the state's last token.
    pub fn logit(&self, st: &NnState, tok: u16) -> f32 {
        match st.hidden.last() {
            Some(h) => self.tok.row_dot(tok as usize, h),
            None => 0.0,
        }
    }
}
