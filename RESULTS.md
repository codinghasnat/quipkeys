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

## Known gaps / next steps

* **Wrong corrections are about 0.11–0.12%** (about 1 in 850 correct words). Most are rare words and names missing from
  the dictionary. Target: under 0.1%.
* **About a quarter of mistakes are left alone.** These are mostly capitalised words (protected as possible names),
  very short ambiguous words (`wnt` = want/went), and words with 3+ slips.
* **Latency:** p99 is about 4 ms, mostly from space repair. That's fine for typing, and the search can be made faster.
* **The typist is simulated.** The trainer's keystroke export is how we get real data to tune against.
