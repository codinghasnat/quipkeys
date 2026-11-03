#!/usr/bin/env python3
"""Error profile of real Typing Lab runs: what kinds of slips, how often.
Aligns each mistyped word with its target (optimal string alignment, so a
swapped pair counts once) and tallies the operations.

  python3 -I scripts/real/error_profile.py <dir of run .json files>
"""
import collections, difflib, glob, json, re, statistics as st, sys

rows, off = ["qwertyuiop", "asdfghjkl", "zxcvbnm"], [0, 0.25, 0.75]
pos = {c: (i + off[r], r) for r, row in enumerate(rows) for i, c in enumerate(row)}
dist = lambda a, b: ((pos[a][0] - pos[b][0]) ** 2 + (pos[a][1] - pos[b][1]) ** 2) ** 0.5 if a in pos and b in pos else 9


def osa(a, b):
    n, m = len(a), len(b)
    D = [[0] * (m + 1) for _ in range(n + 1)]
    for i in range(n + 1): D[i][0] = i
    for j in range(m + 1): D[0][j] = j
    sw = lambda i, j: i > 1 and j > 1 and a[i-1] == b[j-2] and a[i-2] == b[j-1] and a[i-1] != a[i-2]
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            D[i][j] = min(D[i-1][j] + 1, D[i][j-1] + 1, D[i-1][j-1] + (a[i-1] != b[j-1]))
            if sw(i, j): D[i][j] = min(D[i][j], D[i-2][j-2] + 1)
    ops, i, j = [], n, m
    while i > 0 or j > 0:
        if sw(i, j) and D[i][j] == D[i-2][j-2] + 1: ops.append(("swap",)); i -= 2; j -= 2
        elif i > 0 and j > 0 and D[i][j] == D[i-1][j-1] + (a[i-1] != b[j-1]):
            if a[i-1] != b[j-1]: ops.append(("sub", b[j-1], a[i-1]))
            i -= 1; j -= 1
        elif i > 0 and D[i][j] == D[i-1][j] + 1:
            c, p, nx = a[i-1], a[i-2] if i > 1 else "", a[i] if i < n else ""
            kind = "repeat" if c in (p, nx) else "adjacent" if min(dist(c, p) if p else 9, dist(c, nx) if nx else 9) <= 1.3 else "other"
            ops.append(("extra", kind)); i -= 1
        else: ops.append(("missing",)); j -= 1
    return ops


rs = [json.load(open(f)) for f in glob.glob(sys.argv[1] + "/*.json")]
ops = collections.Counter(); sub_adj = [0, 0]; letters = words = wrong = 0; wpm = []; iki = []
for r in rs:
    wpm.append(r["raw_wpm"])
    tw = r["typed"].split()
    if sum(1 for w in tw if w[1:].isupper()) > len(tw) / 2: tw = [w.swapcase() for w in tw]
    tl = [re.sub(r"[^a-z']", "", w.lower()) for w in tw]
    tg = [re.sub(r"[^a-z']", "", w.lower()) for w in r["target"].split()]
    letters += sum(len(x) for x in tg); words += len(tg)
    ks = [t for t, k in r["keys"] if len(k) == 1 and k != " "]
    iki += [b - a for a, b in zip(ks, ks[1:]) if 0 < b - a < 1000]
    for tag, i1, i2, j1, j2 in difflib.SequenceMatcher(a=tl, b=tg, autojunk=False).get_opcodes():
        if tag == "equal": continue
        wrong += max(i2 - i1, j2 - j1)
        if tag == "replace" and i2 - i1 == j2 - j1:
            for a, b in zip(tl[i1:i2], tg[j1:j2]):
                for x in osa(a, b):
                    if x[0] == "sub": ops["wrong key"] += 1; sub_adj[0] += dist(x[1], x[2]) <= 1.3; sub_adj[1] += 1
                    elif x[0] == "extra": ops["extra letter (" + x[1] + ")"] += 1
                    elif x[0] == "missing": ops["missing letter"] += 1
                    else: ops["swapped pair"] += 1
        else: ops["word boundary"] += max(i2 - i1, j2 - j1)
tot = sum(ops.values())
print(json.dumps({"runs": len(rs), "words": words, "letters": letters, "mistyped_words": wrong, "errors": tot,
                  "wpm_median": st.median(wpm), "wpm_min": min(wpm), "wpm_max": max(wpm), "iki_median_ms": st.median(iki),
                  "adjacent_share_of_wrong_key": sub_adj[0] / max(1, sub_adj[1]),
                  "ops": {k: v for k, v in ops.most_common()}}, indent=1))
