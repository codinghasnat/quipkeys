# Results (held-out Brown test split, simulated typist)

Command: `./target/release/qk-bench --data data/eval/test.txt --level all`
(1,500 sentences, about 24k words per level). All parameters are tuned on `dev.txt` only.
Run on a 4 vCPU cloud container, release build.

## Scorecard

| | Round 1 | Round 2 | **Round 3** |
|---|---|---|---|
| **Moderate** (3% of letters wrong): typos fixed | 66.9% | 73.6% | **75.6%** |
| real-word slips fixed (form→from) | 7.7% | 18.8% | **19.5%** |
| spacing errors fixed | 60.7% | 67.5% | **72.0%** |
| correct words wrongly changed | 0.112% | 0.122% | 0.122% |
| word accuracy (no correction: 85.9%) | 95.4% | 96.2% | **96.5%** |
| **Blind** (10% of letters wrong, the north star): typos fixed | 61.9%* | 67.8% | **69.8%** |
| word accuracy (no correction: 61.7%) | 85.4%* | 87.6% | **88.4%** |
| correct words wrongly changed | 0.12%* | 0.111% | **0.104%** |
| latency p50 / p99 per word | 0.2 / 2.7 ms | 0.4–0.7 / 3.6–4.2 ms | 0.5–1.0 / 3.6–5.2 ms |

\* Blind was added in round 2; round-1 numbers are from the round-1 engine on the dev split.

Real-word changes are deliberately conservative (98% confidence plus an extra cost). A looser setting fixes
more real-word slips on this benchmark, but in everyday sentences it turned a correctly typed "ate" into
"are". That trade isn't worth it.

What changed in round 3:
* **Whole-sentence decoder:** a beam search over the last four typed chunks. Each chunk has readings:
  keep, spelling candidates, splits, or learned words. Each adjacent pair adds merge and moved-space
  readings. Neighbouring uncertain words are decided together, with three-word context along the path.
  Confidence is the posterior probability of the new text, summed over every path that produces it.
  Against the old word-by-word pipeline it fixes 0.4–1.0 points more and makes 10–20% fewer wrong
  corrections.
* **Capitalisation knowledge** from Wikipedia casing: `Calicornia`→California, `Monady`→Monday,
  `london`→London. Unknown names stay protected (`Colman`, `Barcus` and `Hasnat` are never "fixed").
* **Document memory:** a word typed the same way twice (or restored with undo) is learned. It's protected
  from then on, and later typos of it are fixed (`kubetcl`→kubectl, `Hasant`→Hasnat). On the Brown
  document test this adds only about 0.1–0.3 points, because the simulated typist rarely mangles repeated
  names. Its value is for your own names and jargon. A plain word-frequency cache was tried and dropped:
  it reinforced uncorrected slips.

What changed in round 2:
* **Three-word context:** 115M words of Wikipedia plus conversational text (2.7M pairs, 4.3M triples).
  Used on the left of each word, and on both sides when revising.
* **Second look-back:** once two more words are typed, an untouched word gets a final check with two words
  of context on each side.
* **Wider search for long words:** the radius grows with length (capped), so 3-slip words get found.
* **Shift-key slips** (`THe`, `hTe`, `uSre`→Sure, `uJnior`→Junior), **hyphenated words** (`floor-to-ceilnig`),
  **stray letters** (`t`, `n`), and **run-together words with a typo inside** (`subjectiveexpreience`).
* **Safety:** inflections of known words (`freewheelers`, `banquetings`) count as real words; mid-sentence
  capitalised words (likely names) need 99.9% confidence and never take part in space repair.

## What each part contributes (fix rate, moderate / blind, round 3)

| Removed | Moderate | Blind |
|---|---|---|
| nothing (full) | 75.6% | 69.8% |
| sentence decoder (word-by-word instead) | 75.2% (−0.4) | 68.8% (−1.0) |
| keyboard geometry | 70.9% (−4.7) | 63.5% (−6.3) |
| trigrams (pairs only) | 73.2% (−2.4) | 67.8% (−2.0) |
| classic spellchecker (`plain`) | 53.2% | 46.2% |

## Full output

