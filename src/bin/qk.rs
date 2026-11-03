//! qk: correct text from stdin, line by line, the way the streaming engine
//! would while you type. `--explain` prints every edit with its confidence.

use std::io::{self, BufRead, Write};
use std::time::Instant;

use quipkeys::{Config, Engine, Stream};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let explain = args.iter().any(|a| a == "--explain");
    // qk --why <prev> <typed> [next]: show the top hypotheses for one word.
    if let Some(i) = args.iter().position(|a| a == "--why") {
        let engine = Engine::new(Config::default());
        let w = |s: Option<&String>| s.filter(|s| *s != "-").and_then(|s| engine.lx.id(s));
        let ctx = quipkeys::Ctx { prev: w(args.get(i + 1)), next: w(args.get(i + 3)), ..Default::default() };
        let core = &args[i + 2];
        let mut h = engine.hypotheses(core, None, &ctx);
        let min = h.iter().map(|x| x.1).fold(f32::INFINITY, f32::min);
        let z: f32 = h.iter().map(|x| (min - x.1).exp()).sum();
        h.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        for (hyp, s) in h.iter().take(8) {
            println!("{:>14}  score {:6.2}  p={:.3}", engine.hyp_text(hyp, core), s, (min - s).exp() / z);
        }
        println!("decision: {:?}", engine.decide(core, None, &ctx, None));
        return;
    }
    let t0 = Instant::now();
    let engine = Engine::new(Config::default());
    if explain {
        eprintln!("loaded {} words in {:.0} ms", engine.lx.len(), t0.elapsed().as_secs_f64() * 1e3);
    }
    let stdin = io::stdin();
    let mut out = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let mut s = Stream::new(&engine);
        for tok in line.split_whitespace() {
            let start = Instant::now();
            let edits = s.push(tok, None);
            let us = start.elapsed().as_secs_f64() * 1e6;
            if explain {
                for e in edits {
                    let kind = if e.revision { "revise" } else { "fix" };
                    eprintln!("  {kind:<6} {:>12} -> {:<12} p={:.3}  ({us:.0} µs)", e.old, e.new, e.confidence);
                }
            }
        }
        writeln!(out, "{}", s.text()).unwrap();
    }
}
