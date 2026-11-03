#!/usr/bin/env python3
"""Build clean English evaluation sentences from the Brown corpus (NLTK copy).

Usage: python3 -I scripts/prepare_eval.py <path-to-extracted-brown-dir> data/eval

Writes dev.txt (for tuning) and test.txt (held out; only for reporting).
The Brown corpus is independent of the Google Books frequency data the
engine uses, so the test sentences are not "seen" by the language model.
"""
import os, random, re, sys

src, out = sys.argv[1], sys.argv[2]
ATTACH_LEFT = {",", ".", ";", ":", "?", "!", ")", "'s", "n't"}
sents = []
for name in sorted(os.listdir(src)):
    if not re.fullmatch(r"c[a-r]\d\d", name):
        continue
    for line in open(os.path.join(src, name), encoding="latin-1"):
        line = line.strip()
        if not line:
            continue
        words = []
        for tok in line.split():
            w = tok.rsplit("/", 1)[0]
            if w in ("``", "''", "--", "(", ")", "'"):
                continue
            if words and w in ATTACH_LEFT:
                words[-1] += w
            else:
                words.append(w)
        if not 6 <= len(words) <= 30:
            continue
        s = " ".join(words)
        # Skip sentences with odd tokenisation artefacts.
        if re.search(r"[\[\]{}*&$#@]|\d", s):
            continue
        sents.append(s)

random.Random(1961).shuffle(sents)
os.makedirs(out, exist_ok=True)
for fname, chunk in (("dev.txt", sents[:1500]), ("test.txt", sents[1500:3000])):
    with open(os.path.join(out, fname), "w") as f:
        f.write("\n".join(chunk) + "\n")
print(f"{len(sents)} usable sentences; wrote 1500 dev + 1500 test to {out}")
