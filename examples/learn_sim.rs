//! Does learning a typist's habits help? A simulation of the app's loop.
//!
//! Several simulated typists with different habits each type for five "days"
//! (150 sentences a day from the dev split). Exactly as in the app, the
//! engine learns only from its own kept fixes and from correctly typed known
//! words, and what it learned by the end of one day shapes the engine the
//! next day. After each day the typist's held-out text (300 sentences of the
//! test split, typed once with a different random seed) is scored with the
//! profile learned so far, against the engine with no profile.
//!
//! Also reported after day 5: learning from the right answers instead (the
//! ceiling for this kind of model), and switching on the per-key confusions.
//!
//!   cargo run --release --example learn_sim [-- persona ...]
use quipkeys::profile::Tallies;
use quipkeys::sim::{SimParams, SimToken, Typist, DEFAULT_MIX};
use quipkeys::{Config, Engine, Stream};

const DAYS: usize = 5;
const PER_DAY: usize = 150;
const TEST: usize = 300;

fn persona(name: &str) -> SimParams {
    // [neighbouring key, swap, dropped letter, extra key, doubled key, far key]
    let (rate, mix, key_habit) = match name {
        "average" => (0.04, DEFAULT_MIX, 0.0),
        "accurate" => (0.01, DEFAULT_MIX, 0.0),
        "swapper" => (0.05, [0.15, 0.55, 0.12, 0.10, 0.05, 0.03], 0.0),
        "dropper" => (0.05, [0.15, 0.10, 0.57, 0.10, 0.05, 0.03], 0.0),
        "extra-keys" => (0.05, [0.15, 0.10, 0.10, 0.52, 0.10, 0.03], 0.0),
        // Like the one real typist measured: swaps, drops and extra keys about equally.
        "like-yours" => (0.06, [0.22, 0.26, 0.26, 0.20, 0.03, 0.03], 0.0),
        // Poor aim, and always onto the same wrong key for each letter.
        "habitual-aim" => (0.06, [0.80, 0.05, 0.05, 0.05, 0.03, 0.02], 0.9),
        _ => panic!("unknown persona {name}"),
    };
    SimParams {
        char_error_rate: rate,
        missing_space_rate: 0.0,
        premature_space_rate: 0.0,
        shifted_space_rate: 0.0,
        hand_shift_rate: 0.0,
        aim_sigma: 0.0,
        mix,
        key_habit,
    }
}

fn core(w: &str) -> String {
    w.trim_matches(|c: char| !c.is_alphanumeric() && c != '\'').to_lowercase()
}

/// (share of words right, share of correctly typed words that were changed)
fn score(engine: &Engine, test: &[Vec<SimToken>]) -> (f64, f64) {
    let (mut n, mut right, mut clean, mut broke) = (0, 0, 0, 0);
    let mut typist = None;
    for toks in test {
        let mut s = match typist.take() {
            Some(t) => Stream::with_typist(engine, t),
            None => Stream::new(engine),
        };
        for t in toks {
            s.push(&t.typed, Some(t.times.clone()));
        }
        for (t, w) in toks.iter().zip(&s.words) {
            n += 1;
            let ok = core(&w.output) == core(&t.intended);
            right += ok as u32;
            if core(&t.typed) == core(&t.intended) {
                clean += 1;
                broke += !ok as u32;
            }
        }
        typist = Some(s.typist());
    }
    (100.0 * right as f64 / n as f64, 100.0 * broke as f64 / clean.max(1) as f64)
}

fn engine_with(apply: impl Fn(&mut quipkeys::channel::ChannelParams)) -> Engine {
    let mut cfg = Config::default();
    apply(&mut cfg.channel);
    Engine::new(cfg)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let all = ["average", "accurate", "swapper", "dropper", "extra-keys", "like-yours", "habitual-aim"];
    let names: Vec<&str> = if args.is_empty() { all.to_vec() } else { args.iter().map(String::as_str).collect() };
    let read = |p: &str| -> Vec<String> { std::fs::read_to_string(p).unwrap().lines().filter(|l| !l.trim().is_empty()).map(String::from).collect() };
    let (train, test_text) = (read("data/eval/dev.txt"), read("data/eval/test.txt"));
    println!("Word accuracy on held-out text (300 sentences), and in brackets the share of correctly typed words changed.\n");
    println!("{:<13} {:>7} {:>15} {:>15} {:>15} {:>15} | {:>15} {:>15}", "typist", "typed", "no profile", "after day 1", "after day 2", "after day 5", "from answers", "+ key map");
    for name in names {
        let p = persona(name);
        let test: Vec<Vec<SimToken>> = {
            let mut ty = Typist::new(p.clone(), 1001);
            test_text.iter().take(TEST).map(|s| ty.type_sentence(s)).collect()
        };
        let raw = {
            let (mut n, mut ok) = (0, 0);
            for t in test.iter().flatten() {
                n += 1;
                ok += (core(&t.typed) == core(&t.intended)) as u32;
            }
            100.0 * ok as f64 / n as f64
        };
        let fmt = |(a, b): (f64, f64)| format!("{a:.2}% ({b:.2})");
        let mut row = vec![fmt(score(&engine_with(|_| {}), &test))];
        let mut learned = Tallies::default(); // from the engine's own fixes
        let mut oracle = Tallies::default(); // from the right answers
        let mut ty = Typist::new(p.clone(), 7);
        for day in 0..DAYS {
            // Today's engine is shaped by what was learned up to yesterday.
            let engine = engine_with(|c| learned.apply(c));
            let mut typist = None;
            for s in train.iter().skip(day * PER_DAY).take(PER_DAY) {
                let toks = ty.type_sentence(s);
                let mut st = match typist.take() {
                    Some(t) => Stream::with_typist(&engine, t),
                    None => Stream::new(&engine),
                };
                for t in &toks {
                    let edits = st.push(&t.typed, Some(t.times.clone()));
                    learned.learn(&engine, &st, &edits);
                    oracle.observe(&engine.ch.kb, &core(&t.typed), &core(&t.intended), true);
                }
                typist = Some(st.typist());
            }
            drop(engine);
            if [0, 1, 4].contains(&day) {
                row.push(fmt(score(&engine_with(|c| learned.apply(c)), &test)));
            }
        }
        let from_answers = fmt(score(&engine_with(|c| oracle.apply(c)), &test));
        let with_keys = fmt(score(&engine_with(|c| learned.apply_full(c, 0, 0)), &test));
        println!("{:<13} {:>6.1}% {:>15} {:>15} {:>15} {:>15} | {:>15} {:>15}", name, raw, row[0], row[1], row[2], row[3], from_answers, with_keys);
        let mut c = quipkeys::channel::ChannelParams::default();
        learned.apply(&mut c);
        eprintln!(
            "  {name}: learned from {} letters. Costs now: swap {:.1}, drop {:.1}, extra neighbouring key {:.1}, extra other key {:.1}, doubled {:.1} (defaults 5.0, 6.5, 6.0, 11.0, 5.5)",
            learned.letters, c.transpose, c.del_other, c.ins_adjacent, c.ins_other, c.ins_double
        );
    }
}