```
== light typing: 1.5% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   92.72%    65.3      -      -
full            77.5%    2.0%   20.5%     14.9%    0.126%   98.24%    76.1    512   3609
              (1739 mistyped units: 1598 non-words, 141 real words, 139 spacing; 22142 clean; fix rate: non-word 83.0%, spacing 71.9%)
plain           55.0%    2.5%   42.5%      0.0%    0.095%   96.63%    73.3    295   2847
no-geometry     72.8%    2.5%   24.7%     12.1%    0.126%   97.90%    75.4    561   3901
no-trigram      75.1%    1.9%   23.0%     10.6%    0.126%   98.07%    75.8    446   3549
word-by-word    76.8%    2.3%   20.9%     14.9%    0.135%   98.19%    75.9    447   3903
== moderate typing: 3.0% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   85.92%    55.8      -      -
full            75.6%    2.5%   21.9%     19.5%    0.122%   96.46%    72.5    579   4017
              (3353 mistyped units: 3076 non-words, 277 real words, 243 spacing; 20457 clean; fix rate: non-word 80.7%, spacing 72.0%)
plain           53.2%    3.2%   43.6%      0.0%    0.078%   93.35%    67.7    330   3089
no-geometry     70.9%    3.0%   26.1%     17.0%    0.122%   95.80%    71.2    512   3817
no-trigram      73.2%    2.6%   24.2%     10.1%    0.117%   96.13%    72.0    453   3626
word-by-word    75.2%    2.7%   22.1%     18.8%    0.142%   96.39%    72.3    461   3950
== messy typing: 6.0% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   73.98%    44.6      -      -
full            74.5%    3.3%   22.3%     18.8%    0.109%   93.28%    67.1    709   4316
              (6128 mistyped units: 5650 non-words, 478 real words, 558 spacing; 17421 clean; fix rate: non-word 79.2%, spacing 70.1%)
plain           51.0%    3.8%   45.2%      0.6%    0.080%   87.19%    59.5    608   3959
no-geometry     69.3%    3.7%   27.0%     14.4%    0.103%   91.93%    65.1    789   4887
no-trigram      72.1%    3.1%   24.8%      9.6%    0.115%   92.65%    66.4    745   4810
word-by-word    73.5%    3.5%   22.9%     17.6%    0.126%   93.01%    66.6    739   5307
== blind typing: 10.0% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   61.74%    37.4      -      -
full            69.8%    4.0%   26.2%     16.9%    0.104%   88.39%    60.2   1028   5247
              (8927 mistyped units: 8193 non-words, 734 real words, 839 spacing; 14404 clean; fix rate: non-word 74.6%, spacing 62.2%)
plain           46.2%    4.8%   49.0%      0.7%    0.076%   79.36%    51.3    903   4285
no-geometry     63.5%    4.8%   31.6%     13.4%    0.097%   85.98%    57.3    972   5157
no-trigram      67.8%    3.8%   28.4%     10.6%    0.111%   87.60%    59.5    946   4888
word-by-word    68.8%    4.3%   26.9%     14.9%    0.125%   87.98%    59.7    854   5216
```

## Spelling by sound (held-out results)

Sounds-like matching (`src/phonetic.rs`, see the README) is a **toggle, off by default**. Off, QuipKeys runs
exactly the decoder measured above. On ("Spell by sound"), words spelt by ear join the candidates and
sentence context picks.

How sound swaps are costed: costs are learned from real misspellings (`examples/phon_learn.rs` on the codespell
tuning half plus `phonetic_dev.txt`). Each misspelling is aligned in sound space, only pairs that already sound
close are kept, and four rounds of align, count and re-estimate are run with the hand-set costs as a prior.
One cost is set by hand: b/d reversal is kept cheap (2.5). It's a well-known dyslexic pattern that the data
barely contains, and it isn't measured here. Typed words of 3 letters or fewer only take exact sound matches.
Settings were tuned on tuning data against both goals at once (sound spellings in sentences, and the keyboard
benchmark).

Held-out results. `phonetic.txt` was written before the mode existed and shares no target words with the
tuning list; it's placed into Brown **test** sentences. codespell is the other half (about 27,000
misspellings), never tuned on.

| | Toggle off | **Toggle on** |
|---|---|---|
| Sound spellings in sentences: fixed | 47.3% | **77.3%** |
| ... wrongly "fixed" | 5.7% | 1.1% |
| ... other words in those sentences changed | 0.11% | 0.11% |
| Sound spellings, words alone: fixed / wrong | 37.7% / 7.9% | 57.0% / 1.3% |
| codespell misspellings, words alone: fixed / wrong | 78.5% / 1.8% | 75.7% / 2.0% |

Effect of switching it on for ordinary keyboard typos (main benchmark, Brown test split, 1,500 sentences
per level):

