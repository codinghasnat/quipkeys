//! Simulated sloppy typing on a physical QWERTY keyboard, with key timings.
//!
//! No real keystroke logs are available here, so the benchmark needs a model.
//! The error mix follows what keystroke studies of physical keyboards report:
//! mostly adjacent-key slips, roll-over transpositions, dropped letters (often
//! in double letters) and fat-finger double presses, plus the occasional
//! missing space. Timings are synthetic: roll-over transpositions get very
//! short inter-key intervals, but so do a share of perfectly correct pairs,
//! so timing is a hint and not a give-away.

use crate::keyboard::Keyboard;

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn f(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[(self.next_u64() % v.len() as u64) as usize]
    }
}

#[derive(Clone, Debug)]
pub struct SimParams {
    /// Probability that any given letter is mistyped.
    pub char_error_rate: f32,
    /// Probability that the space after a word is dropped.
    pub missing_space_rate: f32,
    /// Probability of a stray space inside a word ("meetin g").
    pub premature_space_rate: f32,
    /// Probability that a space lands one letter early/late ("mee tyou").
    pub shifted_space_rate: f32,
    /// Probability that a whole word is typed with the hand one key off.
    pub hand_shift_rate: f32,
    /// "Rough aim" model (the north star): every press lands at a random
    /// point around the intended key, Gaussian with this spread in key
    /// widths, and the nearest key fires. 0 = off (discrete slips only).
    pub aim_sigma: f32,
}

