//! Learn sound-edit costs from real misspellings ("typed intended" pairs).
//!
//! Each pair is aligned in sound space (typed spelling read by the letter-to-
//! sound rules, against the intended word's pronunciation). Only pairs that
//! already sound close are used, so keyboard slips don't teach it that, say,
//! r and t are confusable sounds. Counts of each edit become costs (log-odds
//! against writing the sound correctly), with the hand-set costs as a prior.
//! A few rounds of align -> count -> re-estimate.
//!
//!   cargo run --release --example phon_learn -- data/phon_costs.txt pairs.txt[:weight] ...
use quipkeys::lexicon::Lexicon;
use quipkeys::phonetic::{align, distance_with, spell_codes, Costs, Op, Phonetic};

const ROUNDS: usize = 4;
/// Pairs whose best sound distance is above this aren't sound spellings.
const MAX_DIST: f32 = 2.5;
/// Weight of the hand-set prior (pseudo-counts).
const PRIOR: f64 = 2.0;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = &args[1];
    let lx = Lexicon::english();
    let ph = Phonetic::build(&lx);
    // (typed codes, intended codes, weight)
    let mut data: Vec<(Vec<Vec<u8>>, Vec<Vec<u8>>, f64)> = Vec::new();
    for spec in &args[2..] {
        let (path, w) = match spec.rsplit_once(':') {
            Some((p, w)) if w.parse::<f64>().is_ok() => (p, w.parse().unwrap()),
            _ => (spec.as_str(), 1.0),
        };
        for line in std::fs::read_to_string(path).unwrap().lines() {
            let mut it = line.split_whitespace();
            let (Some(t), Some(i)) = (it.next(), it.next()) else { continue };
            if line.starts_with('#') || t == i {
                continue;
            }
            let Some(id) = lx.id(&i.to_lowercase()) else { continue };
            let wc = ph.codes(id).to_vec();
            if wc.is_empty() {
                continue;
            }
            data.push((spell_codes(t), wc, w));
        }
    }
    eprintln!("{} pairs", data.len());
    let hand = Costs::hand();
    let mut c = hand.clone();
    for round in 0..ROUNDS {
        let mut same = [0f64; 128];
        let mut sub = vec![[0f64; 128]; 128]; // [typed][intended]
        let mut del = [0f64; 128];
        let mut ins = [0f64; 128];
        let mut typed_total = 0f64;
        let (mut used, mut total_cost) = (0usize, 0f64);
        for (tcs, wcs, w) in &data {
            let mut best: Option<(f32, Vec<Op>)> = None;
            for t in tcs {
                for wc in wcs {
                    if best.as_ref().is_some_and(|b| distance_with(&c, t, wc) >= b.0) {
                        continue;
                    }
                    best = Some(align(&c, t, wc));
                }
            }
            let Some((d, ops)) = best else { continue };
            // The cut-off is in hand-cost units in the first round, then
            // scaled to the learned costs' typical size.
            let cap = if round == 0 { MAX_DIST } else { MAX_DIST * 2.5 };
            if d > cap {
                continue;
            }
            used += 1;
            total_cost += d as f64;
            for op in ops {
                match op {
                    Op::Same(b) => same[b as usize & 127] += w,
                    Op::Sub(a, b) => sub[a as usize & 127][b as usize & 127] += w,
                    Op::Del(b) => del[b as usize & 127] += w,
                    Op::Ins(a) => ins[a as usize & 127] += w,
                }
                if !matches!(op, Op::Del(_)) {
                    typed_total += w;
                }
            }
        }
        eprintln!("round {round}: {used} pairs used, mean cost {:.2}", total_cost / used.max(1) as f64);
        // Re-estimate: cost = ln(times written right / times written this way),
        // smoothed towards the hand-set costs.
        let clamp = |v: f64| v.clamp(0.05, 12.0) as f32;
        for b in 0..128 {
            let right = same[b] + 1.0;
            for a in 0..128 {
                if a != b {
                    let prior = PRIOR * (-(hand.sub[a][b] as f64)).exp();
                    c.sub[a][b] = clamp((right / (sub[a][b] + prior)).ln());
                }
            }
            c.del[b] = clamp((right / (del[b] + PRIOR * (-(hand.del[b] as f64)).exp())).ln());
        }
        for a in 0..128 {
            c.ins[a] = clamp(((typed_total + 1.0) / (ins[a] + PRIOR * (-(hand.ins[a] as f64)).exp())).ln());
        }
    }
    // b/d reversal is a well-known dyslexic writing pattern that misspelling
    // lists barely contain: keep it moderately cheap whatever the data says.
    for (a, b) in [(b'b', b'd'), (b'd', b'b')] {
        let v = &mut c.sub[a as usize][b as usize];
        *v = v.min(2.5);
    }
    // Write only the sounds the code uses.
    let syms: Vec<u8> = b"pbtdkgfvsmnlrwyhqxcjN123456789IWY".to_vec();
    let mut txt = String::from(
        "# Learned sound-edit costs (examples/phon_learn.rs): typed -> intended.\n\
         # s <typed> <intended> <cost> | i <extra typed sound> <cost> | d <missing sound> <cost>\n",
    );
    for &a in &syms {
        for &b in &syms {
            if a != b {
                txt.push_str(&format!("s {} {} {:.2}\n", a as char, b as char, c.sub[a as usize][b as usize]));
            }
        }
    }
    for &a in &syms {
        txt.push_str(&format!("i {} {:.2}\n", a as char, c.ins[a as usize]));
        txt.push_str(&format!("d {} {:.2}\n", a as char, c.del[a as usize]));
    }
    std::fs::write(out, txt).unwrap();
    eprintln!("wrote {out}");
}
