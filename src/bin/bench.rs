//! qk-bench: measure correction accuracy, false corrections, latency and an
//! effective-WPM estimate on simulated sloppy typing of clean English.
//!
//!   qk-bench [--data data/eval/dev.txt] [--level light|moderate|messy|all]
//!            [--sentences N] [--seed S] [--configs full,plain,...]
//!            [--show N] [--show-left]
//!   qk-bench --tune [--data data/eval/dev.txt]   grid-search gate parameters

use std::collections::HashMap;
use std::time::Instant;

use quipkeys::lexicon::Lexicon;
use quipkeys::sim::{SimParams, SimToken, Typist};
use quipkeys::stream::{parse_token, Stream};
use quipkeys::{Config, Engine};

fn configs() -> Vec<(&'static str, Config)> {
    let full = Config::default();
    let mut plain = Config::default();
    plain.channel.geometry = false;
    plain.channel.timing = false;
    plain.context = false;
    plain.right_context = false;
    plain.splits = false;
    let mut no_geo = Config::default();
    no_geo.channel.geometry = false;
    let mut no_ctx = Config::default();
    no_ctx.context = false;
    no_ctx.right_context = false;
    let mut no_right = Config::default();
    no_right.right_context = false;
    let mut no_tri = Config::default();
    no_tri.trigrams = false;
    let mut no_space = Config::default();
    no_space.boundary = false;
    let mut no_lb2 = Config::default();
    no_lb2.lookback2 = false;
    let mut no_mem = Config::default();
    no_mem.doc_cache = 0.0;
    no_mem.learned_weight = 0.0;
    // Error model matched to rough aim: adjacent slips cheap, far keys steep.
    let mut sloppy = Config::default();
    sloppy.channel.sub_adjacent = 3.0;
    sloppy.channel.sub_per_key = 5.0;
    sloppy.channel.first_letter = 0.5;
    let mut sloppy2 = sloppy.clone();
    sloppy2.channel.sub_adjacent = 2.0;
    sloppy2.channel.sub_per_key = 6.0;
    let mut no4 = Config::default();
    no4.lambda4 = 0.0;
    let mut nn = Config::default();
    nn.nn_weight = 0.5;
    let mut nn2 = Config::default();
    nn2.nn_weight = 1.0;
    let mut lat = Config::default();
    lat.lattice = false;
    let mut no_time = Config::default();
    no_time.channel.timing = false;
    vec![
        ("full", full),
        ("plain", plain),
        ("no-geometry", no_geo),
        ("no-context", no_ctx),
        ("no-right-ctx", no_right),
        ("no-trigram", no_tri),
        ("no-boundary", no_space),
        ("no-lookback2", no_lb2),
        ("no-memory", no_mem),
        ("word-by-word", lat),
        ("nn", nn),
        ("nn-strong", nn2),
        ("no-4gram", no4),
        ("sloppy", sloppy),
        ("sloppier", sloppy2),
        ("no-timing", no_time),
    ]
}

#[derive(Default, Clone)]
struct Stats {
    clean: u64,
    clean_changed: u64,
    err: u64,
    err_fixed: u64,
    err_untouched: u64,
    err_wrong: u64,
    rw_err: u64,
    rw_fixed: u64,
    nw_err: u64,
    nw_fixed: u64,
    chars_intended: u64,
    chars_typed: u64,
    /// Keystrokes the user would spend repairing what is left wrong.
    repair_keys: u64,
    repairs: u64,
    lat_us: Vec<f64>,
    sp_err: u64,
    sp_fixed: u64,
}

struct Unit<'a> {
    intended: &'a String,
    typed: &'a String,
}

fn pct(a: u64, b: u64) -> f64 {
    if b == 0 {
        0.0
    } else {
        100.0 * a as f64 / b as f64
    }
}

