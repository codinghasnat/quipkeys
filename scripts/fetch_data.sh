#!/usr/bin/env bash
# Re-creates everything in data/ from the original public sources.
# The files are already committed; this exists so the provenance is reproducible.
set -euo pipefail
cd "$(dirname "$0")/.."
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# 1. Word and word-pair frequencies (Google Books Ngram derived, via SymSpell, MIT).
pip download symspellpy==6.10.1 --no-deps -d "$tmp" -q
(cd "$tmp" && unzip -q -o symspellpy-*.whl 'symspellpy/*.txt')
cp "$tmp/symspellpy/frequency_dictionary_en_82_765.txt" data/en_unigrams.txt
cp "$tmp/symspellpy/frequency_bigramdictionary_en_243_342.txt" data/en_bigrams.txt

# 2. Clean evaluation text (Brown corpus, NLTK distribution).
curl -sSfL -o "$tmp/brown.zip" https://raw.githubusercontent.com/nltk/nltk_data/gh-pages/packages/corpora/brown.zip
(cd "$tmp" && unzip -q -o brown.zip)
python3 -I scripts/prepare_eval.py "$tmp/brown" data/eval

# 3. Common words missing from the Google list (American spellings, "oh", "ok"...).
pip download wordfreq==3.1.1 --no-deps -d "$tmp" -q
pip install -q --target "$tmp/pylib" msgpack
(cd "$tmp" && unzip -q -o wordfreq-*.whl 'wordfreq/data/large_en.msgpack.gz')
PYTHONPATH="$tmp/pylib" python3 scripts/build_wordfreq_supplement.py \
  "$tmp/wordfreq/data/large_en.msgpack.gz" data/en_unigrams.txt > data/en_wordfreq.txt