| Typing level | Word accuracy: off / on | Typos fixed: off / on | Correct words changed: off / on | p99 latency (ms) |
|---|---|---|---|---|
| light 1.5% | 98.34 / 98.17 | 78.8 / 76.6 | 0.131 / 0.135 | 3.0 / 5.0 |
| moderate 3% | 96.70 / 96.38 | 77.3 / 75.0 | 0.122 / 0.117 | 3.6 / 6.1 |
| messy 6% | 93.58 / 92.93 | 75.6 / 73.1 | 0.098 / 0.098 | 4.5 / 7.6 |
| blind 10% | 89.06 / 88.15 | 71.6 / 69.2 | 0.104 / 0.083 | 6.8 / 10.7 |
| rough 4% | 86.59 / 85.40 | 72.9 / 70.5 | 0.135 / 0.101 | 7.8 / 11.7 |

So switching it on costs keyboard typing 0.2 to 1.2 points of word accuracy. An earlier version with
hand-set costs cost 0.3 to 2.4 points for about the same result on sound spellings (78.8%). Correct
words are no more likely to be changed.

Caveats:
* **Not tested on dyslexic writers.** The sound spellings are hand-written stand-ins. Real data (for example,
  a dictation mode in the Typing Lab) is the next step and may change the costs.
* Homophone swaps (sun/son, our/hour) only happen when the sentence clearly calls for one. Most are still
  left alone.

## Spell by sound, second round (held-out results)

Changes after the table above: sound-alike real words swap more readily (homophone cost 4.0 → 1.0), the
letter-to-sound rules read "ew", "aw" and "ow" as vowels, the y-glide is cheap ("nyoomonya" → pneumonia),
and an uncommon real word can be a sound-spelling of one at least 50 times more common ("wether" → weather).
Tuned on dev only; the learned costs were re-fitted with the fixed rules.

A new test, `examples/homophone_eval.rs`: in each Brown **test** sentence, one word is swapped for a common
sound-alike (their/there, no/know, by/buy), and the untouched sentences are run too.

| Toggle on | Before this round | Now |
|---|---|---|
| Sound spellings in sentences: fixed / wrong | 77.3% / 1.1% | **80.7%** / 1.1% |
| Sound spellings, words alone: fixed / wrong | 57.0% / 1.3% | 57.0% / 1.3% |
| Swapped sound-alikes restored (1,328 sentences) | not measured (25.5% with the toggle off) | **82.2%** |
| Clean text: correct words changed (23,983 words) | 0.121% with the toggle off | 0.142% |
| codespell misspellings: fixed / wrong | 75.7% / 2.0% | 75.7% / 2.0% |
| Keyboard typos, word accuracy, moderate / blind | 96.38% / 88.15% | 96.32% / 88.06% |

So sound-alike mix-ups go from mostly ignored to mostly fixed, for about 5 more wrongly changed words per
24,000 words of clean text. All of this is with the toggle on; off, nothing here applies.

## Typing almost blind: neighbour-key reading and typist memory (held-out results)

Two changes aimed at the extreme case, where most letters in a word are one key off
("jrkko how are you" → "hello how are you"):

* **Neighbour-key reading** (`Engine::add_neighbour_readings`): every dictionary word of the same length whose
  letters are each the key typed or one beside it becomes a candidate, and sentence context picks. Few words
  ever fit ("jrkko" fits six, and "hello" is far the most common). It's cheap while the typist is being
  sloppy, or when the typed word isn't word-shaped at all (a letter-trigram score, `Engine::wordlikeness`:
  "jrkko" 5.3; "hasnat" 3.0, "nginx" 4.1, "wamt" 3.6; threshold 5.0). Otherwise it's expensive and needs
  three or more slipped letters, so ordinary typos are still handled by the ordinary search.
* **Typist memory** (`Stream::with_typist`): how sloppy the typist has been now carries over when the text
  resets (Enter, a click, a new sentence). Before, every sentence started by assuming an accurate typist and
  needed eight words to notice otherwise.

A new simulator level, **smash**, has every press land so loosely that about half of all letters hit a
neighbouring key: 17% of words come out right as typed. Costs were tuned on dev. Results on the Brown test
split, 1,500 sentences per level, word accuracy:

| Level | As typed | Before | With typist memory | **+ neighbour reading (now)** |
|---|---|---|---|---|
| smash | 17.2% | 65.8% | 73.1% | **74.7%** |
| rough | 50.7% | 86.6% | 87.7% | **87.8%** |
| blind 10% | 61.7% | 89.06% | 88.93% | 88.93% |
| messy 6% | 74.0% | 93.58% | 93.55% | 93.53% |
| moderate 3% | 85.9% | 96.70% | 96.71% | 96.67% |
| light 1.5% | 92.7% | 98.34% | 98.34% | 98.31% |

