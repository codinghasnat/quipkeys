// Load the neural LM, time inference, and show next-word preferences.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let nn = quipkeys::nnlm::NnLm::load(&std::fs::read(&a[1]).unwrap()).expect("bad model file");
    let lx = quipkeys::lexicon::Lexicon::english_without_ngrams();
    let id = |w: &str| lx.id(w).map(|i| i as u16).filter(|&i| (i as usize) < nn.vocab - 2).unwrap_or(nn.unk());
    for (ctx, cands) in [("i went to the", "shop ship"), ("thanks for helping me", "out our"), ("we watched an old film and ate", "far fat"), ("it was nice and", "quiet quite")] {
        let mut toks = vec![nn.bos()];
        toks.extend(ctx.split(' ').map(id));
        let mut st = quipkeys::nnlm::NnState::default();
        let t = std::time::Instant::now();
        nn.sync(&mut st, &toks);
        let us = t.elapsed().as_secs_f64() * 1e6 / toks.len() as f64;
        let s: Vec<String> = cands.split(' ').map(|c| format!("{c}={:.2}", nn.logit(&st, id(c)))).collect();
        println!("{ctx:>34} … {}   ({us:.0} µs/token)", s.join("  "));
    }
}
