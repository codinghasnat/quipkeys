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
    let e: &'static quipkeys::Engine = Box::leak(Box::new(quipkeys::Engine::new(cfg)));
    let mut s = quipkeys::Stream::new(e);
    let lines: Vec<String> = std::io::stdin().lock().lines().map(|l| l.unwrap()).collect();
    for line in lines.iter().chain(std::iter::once(&String::new())) {
        if line.is_empty() {
            if !s.words.is_empty() {
                for w in &s.words { println!("{}", w.output); }
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