So typing with most letters off gains about 9 points and rough typing about 1, while accurate-typist levels
move by 0.03 to 0.13 points down. Typist memory also raises wrongly-changed correct words at the two
sloppiest levels (smash 0.10% → 0.17%, rough 0.14% → 0.18%), since the engine stays in its sloppy mode
longer. Run with `qk-bench --level smash --carry`; `--configs no-neighbour` is the ablation. The scorecard
at the top of this file predates both changes (no `--carry`).

Still out of reach: a mashed word with no context ("jrkko" alone stays as typed), sentences where every
word is mashed, and mashed words that happen to be real words ("us" for "is").

## Learning one typist (measured on real runs)

`src/profile.rs` keeps tallies of a typist's kinds of slips and turns them into channel costs. Tested on
the 50 real runs of one typist, in two sets recorded at different times (33 and 17 runs): learn on one set,
score on the other. Word accuracy after correction:

| Learned from | Test: set 1 (33 runs) | Test: set 2 (17 runs) |
|---|---|---|
| Nothing (default) | 91.3% | 89.0% |
| The correct answers on the other set, all of it (`scripts/real/fit_profile.py`) | 92.5% | 89.0% |
| ... key confusions only | 91.1% | 89.2% |
| ... slip rates only (drop, add, double, swap) | 92.3% | 89.4% |
| **The engine's own fixes on the other set, slip rates only** (`examples/profile_sim.rs`, what the app does) | **92.7%** | **89.4%** |

So the six slip rates help (+1.4 and +0.4 points), and they can be learned without knowing the right
answers. Per-key confusions gave nothing at this amount of data (about 6,000 letters; only one confusion was
seen five times). Caveats: one typist, prompted sentences that repeat across runs, and a run is only 30
words, so each run moves in steps of 3.3 points.

### Simulated typists (`examples/learn_sim.rs`)

Seven simulated typists with different habits each type 150 dev-split sentences a day for five days. As in
the app, the engine learns only from its own kept fixes and correctly typed known words, and each day's
engine is shaped by what was learned up to the day before. Scored on 300 held-out test-split sentences typed
by the same typist. Word accuracy, with the share of correctly typed words changed in brackets:

| Typist | As typed | No profile | After day 1 | After day 5 | Learning from the right answers | Day 5 + key confusions |
|---|---|---|---|---|---|---|
| average | 83.0% | 95.71% (0.10) | 95.89% (0.08) | 95.89% (0.08) | 96.02% | 95.89% |
| accurate (1% of letters wrong) | 95.5% | 98.96% (0.09) | 98.98% (0.05) | 98.98% (0.05) | 98.98% | 98.98% |
| swaps letters | 77.9% | 95.06% (0.06) | 95.28% (0.06) | 95.22% (0.06) | 95.30% | not run |
| drops letters | 78.9% | 92.31% (0.05) | 93.59% (0.05) | 93.51% (0.03) | 93.68% | 93.51% |
| hits extra keys | 79.0% | 95.48% (0.11) | 96.02% (0.14) | 96.12% (0.14) | 96.15% | not run |
| like the real typist above | 75.3% | 93.29% (0.06) | 93.79% (0.06) | 93.83% (0.06) | 93.81% | 93.81% |
| habitual wrong keys | 74.4% | 94.35% (0.06) | 94.41% (0.06) | 94.35% (0.06) | 94.41% | **95.58%** (0.09) |

* Slip-rate learning helped every typist (+0.0 to +1.2 points) and hurt none. Almost all of the gain arrives
  on day 1 (about 2,400 words), and it stays put over five days of learning from its own fixes: no drift.
* Learning from its own fixes gets within about 0.1 points of learning from the right answers.
* Key confusions: applying all of them cost most typists 0.1 to 0.2 points, because pairs not seen yet were
  made dearer. Restricted to confusions seen at least five times, and only ever lowering a cost, they give the
  habitual-wrong-key typist +1.2 points and leave the others unchanged. That is the rule the app uses.
* The one cost: the extra-keys typist has slightly more correct words changed (0.11% → 0.14%).
* These typists are my own models of habits, so this shows the mechanism works and is stable, not how large
  the gain is for real people.

## Known gaps / next steps

* **Wrong corrections are about 0.11–0.12%** (about 1 in 850 correct words). Most are rare words and names missing from
  the dictionary. Target: under 0.1%.
* **About a quarter of mistakes are left alone.** These are mostly capitalised words (protected as possible names),
  very short ambiguous words (`wnt` = want/went), and words with 3+ slips.
* **Latency:** p99 is about 4 ms, mostly from space repair. That's fine for typing, and the search can be made faster.
* **The typist is simulated.** The trainer's keystroke export is how we get real data to tune against.
