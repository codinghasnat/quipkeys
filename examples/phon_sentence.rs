//! Run sentences through the decoder with Spell by sound on, printing the
//! word-by-word states. Used for the pitch-video examples.
use quipkeys::decoder::{Config, Engine};
use quipkeys::stream::Stream;
fn main() {
    let mut cfg = Config::default(); cfg.phonetic = true;
    let e = Engine::new(cfg);
    for line in std::io::stdin().lines() { let line = line.unwrap();
        let mut s = Stream::new(&e); let mut steps = vec![];
        for t in line.split_whitespace() { s.push(t, None); steps.push(s.text()); }
        println!("{line}  =>  {}\n   STEPS {}", s.text(), steps.join(" || "));
    }
}
