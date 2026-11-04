// Replay real typed tokens (with key timings) through the engine.
// stdin: one token per line "token<TAB>t1,t2,...", blank line between runs.
// stdout: the corrected output per token, blank line between runs.
use std::io::BufRead;
fn main() {
    let mut cfg = quipkeys::Config::default();
    if std::env::var("QK_PERSONAL").is_ok() {
        // Fitted to the user's 12 real runs.
        cfg.channel.ins_other = 7.0;
        cfg.channel.del_other = 4.5;
        cfg.channel.transpose = 4.5;
        cfg.channel.timing = false;
        cfg.channel.sub_per_key = 2.0;
    }
    if std::env::var("QK_RHYTHM").is_ok() {
        cfg.channel.rhythm = true;
    }
    if let Ok(t) = std::env::var("QK_TAUR") {
        cfg.tau_realword = t.parse().unwrap();
        cfg.tau_revise = cfg.tau_realword;
        cfg.real_word_extra = 0.0;
    }
    if let Ok(t) = std::env::var("QK_LOOK") {
        cfg.nn_lookahead = 1.0;
        cfg.nn_look_tau = t.parse().unwrap();
    }
    if let Ok(w) = std::env::var("QK_NN") {
        cfg.nn_weight = w.parse().unwrap();
    }
    match std::env::var("QK_CONFIG").as_deref() {
        Ok("plain") => {
            cfg.channel.geometry = false;
            cfg.channel.timing = false;
            cfg.context = false;
            cfg.right_context = false;
            cfg.splits = false;
            cfg.boundary = false;
            cfg.lattice = false;
        }
        Ok("no-geometry") => cfg.channel.geometry = false,
        Ok("no-context") => {
            cfg.context = false;
            cfg.right_context = false;
        }
        Ok("no-trigram") => cfg.trigrams = false,
        Ok("word-by-word") => cfg.lattice = false,
        Ok("no-boundary") => cfg.boundary = false,
        _ => {}
    }
    let e: &'static quipkeys::Engine = Box::leak(Box::new(quipkeys::Engine::new(cfg)));
    let mut s = quipkeys::Stream::new(e);
    let lines: Vec<String> = std::io::stdin().lock().lines().map(|l| l.unwrap()).collect();
    for line in lines.iter().chain(std::iter::once(&String::new())) {
        if line.is_empty() {
            if !s.words.is_empty() {
                // A chunk merged into its neighbour has empty output: mark it, so a
                // blank line still means "end of run".
                for w in &s.words { println!("{}", if w.output.is_empty() { "\u{1}" } else { w.output.as_str() }); }
                println!();
            }
            s = quipkeys::Stream::new(e);
            continue;
        }
        let (tok, t) = line.split_once('\t').unwrap_or((line, ""));
        let times: Vec<f32> = t.split(',').filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect();
        s.push(tok, (times.len() == tok.len()).then_some(times));
    }
}
