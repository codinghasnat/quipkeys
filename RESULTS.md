# Results (held-out Brown test split, simulated typist)

Command: `./target/release/qk-bench --data data/eval/test.txt --level all`
(1,500 sentences, about 24k words per level). All parameters are tuned on `dev.txt` only.
Run on a 4 vCPU cloud container, release build.

## Scorecard

| | Round 1 | **Round 2** |
|---|---|---|
| **Moderate** (3% of letters wrong): typos fixed | 66.9% | **74.6%** |
| real-word slips fixed (form→from) | 7.7% | **29.6%** |
| spacing errors fixed | 60.7% | **67.5%** |
| correct words wrongly changed | 0.112% | 0.152% |
| word accuracy (no correction: 85.9%) | 95.4% | **96.3%** |
| **Blind** (10% of letters wrong, the north star): typos fixed | 61.9%* | **68.9%** |
| word accuracy (no correction: 61.7%) | 85.4%* | **88.0%** |
| correct words wrongly changed | 0.12%* | 0.118% |
| latency p50 / p99 per word | 0.2 / 2.7 ms | 0.4–0.7 / 3–4 ms |

\* Blind was added in round 2; round-1 numbers are from the round-1 engine on the dev split.

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

## What each part contributes (fix rate, moderate / blind)

| Removed | Moderate | Blind |
|---|---|---|
| nothing (full) | 74.6% | 68.9% |
| keyboard geometry | 70.3% (−4.3) | 62.3% (−6.6) |
| trigrams (pairs only) | 72.0% (−2.6) | 66.8% (−2.1) |
| space repair | 71.8% (−2.8) | 66.0% (−2.9) |
| second look-back | 73.4% (−1.2) | 68.0% (−0.9) |
| classic spellchecker (`plain`) | 51.9% | 45.3% |

## Full output

```
== light typing: 1.5% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   92.72%    65.3      -      -
full            76.9%    2.1%   21.0%     29.8%    0.154%   98.18%    75.8    358   3078
              (1739 mistyped units: 1598 non-words, 141 real words, 139 spacing; 22142 clean; fix rate: non-word 81.1%, spacing 66.2%)
plain           54.0%    2.8%   43.2%      1.4%    0.095%   96.56%    73.1    139   2475
no-geometry     72.7%    2.7%   24.6%     26.2%    0.135%   97.89%    75.3    368   3395
no-trigram      73.9%    2.1%   24.0%     17.7%    0.140%   97.97%    75.5    287   3036
no-boundary     73.9%    3.3%   22.8%     29.8%    0.154%   97.96%    75.2    307   2049
no-lookback2    76.0%    2.0%   22.0%     25.5%    0.140%   98.12%    75.7    303   2958
== moderate typing: 3.0% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   85.92%    55.8      -      -
full            74.6%    2.5%   22.9%     29.6%    0.152%   96.29%    71.9    443   3751
              (3353 mistyped units: 3076 non-words, 277 real words, 243 spacing; 20457 clean; fix rate: non-word 78.6%, spacing 67.5%)
plain           51.9%    3.3%   44.8%      0.7%    0.078%   93.15%    67.3    156   2759
no-geometry     70.3%    3.0%   26.8%     23.5%    0.127%   95.70%    71.0    405   3548
no-trigram      72.0%    2.7%   25.3%     20.6%    0.127%   95.95%    71.5    330   3354
no-boundary     71.8%    3.6%   24.5%     29.6%    0.147%   95.91%    71.0    329   2373
no-lookback2    73.4%    2.5%   24.1%     26.0%    0.132%   96.14%    71.7    353   3416
== messy typing: 6.0% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   73.98%    44.6      -      -
full            73.4%    3.4%   23.1%     27.6%    0.138%   92.98%    66.4    561   3966
              (6128 mistyped units: 5650 non-words, 478 real words, 558 spacing; 17421 clean; fix rate: non-word 77.3%, spacing 63.4%)
plain           50.1%    4.0%   45.9%      1.0%    0.080%   86.95%    59.0    281   2977
no-geometry     68.1%    3.9%   27.9%     22.4%    0.098%   91.63%    64.6    572   3858
no-trigram      71.0%    3.3%   25.7%     18.0%    0.115%   92.38%    65.7    486   3727
no-boundary     70.5%    4.8%   24.7%     27.4%    0.132%   92.22%    64.9    279   2221
no-lookback2    72.3%    3.3%   24.4%     24.1%    0.121%   92.70%    66.1    490   3673
== blind typing: 10.0% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   61.74%    37.4      -      -
full            68.9%    4.1%   27.0%     25.7%    0.118%   88.03%    59.5    729   4334
              (8927 mistyped units: 8193 non-words, 734 real words, 839 spacing; 14404 clean; fix rate: non-word 72.8%, spacing 57.9%)
plain           45.3%    5.0%   49.8%      1.8%    0.062%   79.02%    50.9    527   3167
no-geometry     62.3%    4.9%   32.8%     19.2%    0.104%   85.52%    56.7    736   4478
no-trigram      66.8%    4.0%   29.3%     17.4%    0.118%   87.21%    58.8    692   4563
no-boundary     66.0%    4.9%   29.1%     25.2%    0.104%   86.92%    57.8    347   3060
no-lookback2    68.0%    4.1%   27.9%     23.0%    0.118%   87.69%    59.2    726   4591
```

## Known gaps / next steps

* **Wrong corrections are 0.12–0.15%** (about 1 in 700 correct words). Most are rare words and names missing from
  the dictionary. Target: under 0.1%.
* **About a quarter of mistakes are left alone.** These are mostly capitalised words (protected as possible names),
  very short ambiguous words (`wnt` = want/went), and words with 3+ slips.
* **Latency:** p99 is about 4 ms, mostly from space repair. That's fine for typing, and the search can be made faster.
* **The typist is simulated.** The trainer's keystroke export is how we get real data to tune against.
