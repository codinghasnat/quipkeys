# Results (held-out Brown test split, simulated typist)

Command: `./target/release/qk-bench --data data/eval/test.txt --level all`
(1,500 sentences, about 24k words per level). All parameters are tuned on `dev.txt` only.
Run on a 4 vCPU cloud container, release build.

## Scorecard

| | Round 1 | **Round 2** |
|---|---|---|
| **Moderate** (3% of letters wrong): typos fixed | 66.9% | **73.6%** |
| real-word slips fixed (form→from) | 7.7% | **18.8%** |
| spacing errors fixed | 60.7% | **67.5%** |
| correct words wrongly changed | 0.112% | 0.122% |
| word accuracy (no correction: 85.9%) | 95.4% | **96.2%** |
| **Blind** (10% of letters wrong, the north star): typos fixed | 61.9%* | **67.8%** |
| word accuracy (no correction: 61.7%) | 85.4%* | **87.6%** |
| correct words wrongly changed | 0.12%* | 0.111% |
| latency p50 / p99 per word | 0.2 / 2.7 ms | 0.4–0.7 / 3.6–4.2 ms |

\* Blind was added in round 2; round-1 numbers are from the round-1 engine on the dev split.

Real-word changes are deliberately conservative (98% confidence plus an extra cost). A looser setting fixes
29.6% of real-word slips on this benchmark, but in everyday sentences it turned a correctly typed
"ate" into "are". That trade isn't worth it.

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
| nothing (full) | 73.6% | 67.8% |
| keyboard geometry | 69.6% (−4.0) | 61.6% (−6.2) |
| trigrams (pairs only) | 71.1% (−2.5) | 66.0% (−1.8) |
| space repair | 70.8% (−2.8) | 64.9% (−2.9) |
| second look-back | 72.3% (−1.3) | 67.0% (−0.8) |
| classic spellchecker (`plain`) | 51.7% | 45.1% |

## Full output

```
== light typing: 1.5% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   92.72%    65.3      -      -
full            75.5%    2.2%   22.3%     14.9%    0.122%   98.10%    75.7    388   3359
              (1739 mistyped units: 1598 non-words, 141 real words, 139 spacing; 22142 clean; fix rate: non-word 80.9%, spacing 64.0%)
plain           53.9%    2.8%   43.4%      0.0%    0.090%   96.56%    73.1    150   2634
no-geometry     71.4%    2.9%   25.8%     12.8%    0.108%   97.81%    75.1    370   3404
no-trigram      73.3%    2.1%   24.6%     10.6%    0.126%   97.94%    75.4    304   3090
no-boundary     72.5%    3.4%   24.2%     14.9%    0.122%   97.88%    75.1    334   2133
no-lookback2    74.6%    2.1%   23.3%     11.3%    0.122%   98.04%    75.6    323   3165
== moderate typing: 3.0% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   85.92%    55.8      -      -
full            73.6%    2.5%   23.9%     18.8%    0.122%   96.18%    71.8    426   3583
              (3353 mistyped units: 3076 non-words, 277 real words, 243 spacing; 20457 clean; fix rate: non-word 78.6%, spacing 67.5%)
plain           51.7%    3.3%   45.0%      0.0%    0.078%   93.13%    67.3    173   2969
no-geometry     69.6%    2.9%   27.5%     17.3%    0.112%   95.63%    70.9    450   3909
no-trigram      71.1%    2.7%   26.3%     10.1%    0.117%   95.83%    71.3    351   3544
no-boundary     70.8%    3.6%   25.6%     18.8%    0.117%   95.79%    70.9    333   2332
no-lookback2    72.3%    2.4%   25.2%     14.1%    0.122%   96.00%    71.5    368   3570
== messy typing: 6.0% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   73.98%    44.6      -      -
full            72.6%    3.4%   24.0%     18.0%    0.109%   92.80%    66.2    591   4079
              (6128 mistyped units: 5650 non-words, 478 real words, 558 spacing; 17421 clean; fix rate: non-word 77.3%, spacing 63.4%)
plain           50.1%    4.0%   45.9%      0.6%    0.075%   86.95%    59.1    301   3149
no-geometry     67.4%    3.9%   28.7%     13.8%    0.080%   91.46%    64.4    600   4020
no-trigram      70.4%    3.2%   26.3%      9.6%    0.109%   92.23%    65.6    523   4125
no-boundary     69.6%    4.9%   25.6%     17.8%    0.103%   92.00%    64.6    314   2413
no-lookback2    71.6%    3.3%   25.1%     15.9%    0.109%   92.53%    65.9    523   3846
== blind typing: 10.0% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   61.74%    37.4      -      -
full            67.8%    4.1%   28.1%     14.7%    0.111%   87.61%    59.1    718   4242
              (8927 mistyped units: 8193 non-words, 734 real words, 839 spacing; 14404 clean; fix rate: non-word 72.6%, spacing 57.1%)
plain           45.1%    5.0%   49.9%      0.5%    0.062%   78.97%    50.9    532   3197
no-geometry     61.6%    4.9%   33.5%     11.4%    0.090%   85.23%    56.4    705   3957
no-trigram      66.0%    4.0%   30.0%      9.5%    0.111%   86.94%    58.5    652   3982
no-boundary     64.9%    5.0%   30.1%     14.3%    0.097%   86.52%    57.4    293   2438
no-lookback2    67.0%    4.1%   28.9%     12.9%    0.111%   87.32%    58.8    644   3843
```

## Known gaps / next steps

* **Wrong corrections are about 0.11–0.12%** (about 1 in 850 correct words). Most are rare words and names missing from
  the dictionary. Target: under 0.1%.
* **About a quarter of mistakes are left alone.** These are mostly capitalised words (protected as possible names),
  very short ambiguous words (`wnt` = want/went), and words with 3+ slips.
* **Latency:** p99 is about 4 ms, mostly from space repair. That's fine for typing, and the search can be made faster.
* **The typist is simulated.** The trainer's keystroke export is how we get real data to tune against.
