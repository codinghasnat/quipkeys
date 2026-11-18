//! Find wrecked versions of a sentence (letters hit a neighbouring key) that
//! the engine still decodes word-perfect, and replay hand-made ones ("! typed | target"),
//! printing the word-by-word states. Used for the pitch-video examples.
//!   echo "! gsopu votygssu ti tou | happy birthday to you" | cargo run --release --example extreme
use quipkeys::decoder::{Config, Engine};
use quipkeys::stream::Stream;
const ROWS: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
fn nbrs(c: char) -> Vec<char> {
    let mut out = vec![];
    for (r, row) in ROWS.iter().enumerate() { if let Some(i) = row.find(c) {
        let row: Vec<char> = row.chars().collect();
        if i > 0 { out.push(row[i-1]); } if i + 1 < row.len() { out.push(row[i+1]); }
        let _ = r;
    }}
    out
}
struct R(u64); impl R { fn f(&mut self) -> f64 { self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407); (self.0 >> 11) as f64 / (1u64<<53) as f64 } }
fn main() {
    let e = Engine::new(Config::default());
    let a: Vec<String> = std::env::args().collect();
    let p: f64 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.4);
    let tries: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(200);
    // warm the typist as sloppy, the way it would be mid-mash
    let mut warm = Stream::new(&e);
    for w in "i rjink ypu arw tight abput rhar".split(' ') { warm.push(w, None); }
    let st = warm.typist();
    for target in std::io::stdin().lines().map(|l| l.unwrap()) {
        if let Some(h) = target.strip_prefix("! ") { // hand-made: "! typed | target"
            let (ty, tg) = h.split_once(" | ").unwrap();
            let mut s = Stream::with_typist(&e, st.clone()); for w in ty.split_whitespace() { s.push(w, None); }
            let mut c = Stream::new(&e); let mut steps = vec![]; for w in ty.split_whitespace() { c.push(w, None); steps.push(c.text()); }
            println!("STEPS {}", steps.join(" || "));
            println!("HAND {} | warm: {} | cold: {} | {}", ty, s.text(), c.text(), if s.text().to_lowercase()==tg.to_lowercase() {"OK"} else {"--"});
            continue;
        }
        let mut r = R(target.len() as u64 * 7919);
        let mut best: Vec<(usize, String, bool)> = vec![];
        for _ in 0..tries {
            let mut wrong = 0; let mut ty = String::new();
            for c in target.chars() {
                let n = nbrs(c);
                if !n.is_empty() && r.f() < p { ty.push(n[(r.f() * n.len() as f64) as usize]); wrong += 1; } else { ty.push(c); }
            }
            let mut s = Stream::with_typist(&e, st.clone()); for w in ty.split_whitespace() { s.push(w, None); }
            if s.text().to_lowercase() == target.to_lowercase() {
                let mut c = Stream::new(&e); for w in ty.split_whitespace() { c.push(w, None); }
                best.push((wrong, ty, c.text().to_lowercase() == target.to_lowercase()));
            }
        }
        best.sort_by(|a, b| b.0.cmp(&a.0)); best.dedup();
        let letters = target.chars().filter(|c| c.is_alphabetic()).count();
        println!("== {target}  ({} ok of {tries})", best.len());
        for (w, t, cold) in best.iter().take(3) { println!("   {w}/{letters} wrong: {t}   cold-start ok: {cold}"); }
    }
}
