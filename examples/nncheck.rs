//! Check Rust NN inference against PyTorch: prints logits for ids 0..4
//! after the given token sequence.   cargo run --release --example nncheck -- FILE 32769,10,2,5,100
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let nn = quipkeys::nnlm::NnLm::load(&std::fs::read(&a[1]).unwrap()).expect("bad model file");
    let ids: Vec<u16> = a[2].split(',').map(|x| x.parse().unwrap()).collect();
    let mut st = quipkeys::nnlm::NnState::default();
    let t = std::time::Instant::now();
    nn.sync(&mut st, &ids);
    let us = t.elapsed().as_secs_f64() * 1e6 / ids.len() as f64;
    let l: Vec<String> = (0..5).map(|i| format!("{:.4}", nn.logit(&st, i))).collect();
    println!("rust logits for ids 0..4: {}   ({us:.0} µs/token)", l.join(" "));
}
