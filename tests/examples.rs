//! Behavioural tests: the examples QuipKeys must get right, and the things
//! it must never touch.

use quipkeys::{Config, Engine};
use std::sync::OnceLock;

fn engine() -> &'static Engine {
    static E: OnceLock<Engine> = OnceLock::new();
    E.get_or_init(|| Engine::new(Config::default()))
}

#[track_caller]
fn check(typed: &str, want: &str) {
    assert_eq!(engine().correct_line(typed), want, "typed: {typed}");
}

#[test]
fn design_doc_examples() {
    check("I wamt to gp to the stpre", "I want to go to the store");
    check("Thsi is a relaly goos idea", "This is a really good idea");
    check("I habe a meetimg tomorroe", "I have a meeting tomorrow");
}

#[test]
fn clean_text_is_untouched() {
    for s in [
        "I want to go to the store",
        "The quick brown fox jumps over the lazy dog.",
        "We met Siobhan and Okonkwo at the station.",
        "Run cargo build --release then open src/main.rs",
        "Set MAX_RETRIES to 5 and call fetchUserData() again",
        "Email me at someone@example.com or see https://example.com/docs",
        "The NASA and ESA teams use kubectl and nginx daily.",
        "Rust npm Hasnat hunter2 kubectl",
        "Mr. Smith's driver's license, the color of the theater.",
    ] {
        check(s, s);
    }
}

#[test]
fn punctuation_and_case_are_preserved() {
    check("Thsi, relaly.", "This, really.");
    check("\"Wamt\" it?", "\"Want\" it?");
}

#[test]
fn taxonomy_core_categories() {
    for (typed, want) in [
        ("Hwllo", "Hello"),                                       // 1 adjacent key
        ("Helllo", "Hello"),                                      // 2 extra character
        ("Hlelo", "Hello"),                                       // 4 swapped
        ("Hellon ice", "Hello nice"),                             // 5 wrong spacing
        ("Meetin g", "Meeting"),                                  // 7 premature space
        ("Mee tyou", "Meet you"),                                 // 8 delayed space
        ("Hellon ice to mee tyou", "Hello nice to meet you"),     // 9 multiple
        ("I amm happy", "I am happy"),                            // 10 repeated key
        ("Gello", "Hello"),                                       // 17 one key off
        ("Jr;;p there", "Hello there"),                           // 17 whole hand shifted
        ("I wamt tp gp tp thr stpre", "I want to go to the store"), // 18
        ("Dont worry im here", "Don't worry I'm here"),
    ] {
        check(typed, want);
    }
}

#[test]
fn undo_protects_a_word() {
    let e = engine();
    let mut s = quipkeys::Stream::new(e);
    s.push("I", None);
    let edits = s.push("wamt", None);
    assert_eq!(edits[0].new, "want");
    s.reject(1);
    assert_eq!(s.text(), "I wamt");
    s.push("to", None);
    // Never "corrected" again in this session.
    let ed = s.push("wamt", None);
    assert!(ed.iter().all(|e| e.index != 3), "{ed:?}");
}

#[test]
fn learns_names_typed_twice() {
    // Once is not enough (could be a typo); twice means it's a word.
    check("We met Hasnat today and then Hasant said hello", "We met Hasnat today and then Hasant said hello");
    check(
        "We met Hasnat today and Hasnat was great so then Hasant said hello",
        "We met Hasnat today and Hasnat was great so then Hasnat said hello",
    );
    check("the kubectl tool and the kubectl config then the kubetcl apply", "the kubectl tool and the kubectl config then the kubectl apply");
}

#[test]
fn proper_nouns() {
    check("I went to Calicornia last Monady", "I went to California last Monday");
    check("i love london", "I love London");
    check("We met Colman and Barcus", "We met Colman and Barcus");
}

#[test]
fn common_misspellings_are_not_dictionary_words() {
    check("will actyaly have meaningbful improvements", "will actually have meaningful improvements");
    check("I allready told you, it is realy excelent", "I already told you, it is really excellent");
}

#[test]
fn accidental_caps_lock() {
    check("w NEED TO FINSIH THIS VEFORE THE END", "we need to finish this before the end");
    check("The NASA team met at JPL today", "The NASA team met at JPL today");
}
