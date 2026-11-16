//! Sound-alike real words in sentences (their/there, no/know, by/buy).
//!
//! For each sentence, one word that has a common homophone is swapped for it
//! (the mistake), and the sentence is run through the stream with Spell by
//! sound on: was the right word restored? The untouched sentences are run
//! too, to count correct words that get changed.
//!
//!   cargo run --release --example homophone_eval -- data/eval/dev.txt [max sentences]
//!   QK_PHON_HOMO=<cost>  QK_TAUR=<confidence for real-word changes>  QK_SHOW=1
use quipkeys::decoder::{Config, Engine};
use quipkeys::stream::Stream;

fn core(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_lowercase()
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let max: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1500);
    let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f32>().ok());
    let mut cfg = Config { phonetic: true, ..Config::default() };
    cfg.phon_base = cfg.phon_base_sound;
    if let Some(v) = env("QK_PHON_HOMO") { cfg.phon_homophone = v; }
    if let Some(v) = env("QK_PHON_REAL") { cfg.phon_real_base = v; }
    if let Some(v) = env("QK_TAUR") { cfg.tau_realword = v; cfg.tau_revise = v; }
    if std::env::var_os("QK_OFF").is_some() { cfg.phonetic = false; }
    let e = Engine::new(cfg);
    let show = std::env::var_os("QK_SHOW").is_some();
    let common = |id| e.lx.unigram_p(id) >= 1e-5; // about once per 100,000 words or more
    let sentences: Vec<Vec<String>> = std::fs::read_to_string(&a[1]).unwrap().lines()
        .map(|l| l.split_whitespace().map(String::from).collect()).filter(|v: &Vec<String>| v.len() >= 5).take(max).collect();
    let run = |toks: &[String]| -> Vec<String> {
        let mut st = Stream::new(&e);
        for w in toks { st.push(w, None); }
        st.words.iter().map(|w| w.output.clone()).collect()
    };
    let (mut n, mut restored, mut left, mut wrong) = (0, 0, 0, 0);
    let (mut clean_words, mut clean_changed, mut collateral, mut others) = (0, 0, 0, 0);
    for (si, s) in sentences.iter().enumerate() {
        // Clean pass: nothing should change.
        let out = run(s);
        for (w, o) in s.iter().zip(&out) {
            clean_words += 1;
            if core(w) != core(o) {
                clean_changed += 1;
                if show { println!("CLEAN CHANGED {w} -> {o} | {}", s.join(" ")); }
            }
        }
        // Mistake pass: swap one word (not the first) for a common homophone.
        let pick = (1..s.len()).filter(|&i| s[i].chars().all(|c| c.is_ascii_lowercase() || c == '\''))
            .filter_map(|i| {
                let id = e.lx.id(&s[i])?;
                let hs: Vec<u32> = e.phonetic().homophones(id).into_iter().filter(|&h| common(h) && common(id)).collect();
                (!hs.is_empty()).then(|| (i, hs[(si + i) % hs.len()]))
            }).nth(si % 2);
        let Some((i, h)) = pick else { continue };
        let mut t = s.clone();
        t[i] = e.lx.word(h).to_string();
        let out = run(&t);
        n += 1;
        let got = core(&out[i]);
        if got == s[i] { restored += 1 } else if got == t[i] { left += 1; if show { println!("left   {} (want {}) | {}", t[i], s[i], t.join(" ")); } }
        else { wrong += 1; if show { println!("WRONG  {} -> {} (want {}) | {}", t[i], got, s[i], t.join(" ")); } }
        for k in 0..s.len() { if k != i { others += 1; if core(&out[k]) != core(&s[k]) { collateral += 1; } } }
    }
    let p = |x: i32, d: i32| 100.0 * x as f64 / d.max(1) as f64;
    println!("{n} swapped sound-alikes: restored {:.1}%  left {:.1}%  wrong {:.1}%  | other words changed {:.2}%  | clean text: {clean_changed}/{clean_words} words changed ({:.3}%)",
        p(restored, n), p(left, n), p(wrong, n), p(collateral, others), p(clean_changed, clean_words));
}
