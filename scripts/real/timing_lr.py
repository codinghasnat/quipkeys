#!/usr/bin/env python3
"""Likelihood ratios of error types given rhythm, from key-press times only.
For each letter press: r_before = gap before / run's median gap, r_after likewise.
Prints, per bin, P(label | bin) / P(label) so the engine can turn rhythm into costs.

  python3 -I scripts/real/timing_lr.py <runs dir> [ids-to-exclude-file]
"""
import collections, glob, json, math, re, statistics as st, sys
sys.path.insert(0, __import__("os").path.dirname(__file__))
from timing_analysis import osa_labels  # noqa: E402

excl = set(open(sys.argv[2]).read().split()) if len(sys.argv) > 2 else set()
rs = [json.load(open(f)) for f in glob.glob(sys.argv[1] + "/*.json") if f.split("/")[-1][:-5] not in excl]
BINS = [(0, 0.5), (0.5, 0.8), (0.8, 1.3), (1.3, 2.0), (2.0, 99)]
cnt = {s: collections.Counter() for s in ("before", "after")}
tot = collections.Counter()
for r in rs:
    keys = [(t, k) for t, k in r["keys"] if len(k) == 1]
    words, cur = [], []
    for t, k in keys:
        if k == " ":
            if cur: words.append(cur); cur = []
        else: cur.append((t, k))
    if cur: words.append(cur)
    gaps_all = [w[i][0] - w[i-1][0] for w in words for i in range(1, len(w))]
    if not gaps_all: continue
    med = st.median(gaps_all)
    caps = sum(1 for w in words if "".join(k for _, k in w)[1:].isupper()) > len(words) / 2
    tgt = [re.sub(r"[^a-z']", "", x.lower()) for x in r["target"].split()]
    for wi, w in enumerate(words[: len(tgt)]):
        typed = "".join(k for _, k in w).lower()
        if re.sub(r"[^a-z']", "", typed) != typed: continue
        lab = osa_labels(typed, tgt[wi]) if typed != tgt[wi] else ["ok"] * len(typed)
        for i, (t, k) in enumerate(w):
            L = lab[i]
            tot[L] += 1
            for side, j in (("before", i - 1), ("after", i + 1)):
                if 0 <= j < len(w):
                    rr = abs(t - w[j][0]) / med
                    b = next(n for n, (lo, hi) in enumerate(BINS) if lo <= rr < hi)
                    cnt[side][(b, L)] += 1
labels = ["ok", "sub", "extra", "swap", "after_missing"]
N = sum(tot.values())
print(f"{len(rs)} runs, {N} presses: " + ", ".join(f"{L} {tot[L]}" for L in labels))
for side in ("before", "after"):
    print(f"\nlikelihood ratio P(bin|label)/P(bin|ok), gap {side} the press (gap / run median):")
    print(f"{'bin':>10} " + " ".join(f"{L:>14}" for L in labels[1:]))
    for b, (lo, hi) in enumerate(BINS):
        row = []
        pok = (cnt[side][(b, 'ok')] + 0.5) / (tot['ok'] + 2.5)
        for L in labels[1:]:
            pl = (cnt[side][(b, L)] + 0.5) / (tot[L] + 2.5)
            row.append(f"{pl / pok:14.2f}")
        print(f"{f'{lo}-{hi}':>10} " + " ".join(row))
