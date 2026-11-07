# QuipKeys

**Type approximately, get exactly what you meant.**

QuipKeys is an offline, probabilistic typing decoder for physical keyboards.
It isn't a spellchecker. It asks *which words did your fingers mean to type?*
To answer, it uses key geometry, keystroke timing, word frequencies and the
surrounding words. It only changes text when it is confident.

This repo is the **decoder-first validation** from the plan: a small Rust
engine plus a benchmark harness. There is no OS integration yet.

```
raw keys + timing
   │
   ▼  search.rs   geometry-weighted Damerau-Levenshtein over a dictionary trie
candidates ─────── (adjacent keys cheap, far keys expensive, fast roll-over swaps cheap)
   │
   ▼  decoder.rs  noisy channel: error cost + 3-word context (left; both sides on look-back)
ranked readings ── incl. "keep it as typed" and run-together words ("nicetomeetyou")
   │
   ▼  confidence gate (posterior probability vs. threshold)
   │
   ▼  stream.rs   per-word decisions at space/punctuation, two look-backs,
corrected text ─── word-boundary repair ("hellon ice", "mee tyou", "meetin g"), hysteresis
```

## Try it

```sh
cargo build --release
echo "Hellon ice to mee tyou i hopw ypur doimg wel" | ./target/release/qk --explain
./target/release/qk --why to gp to        # top readings for "gp" between "to" … "to"
./target/release/qk-bench --level all     # accuracy / false corrections / latency / eff-WPM
cargo test --release
```

## Running it on your Mac

```sh
# one-time setup
xcode-select --install                                         # Apple's command-line tools (git, compiler)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh  # Rust; accept the defaults, then open a new terminal
git clone https://github.com/codinghasnat/quipkeys && cd quipkeys
git checkout ccr-9d6287c2-w4on6w                                # the branch with all of this work

cargo test --release                       # behaviour tests (about 1 minute the first time, mostly compiling)
cargo run --release --bin qk-bench -- --level all              # the scorecard (about 10–20 minutes)
cargo run --release --bin qk-bench -- --level blind --sentences 300   # a quick 1-minute check
echo "I wamt to gp to the stpre" | cargo run --release --bin qk -- --explain
scripts/trainer.sh                         # the typing trainer at http://localhost:8000
```

Works the same on Apple Silicon and Intel Macs. Numbers will differ slightly from `RESULTS.md`
(different CPU), but accuracy figures are deterministic.

## Background app (Windows + macOS)

`app/` holds a background app that corrects as you type in any program
(toggle: Ctrl+Alt+Q / Ctrl+Option+Q; Backspace right after a fix undoes it).
Build it with `cd app && cargo build --release`; see [app/README.md](app/README.md).

## Typing trainer

A practice page that runs the real engine in your browser (WebAssembly, offline):

```sh
scripts/trainer.sh        # builds trainer/quipkeys.wasm, serves http://localhost:8000
```

* **QuipKeys on / Raw:** type with live correction, or with none.
* **Every run is scored both ways:** the same keystrokes replayed with and without QuipKeys, so you
  can see what it's worth to you.
* **Effective WPM** counts only words that end up exactly right, the metric that matters.
  **Raw WPM** counts keys.
* **No-backspace mode:** practise the north star, typing through mistakes without looking.
* **Progress chart and Export:** the export is a JSON of every keystroke with timing, which is the
  real-typing data the engine still lacks, for tuning QuipKeys to your fingers.

## What it handles (from the typo taxonomy)

| # | Type | Example | Status |
|---|------|---------|--------|
| 1 | Adjacent keys | Hwllo → Hello | ✅ |
| 2 | Extra characters | Helllo → Hello | ✅ |
| 3 | Missing characters | Helo → Hello | ✅ with context (alone, "Help" wins) |
| 4 | Swapped characters | Hlelo → Hello | ✅ (timing makes roll-over swaps cheaper) |
| 5 | Wrong spacing | Hellon ice → Hello nice | ✅ |
| 6 | Missing spacing | thecat → the cat | ✅ two words; ❌ 3+ (`Nicetomeetyou`) not yet |
| 7 | Premature spacing | Meetin g → Meeting | ✅ |
| 8 | Delayed spacing | Mee tyou → Meet you | ✅ |
| 9 | Several at once | Hellon ice to mee tyou | ✅ |
| 10 | Repeated keys | I amm → I am | ✅ |
| 17 | Hand off by one key | Gello → Hello; `jr;;p` → hello | ✅ |
| 18 | Long messy runs | I wamt tp gp tp thr stpre | ✅ |
| 14 | Missing apostrophes | Dont / im → Don't / I'm | ✅ (table of unambiguous forms; no comma insertion) |
| 11 | Spelling by sound | fone → phone, nollij → knowledge, edukashun → education | ✅ sounds-like candidates, chosen by sentence context; bolder with **Spell by sound** (see below) |
| 12 | Homophones | my sun is ill → my son is ill | ✅ partly: only when the sentence clearly says so (real-word bar) |
| 13, 15, 16 | Casing, wrong word, missing word | | later |

### Spelling by sound (dyslexia-friendly)

