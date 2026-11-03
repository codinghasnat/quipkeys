#!/usr/bin/env python3
"""Build data/en_variants.txt: UK/US spelling pairs where both forms are in
the dictionary, so the engine can prefer the user's English when correcting.

Usage: python3 -I scripts/build_variants.py data/en_unigrams.txt data/en_wordfreq.txt > data/en_variants.txt
"""
import sys

words = {}
for path in sys.argv[1:]:
    for line in open(path):
        if line.startswith("#"):
            continue
        p = line.split()
        if len(p) >= 2 and p[0].isalpha():
            words[p[0]] = max(words.get(p[0], 0), int(p[1]))

# (uk suffix, us suffix): applied to word endings only.
SUFFIX = [
    ("our", "or"), ("ours", "ors"), ("oured", "ored"), ("ouring", "oring"), ("ourite", "orite"),
    ("ourites", "orites"), ("ourable", "orable"), ("ourful", "orful"), ("ourless", "orless"),
    ("ise", "ize"), ("ised", "ized"), ("ises", "izes"), ("ising", "izing"), ("isation", "ization"),
    ("isations", "izations"), ("iser", "izer"), ("isers", "izers"), ("yse", "yze"), ("ysed", "yzed"),
    ("ysing", "yzing"), ("tre", "ter"), ("tres", "ters"), ("logue", "log"), ("logues", "logs"),
    ("ence", "ense"), ("lled", "led"), ("lling", "ling"), ("ller", "ler"), ("llers", "lers"),
    ("mme", "m"), ("mmes", "ms"),
]
NOT = {"four", "pour", "your", "hour", "tour", "sour", "flour", "dour", "hours", "tours",
       "rise", "wise", "vise", "since", "fence", "hence", "pence", "whence", "tense", "sense",
       "dense", "lens", "acre", "ogre", "metre", "mitre", "lucre", "ochre", "fibre"}
# A few common whole-word pairs.
PAIRS = [("grey", "gray"), ("aluminium", "aluminum"), ("mum", "mom"), ("arse", "ass"),
         ("plough", "plow"), ("sceptic", "skeptic"), ("sceptical", "skeptical"), ("metre", "meter"),
         ("metres", "meters"), ("fibre", "fiber"), ("fibres", "fibers"), ("ageing", "aging"),
         ("jewellery", "jewelry"), ("manoeuvre", "maneuver"), ("oestrogen", "estrogen"),
         ("paediatric", "pediatric"), ("anaemia", "anemia"), ("encyclopaedia", "encyclopedia")]

out = set()
for w in words:
    if w in NOT:
        continue
    for uk, us in SUFFIX:
        if w.endswith(uk) and len(w) > len(uk) + 2:
            v = w[: -len(uk)] + us
            if v in words and v not in NOT and v != w:
                out.add((w, v))
for uk, us in PAIRS:
    if uk in words and us in words:
        out.add((uk, us))
print("# UK/US spelling pairs (uk us), both forms in the dictionary. Built by scripts/build_variants.py.")
for uk, us in sorted(out):
    print(uk, us)
print(f"{len(out)} pairs", file=sys.stderr)
