//! Real-word slip test: data/eval/realword.txt ("typed<TAB>meant").
//! Counts slips fixed, slips missed, and words wrongly changed (in the slip
//! sentences and in the correct ones). QK_RW=0 turns the scorer off;
//! QK_RW_TAU / QK_RW_KEEP tune it. QK_SHOW=1 prints every sentence.
use quipkeys::{Config, Engine, Stream};

fn words(s: &str) -> Vec<String> {
    s.split_whitespace().map(String::from).collect()
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "data/eval/realword.txt".into());
    let e = Engine::new(Config::default());
    let show = std::env::var_os("QK_SHOW").is_some();
    let (mut slips, mut fixed, mut wrong, mut clean_words, mut clean_changed, mut slip_sent_changed) = (0, 0, 0, 0, 0, 0);
    let (mut sents_ok, mut sents) = (0, 0);
    for line in std::fs::read_to_string(path).unwrap().lines().filter(|l| !l.starts_with('#') && l.contains('\t')) {
        let (typed, meant) = line.split_once('\t').unwrap();
        let mut s = Stream::new(&e);
        for w in typed.split_whitespace() {
            s.push(w, None);
        }
        let out = s.text();
        let (t, m, o) = (words(typed), words(meant), words(&out));
        sents += 1;
        if out.trim() == meant.trim() {
            sents_ok += 1;
        }
        if t.len() != m.len() || o.len() != m.len() {
            println!("(word count changed) {typed} => {out}");
            continue;
        }
        for k in 0..t.len() {
            if t[k] != m[k] {
                slips += 1;
                if o[k] == m[k] {
                    fixed += 1;
                } else if o[k] != t[k] {
                    wrong += 1;
                }
            } else {
                clean_words += 1;
                if o[k] != t[k] {
                    clean_changed += 1;
                    if typed != meant {
                        slip_sent_changed += 1;
                    }
                }
            }
        }
        if show || out.trim() != meant.trim() {
            println!("{} {typed}\n   => {out}", if out.trim() == meant.trim() { "ok " } else { "-- " });
        }
    }
    println!(
        "slips fixed {fixed}/{slips}, changed to something else wrong {wrong}; correct words changed {clean_changed}/{clean_words} ({slip_sent_changed} of them in slip sentences); sentences right {sents_ok}/{sents}"
    );
}
