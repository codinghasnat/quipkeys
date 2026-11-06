#!/usr/bin/env python3
"""Build data/en_phon.txt: CMU Pronouncing Dictionary entries for words the
QuipKeys dictionary knows (lower case, letters and apostrophes only).

  curl -o cmudict.dict https://raw.githubusercontent.com/cmusphinx/cmudict/master/cmudict.dict
  python3 -I scripts/build_phon.py cmudict.dict > data/en_phon.txt

The CMU Pronouncing Dictionary is Copyright (C) 1993-2015 Carnegie Mellon
University, used under its BSD-style licence (see the header of the output).
"""
import re, sys

words = set()
for path in ["data/en_unigrams.txt", "data/en_wordfreq.txt", "data/en_supplement.txt"]:
    for line in open(path):
        if line.startswith("#") or not line.strip():
            continue
        words.add(line.split()[0].lower())

print("# Pronunciations from the CMU Pronouncing Dictionary (cmudict), filtered to")
print("# words QuipKeys knows. Copyright (C) 1993-2015 Carnegie Mellon University.")
print("# Redistribution and use permitted under the BSD-style licence at")
print("# https://github.com/cmusphinx/cmudict/blob/master/LICENSE (copy: data/CMUDICT-LICENSE.txt)")
n = 0
for line in open(sys.argv[1], encoding="utf-8", errors="replace"):
    line = line.split("#")[0].strip()
    if not line:
        continue
    w, *ph = line.split()
    w = re.sub(r"\(\d+\)$", "", w).lower()
    if w in words and re.fullmatch(r"[a-z']+", w):
        print(w, " ".join(p.rstrip("012") for p in ph))
        n += 1
print(f"{n} pronunciations", file=sys.stderr)
