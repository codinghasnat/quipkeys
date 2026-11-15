// Print the wordlikeness score of each argument (see Engine::wordlikeness).
fn main() {
    let e = quipkeys::Engine::new(quipkeys::Config::default());
    for w in std::env::args().skip(1) {
        print!("{w}:{:.1} ", e.wordlikeness(&w));
    }
    println!();
}