Each dictionary word carries its pronunciation, from the CMU Pronouncing Dictionary
(`data/en_phon.txt`). A typed word that isn't in the dictionary is read with
plain letter-to-sound rules ("ph" is f, "c" before e/i is s or k, "tion" is
"shun", a final e is silent, and so on). Words that sound close become
candidates next to the keyboard-slip ones, and the whole-sentence decoder picks
between them using context. A real word can also stand for an exact homophone
(sun/son, our/hour), but only when context clears the real-word confidence bar.

It's a toggle, off by default. With it off, QuipKeys corrects exactly as
before. **Spell by sound**, in the welcome window or the menu, is for people
who often spell by ear, for example with dyslexia. The costs of sound swaps
(which sounds people write in place of which) are learned from real
misspellings by `examples/phon_learn.rs` (`data/phon_costs.txt`). Code: `src/phonetic.rs`; evaluation:
`examples/phon_eval.rs` (words alone) and `examples/phon_ctx.rs` (in sentences).
Results are in RESULTS.md.

Never touched: code-like tokens (`snake_case`, `fooBar`, `src/main.rs`, `--force`, URLs, e-mails), anything
with digits (`hunter2`), ALL-CAPS, capitalised words mid-sentence (names), dialect elisions (`countin'`),
abbreviations (`Mr.`), and any spelling the user has undone once in the session. Unknown words such as
`npm`, `kubectl` or `Hasnat` are kept unless a correction is clearly more likely.

## Behaviour rules

* **When:** a word is decided the moment you press space or punctuation, using the word before it.
* **One look-back:** once the next word arrives, the previous word gets one more chance with
  right-hand context, but only if it was left untouched. A real word needs ≥95% confidence for this.
* **Boundary repair:** at each space the last two chunks are also re-read with the space moved or removed.
  This only happens when one of them isn't a word, and only if every resulting piece is a real word.
* **Hysteresis:** each word changes at most once. The only exception is a boundary repair, which may
  override an earlier spelling fix, and also happens at most once.
* **Undo:** `Stream::reject` restores the typed word, protects it, and never "corrects" that spelling
  again in the session. `Stream::enabled = false` is the off switch (per app, later).

## Benchmark

`qk-bench` types the Brown corpus sentences (`data/eval`) sloppily with a simulated typist. The errors
are adjacent-key slips, roll-over swaps with short inter-key gaps, dropped/doubled letters,
fat-finger double presses and space errors. It then measures:

* **fixed / wrong / left**: what happened to mistyped words.
* **falsecorr**: correctly typed words the engine changed. This is the number that matters most.
* **eff-WPM**: a model of an 80 WPM typist who notices and retypes every wrong word.
  It's an estimate, not a user study.
* **latency**: per word, p50/p99.

It also runs ablations (`plain` = a classic frequency-only spellchecker with no geometry, timing or
context; `no-geometry`, `no-context`, `no-right-ctx`, `no-timing`) to show what each part contributes.
Parameters are tuned on `dev.txt` (`qk-bench --tune`); report on the held-out `test.txt`.

See [RESULTS.md](RESULTS.md) for the current numbers.

### Honest caveats

* **The typist is simulated.** No real keystroke logs were available. The Aalto 136M-keystroke
  dataset would be the right next step, but it couldn't be downloaded here. The timing signal in
  particular is synthetic, so its measured benefit is only as good as the simulation.
* The "native autocorrect" comparison (macOS/Windows) can't be run headless. `plain` stands in as the
  classic-spellchecker baseline.
* Brown is 1960s American prose: lots of rare words and names, which is a tough test for false corrections.

## Data

| File | Source |
|------|--------|
| `data/en_unigrams.txt`, `data/en_bigrams.txt` | SymSpell frequency dictionaries (Google Books Ngram derived), via the `symspellpy` package (MIT) |
| `data/en_wordfreq.txt` | `wordfreq` 3.1.1 English (American spellings, "oh", "ok", contraction counts), filtered against likely misspellings. Built by `scripts/build_wordfreq_supplement.py` |
| `data/en_supplement.txt` | manual overrides |
| `data/eval/{dev,test}.txt` | Brown corpus sentences (NLTK distribution), built by `scripts/prepare_eval.py` |
| `data/en_phon.txt` | CMU Pronouncing Dictionary (BSD-style licence, `data/CMUDICT-LICENSE.txt`), filtered to known words by `scripts/build_phon.py` |
| `data/eval/phonetic.txt`, `phonetic_dev.txt` | sound-based spellings written by hand: `phonetic.txt` was written before the mode was built and is used only for testing; `phonetic_dev.txt` (no shared target words) is for tuning |

`scripts/fetch_data.sh` re-creates all of them from the original sources.

## Layout

```
src/keyboard.rs   QWERTY geometry
src/lexicon.rs    dictionary, frequencies, bigrams, trie
src/channel.rs    error model (costs of slips, swaps, drops, extra keys; timing)
src/search.rs     weighted edit-distance search over the trie
src/decoder.rs    scoring, segmentation, hand-shift, confidence gate
src/stream.rs     tokenisation/protection, streaming decisions, boundary repair, hysteresis
src/sim.rs        simulated sloppy typist
src/bin/qk.rs     CLI
src/bin/bench.rs  benchmark + tuner
```
