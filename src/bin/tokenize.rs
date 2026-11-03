//! qk-tokenize: turn plain text into word-id streams for neural LM training,
//! using exactly the engine's dictionary and tokenisation.
//!
//!   qk-tokenize OUT.u16 VOCAB FILE [FILE ...]
//!
//! Ids: 0..VOCAB-1 are the VOCAB most frequent dictionary words (lexicon ids
//! are frequency ranks), VOCAB = unknown word, VOCAB+1 = sentence boundary.
//! Output is little-endian u16, so VOCAB must be <= 65534.

use std::io::{BufRead, BufReader, BufWriter, Write};

use quipkeys::lexicon::Lexicon;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = &args[1];
    let vocab: u32 = args[2].parse().unwrap();
    assert!(vocab <= 65534);
    let (unk, bos) = (vocab as u16, vocab as u16 + 1);
    let lx = Lexicon::english_without_ngrams();
    let mut w = BufWriter::with_capacity(1 << 22, std::fs::File::create(out).unwrap());
    let mut n = 0u64;
    let mut n_unk = 0u64;
    for path in &args[3..] {
        let f = BufReader::with_capacity(1 << 20, std::fs::File::open(path).unwrap());
        for line in f.lines() {
            let Ok(line) = line else { continue };
            w.write_all(&bos.to_le_bytes()).unwrap();
            for raw in line.split_whitespace() {
                let b = raw.as_bytes();
                let id = match (b.iter().position(|c| c.is_ascii_alphabetic()), b.iter().rposition(|c| c.is_ascii_alphabetic())) {
                    (Some(s), Some(e)) => {
                        let core = raw[s..=e].to_ascii_lowercase();
                        lx.id(&core).or_else(|| core.strip_suffix("'s").and_then(|st| lx.id(st)))
                    }
                    _ => None,
                };
                let tok = match id {
                    Some(i) if i < vocab => i as u16,
                    _ => {
                        n_unk += 1;
                        unk
                    }
                };
                w.write_all(&tok.to_le_bytes()).unwrap();
                n += 1;
                if raw.bytes().rev().take_while(|c| !c.is_ascii_alphabetic()).any(|c| b".!?".contains(&c)) {
                    w.write_all(&bos.to_le_bytes()).unwrap();
                }
            }
        }
        eprintln!("{path}: {n} tokens so far ({:.1}% unknown)", 100.0 * n_unk as f64 / n.max(1) as f64);
    }
}
