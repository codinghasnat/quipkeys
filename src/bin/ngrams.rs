//! qk-ngrams: count word bigrams, trigrams and 4-grams over plain text and
//! write the compact `data/en_ngrams.bin` that the engine loads.
//!
//!   qk-ngrams OUT.bin [--min2 N] [--min3 N] [--min4 N] [--caps CAPS.txt]
//!             [--spill DIR] WEIGHT:FILE [WEIGHT:FILE ...]
//!
//! Tokenisation mirrors how the engine sees context while you type: words are
//! runs of letters/apostrophes, lowercased; '.', '!', '?' end a sentence; any
//! token that is not a dictionary word (numbers, names the dictionary lacks,
//! code) breaks the context, exactly as an unknown word does in the stream.
//!
//! Scales past RAM: trigram/4-gram tables are spilled to sorted run files in
//! `--spill` whenever they grow large (dropping entries seen once in that
//! chunk, a small approximation), then merged.

use std::collections::{BinaryHeap, HashMap};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};

use quipkeys::lexicon::{write_ngrams, Lexicon, NgramCounts};

const SPILL_AT: usize = 180_000_000;

fn spill(map: &mut HashMap<u64, u32>, dir: &str, tag: &str, n: &mut usize) -> String {
    let mut v: Vec<(u64, u32)> = map.drain().filter(|&(_, c)| c >= 2).collect();
    v.sort_unstable();
    let path = format!("{dir}/{tag}-{n:03}.run");
    *n += 1;
    let mut w = BufWriter::with_capacity(1 << 22, std::fs::File::create(&path).unwrap());
    for (k, c) in &v {
        w.write_all(&k.to_le_bytes()).unwrap();
        w.write_all(&c.to_le_bytes()).unwrap();
    }
    eprintln!("  spilled {} {tag} entries -> {path}", v.len());
    map.shrink_to_fit();
    path
}

struct Run {
    r: BufReader<std::fs::File>,
}

impl Run {
    fn next(&mut self) -> Option<(u64, u32)> {
        let mut b = [0u8; 12];
        self.r.read_exact(&mut b).ok()?;
        Some((u64::from_le_bytes(b[..8].try_into().unwrap()), u32::from_le_bytes(b[8..].try_into().unwrap())))
    }
}

