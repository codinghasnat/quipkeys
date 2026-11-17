// Show the next guesses for the last word of each argument sentence.
fn main() {
    let e = quipkeys::Engine::new(quipkeys::Config::default());
    for s in std::env::args().skip(1) {
        let mut st = quipkeys::Stream::new(&e);
        for w in s.split_whitespace() {
            st.push(w, None);
        }
        let last = st.words.len() - 1;
        println!("{s:?} -> shown {:?}, next: {:?}", st.words[last].output, st.alternatives(last, 4));
    }
}
