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
   ▼  decoder.rs  noisy channel: error cost + bigram context (left, and right on revision)
ranked readings ── incl. "keep it as typed" and run-together words ("nicetomeetyou")
   │
   ▼  confidence gate (posterior probability vs. threshold)
   │
   ▼  stream.rs   per-word decisions at space/punctuation, one look-back revision,
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

## What it handles (from the typo taxonomy)

| # | Type | Example | Status |
|---|------|---------|--------|
| 1 | Adjacent keys | Hwllo → Hello | ✅ |
| 2 | Extra characters | Helllo → Hello | ✅ |
| 3 | Missing characters | Helo → Hello | ✅ with context (alone, "Help" wins) |
| 4 | Swapped characters | Hlelo → Hello | ✅ (timing makes roll-over swaps cheaper) |
| 5 | Wrong spacing | Hellon ice → Hello nice | ✅ |
| 6 | Missing spacing | thecat, Nicetomeetyou | ✅ two words; longer runs depend on tuning |
| 7 | Premature spacing | Meetin g → Meeting | ✅ |
| 8 | Delayed spacing | Mee tyou → Meet you | ✅ |
| 9 | Several at once | Hellon ice to mee tyou | ✅ |
| 10 | Repeated keys | I amm → I am | ✅ |
| 17 | Hand off by one key | Gello → Hello; `jr;;p` → hello | ✅ |
| 18 | Long messy runs | I wamt tp gp tp thr stpre | ✅ |
| 14 | Missing apostrophes | Dont / im → Don't / I'm | ✅ partial (no comma insertion) |
| 11–13, 15, 16 | Phonetic, homophones, casing, wrong word, missing word | | later (needs more than bigrams) |

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
