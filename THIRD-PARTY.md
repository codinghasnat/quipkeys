# Third-party data

QuipKeys' own code is licensed under the PolyForm Noncommercial License 1.0.0
(see `LICENSE`). The data files below come from other projects and stay under
their own terms. Check those terms before reusing them.

| File(s) | Source | Terms |
|---|---|---|
| `data/en_unigrams.txt`, `data/en_bigrams.txt` | SymSpell frequency dictionaries (derived from Google Books Ngram), via the `symspellpy` package | MIT |
| `data/en_wordfreq.txt` | Derived from the `wordfreq` 3.1.1 English word list | wordfreq's data licence (CC BY-SA 4.0) |
| `data/en_ngrams.bin`, `data/en_capitalized.txt` | Word-sequence counts computed from Wikipedia text (Wiki40B) | Wikipedia content is CC BY-SA |
| `data/en_nnlm.bin`, `data/nn_vocab.txt` | Small neural language model trained on the same Wikipedia and conversational text | As its training text |
| `data/en_phon.txt` | CMU Pronouncing Dictionary, filtered | BSD-style, Copyright (C) 1993-2015 Carnegie Mellon University; full text in `data/CMUDICT-LICENSE.txt` |
| `data/eval/{dev,test}.txt`, `data/eval/docs_*.txt` | Brown corpus sentences (NLTK distribution), for evaluation only | Brown corpus terms (non-commercial research use) |

`data/eval/phonetic*.txt`, `data/phon_costs.txt`, `data/en_variants.txt` and
`data/en_supplement.txt` were made for this project and fall under `LICENSE`.
