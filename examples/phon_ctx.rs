//! Misspellings in real sentences: for each "typed intended" pair, take up to
//! N sentences containing the intended word, put the misspelling in its
//! place, and run the whole sentence through the stream decoder.
//!
//!   cargo run --release --example phon_ctx -- data/eval/phonetic_dev.txt data/eval/dev.txt 3
//!   QK_PHON=1 (sounds-like mode); QK_PHON_BASE / QK_PHON_SCALE / QK_PHON_MAX; QK_SHOW=1
use quipkeys::decoder::{Config, Engine};
use quipkeys::stream::Stream;

fn env(k: &str) -> Option<f32> {
    std::env::var(k).ok().and_then(|v| v.parse().ok())
}

fn core(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_lowercase()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let per: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
    let mut cfg = Config::default();
    cfg.phonetic = std::env::var_os("QK_PHON").is_some();
    if let Some(v) = env("QK_PHON_BASE") { cfg.phon_base = v; }
    if let Some(v) = env("QK_PHON_SCALE") { cfg.phon_scale = v; }
    if let Some(v) = env("QK_PHON_MAX") { cfg.phon_max = v; }
    if let Some(v) = env("QK_PHON_HOMO") { cfg.phon_homophone = v; }
    let e = Engine::new(cfg);
    let show = std::env::var_os("QK_SHOW").is_some();
    let sentences: Vec<Vec<String>> = std::fs::read_to_string(&a[2]).unwrap().lines()
        .map(|l| l.split_whitespace().map(String::from).collect()).filter(|v: &Vec<String>| v.len() >= 4).collect();
    let (mut n, mut right, mut kept, mut wrong, mut others, mut other_bad, mut pairs_found) = (0, 0, 0, 0, 0, 0, 0);
    for line in std::fs::read_to_string(&a[1]).unwrap().lines() {
        if line.starts_with('#') || line.trim().is_empty() { continue; }
        let mut it = line.split_whitespace();
        let (Some(typed), Some(want)) = (it.next(), it.next()) else { continue };
        if typed == want { continue; }
        let mut used = 0;
        for s in &sentences {
            if used == per { break; }
            let Some(pos) = s.iter().position(|w| core(w) == want) else { continue };
            used += 1;
            // Put the misspelling in, keeping punctuation and a leading capital.
            let orig = &s[pos];
            let mut tok = orig.to_lowercase().replacen(want, typed, 1);
            if orig.chars().next().is_some_and(|c| c.is_uppercase()) {
                let mut c = tok.chars();
                tok = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
            }
            let mut st = Stream::new(&e);
            for (k, w) in s.iter().enumerate() {
                st.push(if k == pos { &tok } else { w }, None);
            }
            let out: Vec<String> = st.words.iter().map(|w| w.output.clone()).collect();
            n += 1;
            let got = core(&out[pos]);
            if got == want { right += 1 } else if got == core(typed) { kept += 1 } else { wrong += 1 }
            let mut bad = Vec::new();
            for (k, w) in s.iter().enumerate() {
                if k == pos { continue; }
                others += 1;
                if core(&out[k]) != core(w) { other_bad += 1; bad.push(format!("{w}->{}", out[k])); }
            }
            if show && (got != want || !bad.is_empty()) {
                println!("{typed}->{got} (want {want}) {}  | {}", bad.join(" "), s.join(" "));
            }
        }
        if used > 0 { pairs_found += 1; }
    }
    let pct = |x: i32| 100.0 * x as f64 / n.max(1) as f64;
    println!(
        "{n} sentences ({pairs_found} pairs): right {:.1}%  left alone {:.1}%  wrong {:.1}%   other words changed {other_bad}/{others} ({:.2}%)",
        pct(right), pct(kept), pct(wrong), 100.0 * other_bad as f64 / others.max(1) as f64
    );
}