/// k-way merge of sorted runs plus the in-memory remainder; keep sums >= min.
fn merge(runs: &[String], rest: HashMap<u64, u32>, min: u32) -> HashMap<u64, u32> {
    let mut rest: Vec<(u64, u32)> = rest.into_iter().collect();
    rest.sort_unstable();
    let mut readers: Vec<Run> =
        runs.iter().map(|p| Run { r: BufReader::with_capacity(1 << 22, std::fs::File::open(p).unwrap()) }).collect();
    let mut heap = BinaryHeap::new();
    for (i, r) in readers.iter_mut().enumerate() {
        if let Some((k, c)) = r.next() {
            heap.push(std::cmp::Reverse((k, c, i)));
        }
    }
    let rest_idx = readers.len();
    let mut rest_it = rest.into_iter();
    if let Some((k, c)) = rest_it.next() {
        heap.push(std::cmp::Reverse((k, c, rest_idx)));
    }
    let mut out = HashMap::new();
    let mut cur: Option<(u64, u32)> = None;
    while let Some(std::cmp::Reverse((k, c, i))) = heap.pop() {
        let nx = if i == rest_idx { rest_it.next() } else { readers[i].next() };
        if let Some((k2, c2)) = nx {
            heap.push(std::cmp::Reverse((k2, c2, i)));
        }
        match &mut cur {
            Some((ck, cc)) if *ck == k => *cc += c,
            _ => {
                if let Some((ck, cc)) = cur {
                    if cc >= min {
                        out.insert(ck, cc);
                    }
                }
                cur = Some((k, c));
            }
        }
    }
    if let Some((ck, cc)) = cur {
        if cc >= min {
            out.insert(ck, cc);
        }
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let out = &args[1];
    let (mut min2, mut min3, mut min4) = (2u32, 3u32, 0u32);
    let mut caps_out: Option<String> = None;
    let mut spill_dir = std::env::temp_dir().to_string_lossy().to_string();
    let mut inputs = Vec::new();
    let mut i = 2;
    while i < args.len() {
        let next = || args[i + 1].clone();
        match args[i].as_str() {
            "--min2" => min2 = next().parse().unwrap(),
            "--min3" => min3 = next().parse().unwrap(),
            "--min4" => min4 = next().parse().unwrap(),
            "--caps" => caps_out = Some(next()),
            "--spill" => spill_dir = next(),
            a => {
                let (w, f) = a.split_once(':').expect("WEIGHT:FILE");
                inputs.push((w.parse::<u32>().unwrap(), f.to_string()));
                i += 1;
                continue;
            }
        }
        i += 2;
    }
    let lx = Lexicon::english_without_ngrams();
    let mut uni: Vec<u64> = vec![0; lx.len()];
    let mut bi: HashMap<u64, u32> = HashMap::new();
    let mut tri: HashMap<u64, u32> = HashMap::new();
    let mut four: HashMap<u64, u32> = HashMap::new();
    let (mut tri_runs, mut four_runs, mut nspill) = (Vec::new(), Vec::new(), 0usize);
    let v = lx.len() as u64;
    let mut tokens = 0u64;
    // Mid-sentence capitalised vs lowercase occurrences, per lexicon word.
    let mut cap: Vec<u32> = vec![0; lx.len()];
    let mut low: Vec<u32> = vec![0; lx.len()];
    for (weight, path) in &inputs {
        let f = BufReader::with_capacity(1 << 20, std::fs::File::open(path).expect("open input"));
        for line in f.lines() {
            let Ok(line) = line else { continue };
            let (mut p3, mut p2, mut p1): (Option<u32>, Option<u32>, Option<u32>) = (None, None, None);
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
                                // 4-grams over the 65,536 most frequent words (16-bit ids).
                                if let Some(y) = p3 {
                                    if min4 > 0 && y < 65536 && z < 65536 && a < 65536 && w < 65536 {
                                        let k = (y as u64) << 48 | (z as u64) << 32 | (a as u64) << 16 | w as u64;
                                        *four.entry(k).or_insert(0) += weight;
                                    }
                                }
                            }
                        }
                        p3 = p2;
                        p2 = p1;
                        p1 = Some(w);
                    }
                    None => {
                        (p3, p2, p1) = (None, None, None);
                    }
                }
                let trail_end = raw.bytes().rev().take_while(|c| !c.is_ascii_alphabetic()).any(|c| b".!?;:()\"".contains(&c));
                if trail_end {
                    (p3, p2, p1) = (None, None, None);
                }
            }
            if tri.len() + four.len() > SPILL_AT {
                tri_runs.push(spill(&mut tri, &spill_dir, "tri", &mut nspill));
                four_runs.push(spill(&mut four, &spill_dir, "four", &mut nspill));
            }
        }
        eprintln!("{path}: {tokens} tokens so far, {} bi / {} tri / {} four in memory", bi.len(), tri.len(), four.len());
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
    eprintln!("merging {} trigram runs, {} 4-gram runs", tri_runs.len(), four_runs.len());
    let trigrams = merge(&tri_runs, tri, min3);
    let fourgrams = if min4 > 0 { merge(&four_runs, four, min4) } else { HashMap::new() };
    for p in tri_runs.iter().chain(&four_runs) {
        let _ = std::fs::remove_file(p);
    }
    let counts = NgramCounts {
        unigrams: uni,
        bigrams: bi.into_iter().filter(|&(_, c)| c >= min2).collect(),
        trigrams,
        fourgrams,
    };
    let bytes = write_ngrams(&lx, &counts);
    std::fs::File::create(out).unwrap().write_all(&bytes).unwrap();
    eprintln!(
        "kept {} bigrams (>= {min2}), {} trigrams (>= {min3}), {} 4-grams (>= {min4}); wrote {} ({:.1} MB)",
        counts.bigrams.len(),
        counts.trigrams.len(),
        counts.fourgrams.len(),
        out,
        bytes.len() as f64 / 1e6
    );
}