impl SimParams {
    pub fn level(name: &str) -> SimParams {
        match name {
            "light" => SimParams {
                char_error_rate: 0.015,
                missing_space_rate: 0.003,
                premature_space_rate: 0.002,
                shifted_space_rate: 0.003,
                hand_shift_rate: 0.0,
                aim_sigma: 0.0,
            },
            "messy" => SimParams {
                char_error_rate: 0.06,
                missing_space_rate: 0.012,
                premature_space_rate: 0.008,
                shifted_space_rate: 0.012,
                hand_shift_rate: 0.002,
                aim_sigma: 0.0,
            },
            // The north star: typing without looking, fast and loose.
            "blind" => SimParams {
                char_error_rate: 0.10,
                missing_space_rate: 0.02,
                premature_space_rate: 0.012,
                shifted_space_rate: 0.02,
                hand_shift_rate: 0.006,
                aim_sigma: 0.0,
            },
            // Fingers land roughly near the key, fast, without looking.
            "rough" | "rougher" => SimParams {
                char_error_rate: 0.04, // roll-overs, dropped and doubled keys on top
                missing_space_rate: 0.02,
                premature_space_rate: 0.012,
                shifted_space_rate: 0.02,
                hand_shift_rate: 0.004,
                aim_sigma: if name == "rough" { 0.30 } else { 0.38 },
            },
            _ => SimParams {
                char_error_rate: 0.03,
                missing_space_rate: 0.006,
                premature_space_rate: 0.004,
                shifted_space_rate: 0.006,
                hand_shift_rate: 0.001,
                aim_sigma: 0.0,
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct SimToken {
    /// What the user meant by this token's *unit*: tokens sharing a `unit`
    /// together stand for `intended` (spacing errors make units of 1-2 tokens
    /// covering 1-2 words). Only the first token of a unit carries it.
    pub intended: String,
    pub unit: usize,
    /// The unit contains a spacing error.
    pub spacing: bool,
    /// What they typed.
    pub typed: String,
    /// Key-down time (ms) for each byte of `typed`.
    pub times: Vec<f32>,
}

#[derive(Clone, Copy, Debug)]
enum Err {
    Adjacent,
    Transpose,
    Omit,
    FatFinger,
    DoubleTap,
    Far,
}

fn choose_err(r: f32) -> Err {
    match r {
        x if x < 0.36 => Err::Adjacent,
        x if x < 0.58 => Err::Transpose,
        x if x < 0.77 => Err::Omit,
        x if x < 0.90 => Err::FatFinger,
        x if x < 0.96 => Err::DoubleTap,
        _ => Err::Far,
    }
}

fn match_case(c: u8, like: u8) -> u8 {
    if like.is_ascii_uppercase() {
        c.to_ascii_uppercase()
    } else {
        c
    }
}

pub struct Typist {
    kb: Keyboard,
    pub p: SimParams,
    rng: Rng,
}

impl Typist {
    pub fn new(p: SimParams, seed: u64) -> Self {
        Typist { kb: Keyboard::qwerty(), p, rng: Rng::new(seed) }
    }

    fn iki(&mut self) -> f32 {
        if self.rng.f() < 0.06 {
            8.0 + 30.0 * self.rng.f()
        } else {
            // Long-tailed, mean ~170 ms.
            70.0 + 100.0 * -(1.0 - self.rng.f()).max(1e-6).ln()
        }
    }

    /// Type one word's letters; returns (bytes, fast_pair flags as times).
    /// The whole word with the hand resting one key left or right.
    fn hand_shifted(&mut self, word: &str) -> Option<Vec<u8>> {
        const ROWS: [&[u8]; 3] = [b"qwertyuiop[]", b"asdfghjkl;'", b"zxcvbnm,./"];
        let dir: i32 = if self.rng.f() < 0.5 { -1 } else { 1 };
        word.bytes()
            .map(|c| {
                let lc = c.to_ascii_lowercase();
                let r = ROWS.iter().find(|r| r.contains(&lc))?;
                let i = r.iter().position(|&k| k == lc)? as i32 + dir;
                let k = *r.get(usize::try_from(i).ok()?)?;
                Some(if c.is_ascii_uppercase() { k.to_ascii_uppercase() } else { k })
            })
            .collect()
    }

    fn gauss(&mut self) -> f32 {
        let (u1, u2) = (self.rng.f().max(1e-7), self.rng.f());
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos()
    }

    /// Where a press aimed at `c` actually lands (nearest letter key).
    fn aim(&mut self, c: u8) -> u8 {
        let Some((x, y)) = self.kb.position(c) else { return c };
        let (px, py) = (x + self.p.aim_sigma * self.gauss(), y + 0.8 * self.p.aim_sigma * self.gauss());
        let mut best = (f32::MAX, c.to_ascii_lowercase());
        for k in b'a'..=b'z' {
            let (kx, ky) = self.kb.position(k).unwrap();
            let d = (kx - px).powi(2) + (ky - py).powi(2);
            if d < best.0 {
                best = (d, k);
            }
        }
        match_case(best.1, c)
    }

    fn type_word(&mut self, word: &str) -> (Vec<u8>, Vec<f32>) {
        let aimed: String;
        let word = if self.p.aim_sigma > 0.0 && word.bytes().filter(|c| c.is_ascii_alphabetic()).count() >= 2 {
            aimed = word.bytes().map(|c| if c.is_ascii_alphabetic() { self.aim(c) as char } else { c as char }).collect();
            aimed.as_str()
        } else {
            word
        };
        if word.len() >= 3 && word.bytes().all(|c| c.is_ascii_alphabetic()) && self.rng.f() < self.p.hand_shift_rate {
            if let Some(t) = self.hand_shifted(word) {
                let gaps = t.iter().map(|_| self.iki()).collect();
                return (t, gaps);
            }
        }
        let w = word.as_bytes();
        let letters = w.iter().filter(|c| c.is_ascii_alphabetic()).count();
        let mut out: Vec<u8> = Vec::with_capacity(w.len() + 2);
        let mut gaps: Vec<f32> = Vec::with_capacity(w.len() + 2);
        let mut i = 0;
        while i < w.len() {
            let c = w[i];
            let lc = c.to_ascii_lowercase();
            let corruptible = c.is_ascii_alphabetic() && letters >= 2;
            if corruptible && self.rng.f() < self.p.char_error_rate {
                let r = self.rng.f();
                let kind = if self.p.aim_sigma > 0.0 { choose_err(0.36 + r * 0.6) } else { choose_err(r) };
                match kind {
                    Err::Adjacent => {
                        let n = self.kb.neighbours(lc);
                        let n: Vec<u8> = n.into_iter().filter(|k| k.is_ascii_alphabetic()).collect();
                        out.push(match_case(self.rng.pick(&n), c));
                        gaps.push(self.iki());
                    }
                    Err::Far => {
                        let k = b'a' + (self.rng.next_u64() % 26) as u8;
                        out.push(match_case(k, c));
                        gaps.push(self.iki());
                    }
                    Err::Transpose if i + 1 < w.len() && w[i + 1].is_ascii_alphabetic() && w[i + 1] != c => {
                        out.push(w[i + 1]);
                        gaps.push(self.iki());
                        out.push(c);
                        gaps.push(4.0 + 36.0 * self.rng.f());
                        i += 2;
                        continue;
                    }
                    Err::Transpose | Err::Omit => {
                        // Dropping a letter is the fallback when a swap is impossible.
                    }
                    Err::FatFinger => {
                        let n = self.kb.neighbours(lc);
                        let n: Vec<u8> = n.into_iter().filter(|k| k.is_ascii_alphabetic()).collect();
                        let extra = match_case(self.rng.pick(&n), c);
                        let (a, b) = if self.rng.f() < 0.5 { (c, extra) } else { (extra, c) };
                        out.push(a);
                        gaps.push(self.iki());
                        out.push(b);
                        gaps.push(5.0 + 25.0 * self.rng.f());
                    }
                    Err::DoubleTap => {
                        out.push(c);
                        gaps.push(self.iki());
                        out.push(c);
                        gaps.push(40.0 + 60.0 * self.rng.f());
                    }
                }
            } else {
                out.push(c);
                gaps.push(self.iki());
            }
            i += 1;
        }
        (out, gaps)
    }

    /// Type a sentence; returns one token per typed (space-separated) token.
    pub fn type_sentence(&mut self, sentence: &str) -> Vec<SimToken> {
        let words: Vec<&str> = sentence.split_whitespace().collect();
        let plain = |s: &str| s.len() >= 2 && s.bytes().all(|c| c.is_ascii_alphabetic());
        // First decide the spacing: a list of units, each (intended, chunks).
        let mut units: Vec<(String, Vec<String>)> = Vec::new();
        let mut i = 0;
        while i < words.len() {
            let w = words[i];
            let next = words.get(i + 1).copied();
            let both = plain(w) && next.is_some_and(plain);
            let r = self.rng.f();
            let p = &self.p;
            if both && r < p.missing_space_rate {
                let n = next.unwrap();
                units.push((format!("{w} {n}"), vec![format!("{w}{n}")]));
                i += 2;
                continue;
            }
            if both && r < p.missing_space_rate + p.shifted_space_rate {
                let n = next.unwrap();
                let (x, y) = if self.rng.f() < 0.5 && w.len() > 2 {
                    // space one letter early: "mee tyou"
                    (w[..w.len() - 1].to_string(), format!("{}{n}", &w[w.len() - 1..]))
                } else {
                    // space one letter late: "hellon ice"
                    (format!("{w}{}", &n[..1]), n[1..].to_string())
                };
                units.push((format!("{w} {n}"), vec![x, y]));
                i += 2;
                continue;
            }
            if plain(w) && w.len() >= 4 && r < p.missing_space_rate + p.shifted_space_rate + p.premature_space_rate {
                let k = 1 + (self.rng.next_u64() % (w.len() as u64 - 1)) as usize;
                units.push((w.to_string(), vec![w[..k].to_string(), w[k..].to_string()]));
                i += 1;
                continue;
            }
            units.push((w.to_string(), vec![w.to_string()]));
            i += 1;
        }
        // Then type every chunk with letter-level slips and timing.
        let mut toks = Vec::new();
        for (u, (intended, chunks)) in units.into_iter().enumerate() {
            let spacing = chunks.len() > 1 || intended.contains(' ');
            for (k, chunk) in chunks.iter().enumerate() {
                let (mut typed, mut gaps) = self.type_word(chunk);
                if !typed.iter().any(|c| c.is_ascii_alphanumeric()) {
                    typed = chunk.as_bytes().to_vec();
                    gaps = typed.iter().map(|_| 150.0).collect();
                }
                let mut t = 0.0;
                let times = gaps
                    .iter()
                    .map(|g| {
                        t += g;
                        t
                    })
                    .collect();
                toks.push(SimToken {
                    intended: if k == 0 { intended.clone() } else { String::new() },
                    unit: u,
                    spacing,
                    typed: String::from_utf8(typed).unwrap(),
                    times,
                });
            }
        }
        toks
    }
}
