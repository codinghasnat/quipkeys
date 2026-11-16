// Show how a typed word reads by sound, and the closest-sounding words.
fn main() {
    let e = quipkeys::Engine::new(quipkeys::Config { phonetic: true, ..Default::default() });
    for w in std::env::args().skip(1) {
        let codes: Vec<String> = quipkeys::phonetic::spell_codes(&w).into_iter().map(|c| String::from_utf8(c).unwrap()).collect();
        let mut c = e.phonetic().candidates(&w, 12.0);
        c.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        let top: Vec<String> = c.iter().take(6).map(|(id, d)| format!("{}:{d:.1}", e.lx.word(*id))).collect();
        println!("{w} {codes:?} -> {}", top.join(" "));
    }
}
