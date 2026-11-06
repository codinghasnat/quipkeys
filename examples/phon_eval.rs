//! Score isolated misspellings ("typed intended" per line) through the
//! engine's real decision (confidence gate included).
//!
//!   cargo run --release --example phon_eval -- data/eval/phonetic.txt
//!   QK_PHON=1 ... (sounds-like mode); QK_PHON_BASE / QK_PHON_SCALE / QK_PHON_MAX
//!   QK_SHOW=1 lists every miss.
use quipkeys::decoder::{Config, Ctx, Engine};

fn env(k: &str) -> Option<f32> {
    std::env::var(k).ok().and_then(|v| v.parse().ok())
}

fn main() {
    let path = std::env::args().nth(1).expect("pairs file");
    let mut cfg = Config::default();
    cfg.phonetic = std::env::var_os("QK_PHON").is_some();
    if let Some(v) = env("QK_PHON_BASE") { cfg.phon_base = v; }
    if let Some(v) = env("QK_PHON_SCALE") { cfg.phon_scale = v; }
    if let Some(v) = env("QK_PHON_MAX") { cfg.phon_max = v; }
    if let Some(v) = env("QK_PHON_HOMO") { cfg.phon_homophone = v; }
    let e = Engine::new(cfg);
    let show = std::env::var_os("QK_SHOW").is_some();
    let (mut n, mut right, mut kept, mut wrong, mut ctrl, mut ctrl_bad) = (0, 0, 0, 0, 0, 0);
    let t = std::time::Instant::now();
    for line in std::fs::read_to_string(&path).unwrap().lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let mut it = line.split_whitespace();
        let (Some(typed), Some(want)) = (it.next(), it.next()) else { continue };
        let (typed, want) = (typed.to_lowercase(), want.to_lowercase());
        let got = e.decide(&typed, None, &Ctx::default(), None).map(|s| s.text).unwrap_or_else(|| typed.clone());
        if typed == want {
            // A control: a correctly spelt word must stay.
            ctrl += 1;
            if got != want {
                ctrl_bad += 1;
                if show { println!("CHANGED  {typed} -> {got}"); }
            }
            continue;
        }
        n += 1;
        if got == want {
            right += 1;
        } else if got == typed {
            kept += 1;
            if show { println!("kept     {typed} (want {want})"); }
        } else {
            wrong += 1;
            if show { println!("WRONG    {typed} -> {got} (want {want})"); }
        }
    }
    let pct = |x: i32| 100.0 * x as f64 / n.max(1) as f64;
    println!(
        "{n} misspellings: right {:.1}%  left alone {:.1}%  wrong {:.1}%   controls changed {ctrl_bad}/{ctrl}   ({:.2} ms/word)",
        pct(right), pct(kept), pct(wrong), t.elapsed().as_secs_f64() * 1000.0 / (n + ctrl).max(1) as f64
    );
}