impl Stats {
    /// Effective WPM for a typist with raw speed `raw_wpm` who notices and
    /// fixes every wrong word right away (backspace over it, retype it,
    /// 0.5 s to notice).
    fn effective_wpm(&self, raw_wpm: f64) -> f64 {
        let keys_per_s = raw_wpm * 5.0 / 60.0;
        let secs = (self.chars_typed + self.repair_keys) as f64 / keys_per_s + 0.5 * self.repairs as f64;
        (self.chars_intended as f64 / 5.0) / (secs / 60.0)
    }
    fn word_acc(&self) -> f64 {
        pct(self.clean - self.clean_changed + self.err_fixed, self.clean + self.err)
    }
}

fn percentile(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

#[derive(Default)]
struct Show {
    n: usize,
    left: bool,
}

/// Run the engine over simulated sentences. `engine == None` means "no correction".
fn evaluate(engine: Option<&Engine>, lx: &Lexicon, sim: &[Vec<SimToken>], show: &Show) -> Stats {
    let mut st = Stats::default();
    let mut shown = 0;
    for toks in sim {
        let outputs: Vec<String> = match engine {
            None => toks.iter().map(|t| t.typed.clone()).collect(),
            Some(e) => {
                let mut s = Stream::new(e);
                for t in toks {
                    let start = Instant::now();
                    s.push(&t.typed, Some(t.times.clone()));
                    st.lat_us.push(start.elapsed().as_secs_f64() * 1e6);
                }
                s.words.into_iter().map(|w| w.output).collect()
            }
        };
        // Group tokens into units (spacing errors span 2 tokens and/or 2 words).
        let mut units: Vec<(String, String, String, usize, bool)> = Vec::new();
        for (t, out) in toks.iter().zip(&outputs) {
            match units.last_mut() {
                Some(u) if u.3 == t.unit => {
                    u.1 = format!("{} {}", u.1, t.typed);
                    if !out.is_empty() {
                        u.2 = if u.2.is_empty() { out.clone() } else { format!("{} {}", u.2, out) };
                    }
                }
                _ => units.push((t.intended.clone(), t.typed.clone(), out.clone(), t.unit, t.spacing)),
            }
        }
        for (intended, typed, out, _, spacing) in &units {
            let t = Unit { intended, typed };
            let len = t.intended.len() as u64 + 1;
            st.chars_intended += len;
            st.chars_typed += t.typed.len() as u64 + 1;
            if *spacing {
                st.sp_err += 1;
                if out == intended {
                    st.sp_fixed += 1;
                }
            }
            let ok = out == t.intended;
            if !ok {
                st.repairs += 1;
                st.repair_keys += (out.len() as u64 + 1) + len;
            }
            if *t.typed == *t.intended {
                st.clean += 1;
                if !ok {
                    st.clean_changed += 1;
                    if shown < show.n {
                        shown += 1;
                        let ctx: Vec<&str> = toks.iter().map(|x| x.intended.as_str()).filter(|x| !x.is_empty()).collect();
                        println!("   false: {:>14} -> {:<14} | {}", t.typed, out, ctx.join(" "));
                    }
                }
                continue;
            }
            st.err += 1;
            let core = parse_token(&t.typed, true).core.to_ascii_lowercase();
            let real = lx.id(&core).is_some();
            if real {
                st.rw_err += 1;
            } else {
                st.nw_err += 1;
            }
            if ok {
                st.err_fixed += 1;
                if real {
                    st.rw_fixed += 1;
                } else {
                    st.nw_fixed += 1;
                }
            } else if out == typed {
                st.err_untouched += 1;
                if show.left && shown < show.n {
                    shown += 1;
                    println!("    left: {:>14}    (meant {})", t.typed, t.intended);
                }
            } else {
                st.err_wrong += 1;
                if shown < show.n {
                    shown += 1;
                    println!("   wrong: {:>14} -> {:<14} (meant {})", t.typed, out, t.intended);
                }
            }
        }
    }
    st
}

fn simulate(sentences: &[&str], level: &str, seed: u64) -> Vec<Vec<SimToken>> {
    let mut ty = Typist::new(SimParams::level(level), seed);
    sentences.iter().map(|s| ty.type_sentence(s)).collect()
}

/// Grid search over the gate / weighting parameters on moderate, messy and
/// blind typing. Objective: +1 per fixed mistake, -1.5 per mistake changed to
/// the wrong word (still wrong, now harder to spot), -4 per correctly typed
/// word that gets changed (the thing that makes people turn autocorrect off).
fn tune(sentences: &[&str], seed: u64) {
    let sims: Vec<_> = ["moderate", "messy", "blind"].iter().map(|l| simulate(sentences, l, seed)).collect();
    let mut grid = Vec::new();
    for &tau in &[0.5f32, 0.7, 0.9] {
        for &oov in &[12.0f32, 14.0, 16.0] {
            for &per in &[1.0f32, 1.5, 2.0] {
                for &rwe in &[0.0f32, 1.5] {
                    for &taur in &[0.95f32, 0.98] {
                        grid.push((tau, oov, per, rwe, taur));
                    }
                }
            }
        }
    }
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let chunks: Vec<_> = grid.chunks(grid.len().div_ceil(threads)).map(|c| c.to_vec()).collect();
    let mut results: Vec<(f64, String)> = std::thread::scope(|s| {
        let hs: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                let sims = &sims;
                s.spawn(move || {
                    let mut e = Engine::new(Config::default());
                    let mut out = Vec::new();
                    for (tau, oov, per, rwe, taur) in chunk {
                        e.cfg.tau_nonword = tau;
                        e.cfg.oov_keep_base = oov;
                        e.cfg.oov_keep_per_char = per;
                        e.cfg.real_word_extra = rwe;
                        e.cfg.tau_realword = taur;
                        e.cfg.tau_revise = taur;
                        let (mut fixed, mut wrong, mut fc, mut clean, mut err) = (0u64, 0u64, 0u64, 0u64, 0u64);
                        for sim in sims {
                            let st = evaluate(Some(&e), &e.lx, sim, &Show::default());
                            fixed += st.err_fixed;
                            wrong += st.err_wrong;
                            fc += st.clean_changed;
                            clean += st.clean;
                            err += st.err;
                        }
                        let score = fixed as f64 - 1.5 * wrong as f64 - 4.0 * fc as f64;
                        out.push((
                            score,
                            format!(
                                "tau={tau:<3} oov={oov:<4} per={per:<3} rwe={rwe:<3} taur={taur:<4} | fixed {:.1}% wrong {:.1}% falsecorr {:.3}%",
                                pct(fixed, err),
                                pct(wrong, err),
                                pct(fc, clean)
                            ),
                        ));
                    }
                    out
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    for (s, r) in results.iter().take(15) {
        println!("{s:>7.0}  {r}");
    }
}

/// Collect (features, label, weight) for every change the decoder considers.
fn gate_examples(e: &Engine, sim: &[Vec<SimToken>]) -> Vec<(Vec<f32>, f32, f32, bool)> {
    let norm = |x: &str| {
        let t: String = x.chars().filter(|c| c.is_ascii_alphabetic() || *c == '\'' || *c == ' ').collect::<String>().to_ascii_lowercase();
        t.split_whitespace().collect::<Vec<_>>().join(" ")
    };
    let mut out = Vec::new();
    for toks in sim {
        let mut s = Stream::new(e);
        s.log = Some(Vec::new());
        for t in toks {
            s.push(&t.typed, Some(t.times.clone()));
        }
        // unit -> (first token, token count)
        let mut first: HashMap<usize, (usize, usize)> = HashMap::new();
        for (i, t) in toks.iter().enumerate() {
            first.entry(t.unit).or_insert((i, 0)).1 += 1;
        }
        for ex in s.log.take().unwrap() {
            let u = toks[ex.chunk].unit;
            let (f0, n) = first[&u];
            if f0 != ex.chunk || n != ex.span {
                continue; // decision doesn't line up with one meaning unit
            }
            let intended = norm(&toks[f0].intended);
            let typed = norm(&toks[f0..f0 + n].iter().map(|t| t.typed.as_str()).collect::<Vec<_>>().join(" "));
            let new = norm(&ex.outs.join(" "));
            let clean = typed == intended;
            let (y, w) = if !clean && new == intended {
                (1.0, 1.0)
            } else if clean {
                (0.0, 4.0)
            } else {
                (0.0, 1.5)
            };
            out.push((ex.feats, y, w, ex.applied));
        }
    }
    out
}

/// Weighted logistic regression (full-batch Adam with light L2).
fn fit_logistic(data: &[(Vec<f32>, f32, f32, bool)]) -> Vec<f32> {
    let d = data[0].0.len();
    let mut w = vec![0f32; d];
    let (mut m, mut v) = (vec![0f32; d], vec![0f32; d]);
    let wsum: f32 = data.iter().map(|x| x.2).sum();
    for it in 1..=600 {
        let mut g = vec![0f32; d];
        for (x, y, wt, _) in data {
            let z: f32 = w.iter().zip(x).map(|(a, b)| a * b).sum();
            let p = 1.0 / (1.0 + (-z).exp());
            for j in 0..d {
                g[j] += wt * (p - y) * x[j];
            }
        }
        for j in 0..d {
            let gj = g[j] / wsum + 1e-4 * w[j];
            m[j] = 0.9 * m[j] + 0.1 * gj;
            v[j] = 0.999 * v[j] + 0.001 * gj * gj;
            let mh = m[j] / (1.0 - 0.9f32.powi(it));
            let vh = v[j] / (1.0 - 0.999f32.powi(it));
            w[j] -= 0.05 * mh / (vh.sqrt() + 1e-8);
        }
    }
    w
}

/// Objective of a set of decisions: +1 fix, -1.5 wrong change, -4 clean word changed.
fn gate_objective(data: &[(Vec<f32>, f32, f32, bool)], apply: impl Fn(&[f32], bool) -> bool) -> f64 {
    data.iter()
        .filter(|(x, _, _, a)| apply(x, *a))
        .map(|(_, y, w, _)| if *y > 0.5 { 1.0 } else { -(*w as f64) })
        .sum()
}

fn train_gate(sentences: &[&str], seed: u64) {
    let mut cfg = Config::default();
    cfg.use_gate = false;
    let e = Engine::new(cfg);
    let half = sentences.len() / 2;
    let mut train = Vec::new();
    let mut valid = Vec::new();
    for lvl in ["light", "moderate", "messy", "blind"] {
        let sim = simulate(sentences, lvl, seed);
        train.extend(gate_examples(&e, &sim[..half]));
        valid.extend(gate_examples(&e, &sim[half..]));
        eprintln!("{lvl}: {} train / {} valid examples so far", train.len(), valid.len());
    }
    let w = fit_logistic(&train);
    let g = quipkeys::decoder::Gate { w: w.clone(), threshold: 0.5 };
    for (name, set) in [("train", &train), ("valid", &valid)] {
        let fixed = gate_objective(set, |_, a| a);
        let learned = gate_objective(set, |x, _| g.prob(x) >= 0.5);
        println!("{name}: objective with fixed thresholds {fixed:.0}, with learned gate {learned:.0}");
    }
    let mut txt = String::from("# Learned gate weights (qk-bench --train-gate on dev sentences, all typing levels).\n");
    for (n, v) in quipkeys::decoder::GATE_FEATURES.iter().zip(&w) {
        txt.push_str(&format!("{n} {v:.5}\n"));
        println!("  {n:<14} {v:+.3}");
    }
    txt.push_str("threshold 0.5\n");
    std::fs::write("data/gate.txt", txt).unwrap();
    println!("wrote data/gate.txt");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let data = get("--data").unwrap_or_else(|| "data/eval/dev.txt".into());
    let level = get("--level").unwrap_or_else(|| "moderate".into());
    let n: usize = get("--sentences").and_then(|s| s.parse().ok()).unwrap_or(1500);
    let seed: u64 = get("--seed").and_then(|s| s.parse().ok()).unwrap_or(7);
    let show = Show {
        n: get("--show").and_then(|s| s.parse().ok()).unwrap_or(0),
        left: args.iter().any(|a| a == "--show-left"),
    };
    let wanted: Option<Vec<String>> = get("--configs").map(|s| s.split(',').map(String::from).collect());
    let text = std::fs::read_to_string(&data).expect("read eval data");
    // A docs file (blank-line separated texts) is evaluated one session per
    // document, so document memory carries across its sentences.
    let units: Vec<String> = if data.contains("docs") {
        text.split("\n\n").map(|d| d.lines().collect::<Vec<_>>().join(" ")).filter(|d| !d.trim().is_empty()).take(n).collect()
    } else {
        text.lines().filter(|l| !l.trim().is_empty()).take(n).map(String::from).collect()
    };
    let sentences: Vec<&str> = units.iter().map(String::as_str).collect();

    if args.iter().any(|a| a == "--train-gate") {
        train_gate(&sentences, seed);
        return;
    }
    if args.iter().any(|a| a == "--tune") {
        tune(&sentences, seed);
        return;
    }

    let t0 = Instant::now();
    let base = Engine::new(Config::default());
    eprintln!("dictionary: {} words, loaded in {:.0} ms", base.lx.len(), t0.elapsed().as_secs_f64() * 1e3);
    let levels: Vec<&str> = if level == "all" { vec!["light", "moderate", "messy", "blind", "rough"] } else { vec![level.as_str()] };

    for lvl in levels {
        let p = SimParams::level(lvl);
        let sim = simulate(&sentences, lvl, seed);
        println!(
            "\n== {} typing: {:.1}% of letters mistyped, {} sentences from {} ==",
            lvl,
            p.char_error_rate * 100.0,
            sim.len(),
            data
        );
        println!(
            "{:<13} {:>7} {:>7} {:>7} {:>9} {:>9} {:>8} {:>7} {:>6} {:>6}",
            "config", "fixed", "wrong", "left", "realword", "falsecorr", "word-acc", "eff-WPM", "p50µs", "p99µs"
        );
        let none = evaluate(None, &base.lx, &sim, &Show::default());
        println!(
            "{:<13} {:>7} {:>7} {:>6.1}% {:>9} {:>9} {:>7.2}% {:>7.1} {:>6} {:>6}",
            "no-correct",
            "-",
            "-",
            100.0,
            "-",
            "-",
            none.word_acc(),
            none.effective_wpm(80.0),
            "-",
            "-"
        );
        for (name, cfg) in configs() {
            if let Some(w) = &wanted {
                if !w.iter().any(|x| x == name) {
                    continue;
                }
            }
            let owned;
            let engine = if name == "full" {
                &base
            } else {
                owned = Engine::new(cfg);
                &owned
            };
            let mut st = evaluate(Some(engine), &engine.lx, &sim, &show);
            let p50 = percentile(&mut st.lat_us, 0.5);
            let p99 = percentile(&mut st.lat_us, 0.99);
            println!(
                "{:<13} {:>6.1}% {:>6.1}% {:>6.1}% {:>8.1}% {:>8.3}% {:>7.2}% {:>7.1} {:>6.0} {:>6.0}",
                name,
                pct(st.err_fixed, st.err),
                pct(st.err_wrong, st.err),
                pct(st.err_untouched, st.err),
                pct(st.rw_fixed, st.rw_err),
                pct(st.clean_changed, st.clean),
                st.word_acc(),
                st.effective_wpm(80.0),
                p50,
                p99
            );
            if name == "full" {
                println!(
                    "{:<13} ({} mistyped units: {} non-words, {} real words, {} spacing; {} clean; fix rate: non-word {:.1}%, spacing {:.1}%)",
                    "",
                    st.err,
                    st.nw_err,
                    st.rw_err,
                    st.sp_err,
                    st.clean,
                    pct(st.nw_fixed, st.nw_err),
                    pct(st.sp_fixed, st.sp_err)
                );
            }
        }
    }
    println!("\nfixed / wrong / left: share of mistyped words corrected / changed to the wrong word / left alone.");
    println!("realword: share of mistypes that happen to be real words (form/from) that got fixed.");
    println!("falsecorr: share of correctly typed words the engine changed.");
    println!("eff-WPM: an 80 WPM typist who notices and retypes every wrong word (model, not a user study).");
}
