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
  "$tmp/wordfreq/data/large_en.msgpack.gz" data/en_unigrams.txt data/en_capitalized.txt > data/en_wordfreq.txt

# 4. Corpus bigram/trigram counts (three-word context): 10 Wiki40B shards (~110M words,
#    CC BY-SA) + conversational NLTK corpora (Gutenberg, web chat, NPS chat, movie
#    reviews; weighted x5) + news (ABC, Reuters). Brown is excluded: it is the test set.
mkdir -p "$tmp/wiki" "$tmp/nltk"
for i in 00 01 02 03 04 05 06 07 08 09; do
  curl -sSf -o "$tmp/wiki/w$i.tfrecord" \
    "https://storage.googleapis.com/tfds-data/datasets/wiki40b/Wiki40B.en/1.0.0/wiki40b-train.tfrecord-000$i-of-00128"
done
python3 -I scripts/extract_wiki40b.py "$tmp"/wiki/*.tfrecord > "$tmp/wiki.txt"
for c in gutenberg webtext nps_chat movie_reviews abc reuters; do
  curl -sSfL -o "$tmp/nltk/$c.zip" "https://raw.githubusercontent.com/nltk/nltk_data/gh-pages/packages/corpora/$c.zip"
  (cd "$tmp/nltk" && unzip -q -o "$c.zip")
done
python3 -I scripts/prep_nltk_text.py "$tmp/nltk"
cargo run --release --bin qk-ngrams -- data/en_ngrams.bin --caps data/en_capitalized.txt --min2 3 --min3 3 \
  "1:$tmp/wiki.txt" "1:$tmp/nltk/news.txt" "5:$tmp/nltk/chatty.txt"
# (the same qk-ngrams run also writes data/en_capitalized.txt when given --caps)
