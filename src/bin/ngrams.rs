//! qk-ngrams: count word bigrams and trigrams over plain text and write the
//! compact `data/en_ngrams.bin` that the engine loads.
//!
//!   qk-ngrams OUT.bin [--min2 N] [--min3 N] WEIGHT:FILE [WEIGHT:FILE ...]
//!
//! Tokenisation mirrors how the engine sees context while you type: words are
//! runs of letters/apostrophes, lowercased; '.', '!', '?' end a sentence; any
//! token that is not a dictionary word (numbers, names the dictionary lacks,
//! code) breaks the context, exactly as an unknown word does in the stream.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};

use quipkeys::lexicon::{write_ngrams, Lexicon, NgramCounts};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = &args[1];
    let mut min2 = 2u32;
    let mut min3 = 3u32;
    let mut caps_out: Option<String> = None;
    let mut inputs = Vec::new();
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--min2" => {
                min2 = args[i + 1].parse().unwrap();
                i += 1;
            }
            "--caps" => {
                caps_out = Some(args[i + 1].clone());
                i += 1;
            }
            "--min3" => {
                min3 = args[i + 1].parse().unwrap();
                i += 1;
            }
            a => {
                let (w, f) = a.split_once(':').expect("WEIGHT:FILE");
                inputs.push((w.parse::<u32>().unwrap(), f.to_string()));
            }
        }
        i += 1;
    }
    let lx = Lexicon::english_without_ngrams();
    let mut uni: Vec<u64> = vec![0; lx.len()];
    let mut bi: HashMap<u64, u32> = HashMap::new();
    let mut tri: HashMap<u64, u32> = HashMap::new();
    let v = lx.len() as u64;
    let mut tokens = 0u64;
    // Mid-sentence capitalised vs lowercase occurrences, per lexicon word.
    let mut cap: Vec<u32> = vec![0; lx.len()];
    let mut low: Vec<u32> = vec![0; lx.len()];
    for (weight, path) in &inputs {
        let f = BufReader::with_capacity(1 << 20, std::fs::File::open(path).expect("open input"));
        for line in f.lines() {
            let Ok(line) = line else { continue };
            let (mut p2, mut p1): (Option<u32>, Option<u32>) = (None, None);
            let mut sentence_start = true;
            for raw in line.split_whitespace() {
                let starts_sentence = std::mem::replace(&mut sentence_start, false);
                if raw.bytes().rev().take_while(|c| !c.is_ascii_alphabetic()).any(|c| b".!?".contains(&c)) {
                    sentence_start = true;
                }
                let b = raw.as_bytes();
                let s = b.iter().position(|c| c.is_ascii_alphabetic());
                let id = s.and_then(|s| {
                    let e = b.iter().rposition(|c| c.is_ascii_alphabetic()).unwrap() + 1;
                    let core = raw[s..e].to_ascii_lowercase();
                    if raw[..s].bytes().any(|c| c.is_ascii_alphanumeric()) {
                        return None;
                    }
                    lx.id(&core).or_else(|| core.strip_suffix("'s").and_then(|st| lx.id(st)))
                });
                tokens += 1;
                if let (Some(w), Some(s)) = (id, s) {
                    let b = &raw.as_bytes()[s..];
                    let rest_lower = b[1..].iter().take_while(|c| c.is_ascii_alphabetic()).all(|c| c.is_ascii_lowercase());
                    if !starts_sentence && rest_lower {
                        if b[0].is_ascii_uppercase() {
                            cap[w as usize] += 1;
                        } else {
                            low[w as usize] += 1;
                        }
                    }
                }
                match id {
                    Some(w) => {
                        uni[w as usize] += *weight as u64;
                        if let Some(a) = p1 {
                            *bi.entry(a as u64 * v + w as u64).or_insert(0) += weight;
                            if let Some(z) = p2 {
                                *tri.entry((z as u64 * v + a as u64) * v + w as u64).or_insert(0) += weight;
                            }
                        }
                        p2 = p1;
                        p1 = Some(w);
                    }
                    None => {
                        p2 = None;
                        p1 = None;
                    }
                }
                let trail_end = raw.bytes().rev().take_while(|c| !c.is_ascii_alphabetic()).any(|c| b".!?;:()\"".contains(&c));
                if trail_end {
                    p2 = None;
                    p1 = None;
                }
            }
        }
        eprintln!("{path}: {tokens} tokens so far, {} bigrams, {} trigrams", bi.len(), tri.len());
    }
    if let Some(path) = caps_out {
        let mut out = String::from("# Words written capitalised mid-sentence >= 90% of the time (count >= 20).\n# word share-capitalised\n");
        for id in 0..lx.len() {
            let (c, l) = (cap[id] as f64, low[id] as f64);
            if c + l >= 20.0 && c / (c + l) >= 0.9 && lx.word(id as u32) != "i" && !lx.word(id as u32).starts_with("i'") {
                out.push_str(&format!("{} {:.3}\n", lx.word(id as u32), c / (c + l)));
            }
        }
        std::fs::write(&path, out).unwrap();
        eprintln!("wrote {path}");
    }
    let counts = NgramCounts {
        unigrams: uni,
        bigrams: bi.into_iter().filter(|&(_, c)| c >= min2).collect(),
        trigrams: tri.into_iter().filter(|&(_, c)| c >= min3).collect(),
    };
    let bytes = write_ngrams(&lx, &counts);
    std::fs::File::create(out).unwrap().write_all(&bytes).unwrap();
    eprintln!(
        "kept {} bigrams (>= {min2}), {} trigrams (>= {min3}); wrote {} ({:.1} MB)",
        counts.bigrams.len(),
        counts.trigrams.len(),
        out,
        bytes.len() as f64 / 1e6
    );
}
