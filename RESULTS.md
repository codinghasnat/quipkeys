# Results (held-out Brown test split, simulated typist)

Command: `./target/release/qk-bench --data data/eval/test.txt --level all`
(1,500 sentences, about 24k words per level). Parameters were tuned only on `dev.txt`. Single machine,
4 vCPU cloud container, release build.

## Headline (moderate typing: 3% of letters mistyped)

| | No correction | Classic spellchecker (`plain`) | **QuipKeys** |
|---|---|---|---|
| Mistyped words fixed | 0% | 51.0% | **66.9%** |
| Changed to the wrong word | – | 3.1% | 2.8% |
| Correct words wrongly changed | 0% | 0.107% | **0.112%** |
| Word accuracy | 86.3% | 93.2% | **95.4%** |
| Effective WPM (80 WPM typist, model) | 56.3 | 67.4 | **70.7** |
| Latency p50 / p99 per word | – | 112 / 2499 µs | 218 / 2659 µs |

QuipKeys fixes about 16 points more mistakes than a frequency-only spellchecker, at the same
false-correction rate (about 1 in 900 correctly typed words). Most remaining false corrections are rare
words and names that the dictionary lacks (`Agnese`→`Agnes`, `torquer`→`torque`), plus a few
misspellings already present in the source text.

## What each part contributes (fix rate, moderate)

| Removed | Fix rate | Δ |
|---|---|---|
| nothing (full) | 66.9% | |
| keyboard geometry | 61.1% | −5.8 |
| all context | 61.8% | −5.1 |
| right-hand context (look-back) | 64.2% | −2.7 |
| key timing | 66.1% | −0.8 (synthetic timing, see caveats) |

## Full output

```
== light typing: 1.5% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   93.00%    65.6      -      -
full            69.7%    3.1%   27.2%      7.4%    0.126%   97.76%    75.0    205   2308
              (1672 mistyped units: 1537 non-words, 135 real words, 127 spacing; 22215 clean; fix rate: non-word 75.2%, spacing 58.3%)
plain           53.5%    3.1%   43.5%      0.0%    0.108%   96.64%    73.0    105   2241
no-geometry     65.0%    3.0%   32.0%      6.7%    0.108%   97.45%    74.4    204   2365
no-context      63.9%    3.1%   33.0%      1.5%    0.135%   97.35%    74.3    106   2322
no-right-ctx    67.0%    3.1%   29.9%      4.4%    0.126%   97.58%    74.7    107   2240
no-timing       68.8%    3.0%   28.2%      5.2%    0.126%   97.70%    74.9    221   2567
== moderate typing: 3.0% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   86.33%    56.3      -      -
full            66.9%    2.8%   30.3%      7.7%    0.112%   95.39%    70.7    218   2659
              (3255 mistyped units: 2971 non-words, 284 real words, 219 spacing; 20561 clean; fix rate: non-word 72.6%, spacing 60.7%)
plain           51.0%    3.1%   45.9%      0.0%    0.107%   93.21%    67.4    112   2499
no-geometry     61.1%    2.7%   36.2%      5.6%    0.112%   94.58%    69.3    212   2621
no-context      61.8%    2.7%   35.5%      1.8%    0.126%   94.67%    69.7    112   2474
no-right-ctx    64.2%    2.6%   33.2%      3.2%    0.112%   95.01%    70.1    118   2543
no-timing       66.1%    2.8%   31.1%      6.3%    0.112%   95.27%    70.5    216   2574
== messy typing: 6.0% of letters mistyped, 1500 sentences from data/eval/test.txt ==
config          fixed   wrong    left  realword falsecorr word-acc eff-WPM  p50µs  p99µs
no-correct          -       -  100.0%         -         -   74.49%    45.1      -      -
full            63.2%    3.3%   33.6%      7.3%    0.114%   90.52%    62.8    233   2824
              (6021 mistyped units: 5497 non-words, 524 real words, 515 spacing; 17580 clean; fix rate: non-word 68.5%, spacing 48.9%)
plain           45.8%    4.2%   49.9%      0.0%    0.108%   86.10%    57.5    135   2734
no-geometry     56.1%    3.8%   40.0%      4.8%    0.102%   88.73%    60.4    227   2712
no-context      57.3%    3.6%   39.1%      1.5%    0.114%   89.03%    61.2    131   2696
no-right-ctx    59.9%    3.1%   36.9%      2.5%    0.114%   89.70%    61.9    131   2607
no-timing       61.8%    3.2%   35.0%      6.1%    0.114%   90.17%    62.4    235   2848
```

## Known gaps / next steps

* **About 30% of mistakes are left alone.** Some are deliberate (capitalised words mid-sentence are treated as names;
  ambiguous cases like `hopw` → hope/how stay below the 90% confidence bar). Others are 3+ slips in one word.
* **Real-word slips** (`form`→`from`, `fat`→`far`): only about 7% fixed. Bigrams aren't enough evidence to overrule a
  real word safely. That needs trigrams or a small n-gram LM, which is still offline.
* **Long run-togethers** (`Nicetomeetyou`, 4 words) aren't split yet. Two-word joins are.
* **Latency:** p50 is about 0.2 ms; p99 is about 2.5 ms, all from the word-boundary repair, which runs extra searches.
  It's fine for typing, but caching the previous word's search would halve it.
* **Phonetic** (`definately`), homophones, casing (`london`), missing words: out of scope for v1, as agreed.
* **Validate on real keystrokes** (Aalto 136M dataset or a personal log) before trusting the timing numbers.
