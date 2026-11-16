//! Simulate what the app learns about a typist: feed recorded typing through
//! the stream (same stdin format as examples/replay.rs) and write the tallies
//! it would have collected from its own fixes. No correct answers are used.
//!
//!   profile_sim tallies.txt < tokens
//!   QK_TALLIES=tallies.txt replay < other tokens      (apply them)
use std::io::BufRead;
fn main() {
    let out = std::env::args().nth(1).expect("output file");
    let engine = quipkeys::Engine::new(quipkeys::Config::default());
    let mut tallies = quipkeys::profile::Tallies::default();
    let mut stream = quipkeys::Stream::new(&engine);
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        if line.is_empty() {
            stream = quipkeys::Stream::with_typist(&engine, stream.typist());
            continue;
        }
        let (tok, times) = line.split_once('\t').unwrap_or((&line, ""));
        let times: Vec<f32> = times.split(',').filter_map(|x| x.parse().ok()).collect();
        let times = (times.len() == tok.len()).then_some(times);
        let edits = stream.push(tok, times);
        tallies.learn(&engine, &stream, &edits);
    }
    std::fs::write(&out, tallies.to_text()).unwrap();
    eprintln!("{} letters; swaps {}, drops {}, extra keys {}", tallies.letters, tallies.transpose, tallies.del_other + tallies.del_double,
        tallies.ins_double + tallies.ins_adjacent + tallies.ins_other);
}
