#!/usr/bin/env python3
"""Timing signature of typos in Typing Lab v2 runs (key press + release logged).

Each letter press is aligned to the intended text and labelled: correct, wrong
key, extra key, swapped pair, or the key right after a missing letter. Then we
compare hold time, the gap before/after the press, and overlap with the
previous key, plus word-level hesitation patterns.

  python3 -I scripts/real/timing_analysis.py <dir of run .json files>
"""
import collections, glob, json, re, statistics as st, sys



def osa_labels(a, b):
    """Label each typed char of a (vs intended b): ok/sub/extra/swap; also mark
    typed positions right after an omission ('after_missing')."""
    n, m = len(a), len(b)
    D = [[0] * (m + 1) for _ in range(n + 1)]
    for i in range(n + 1): D[i][0] = i
    for j in range(m + 1): D[0][j] = j
    sw = lambda i, j: i > 1 and j > 1 and a[i-1] == b[j-2] and a[i-2] == b[j-1] and a[i-1] != a[i-2]
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            D[i][j] = min(D[i-1][j] + 1, D[i][j-1] + 1, D[i-1][j-1] + (a[i-1] != b[j-1]))
            if sw(i, j): D[i][j] = min(D[i][j], D[i-2][j-2] + 1)
    lab = ["ok"] * n; i, j = n, m
    while i > 0 or j > 0:
        if sw(i, j) and D[i][j] == D[i-2][j-2] + 1: lab[i-1] = lab[i-2] = "swap"; i -= 2; j -= 2
        elif i > 0 and j > 0 and D[i][j] == D[i-1][j-1] + (a[i-1] != b[j-1]):
            if a[i-1] != b[j-1]: lab[i-1] = "sub"
            i -= 1; j -= 1
        elif i > 0 and D[i][j] == D[i-1][j] + 1: lab[i-1] = "extra"; i -= 1
        else:
            if i < n and lab[i] == "ok": lab[i] = "after_missing"
            j -= 1
    return lab


def main():
    rs = [json.load(open(f)) for f in glob.glob(sys.argv[1] + "/*.json")]
    rs = [r for r in rs if r.get("v") == 2 and r.get("downs") and r.get("ups")]
    feat = collections.defaultdict(lambda: collections.defaultdict(list))
    word_stats = collections.defaultdict(lambda: collections.defaultdict(list))
    n_keys = 0
    for r in rs:
        ev = sorted([(t, c, k, 1) for t, c, k in r["downs"]] + [(t, c, k, 0) for t, c, k in r["ups"]], key=lambda x: (x[0], -x[3]))
        held, presses = {}, []
        for t, c, k, d in ev:
            if d:
                presses.append({"t": t, "code": c, "key": k, "up": None, "held_others": [p for p in held.values()]})
                held[c] = presses[-1]
            elif c in held:
                held.pop(c)["up"] = t
        # letter presses only, grouped into words by Space
        words, cur = [], []
        for p in presses:
            if p["key"] == " ":
                if cur: words.append(cur); cur = []
            elif len(p["key"]) == 1 and p["key"] != "Backspace":
                cur.append(p)
        if cur: words.append(cur)
        tgt = [re.sub(r"[^a-z']", "", w.lower()) for w in r["target"].split()]
        for wi, w in enumerate(words[: len(tgt)]):
            typed = "".join(p["key"] for p in w).lower()
            tw = re.sub(r"[^a-z']", "", typed)
            if len(tw) != len(typed): continue  # punctuation inside; skip for clean alignment
            lab = osa_labels(typed, tgt[wi]) if typed != tgt[wi] else ["ok"] * len(typed)
            gaps = [w[k]["t"] - w[k-1]["t"] for k in range(1, len(w))]
            clean = typed == tgt[wi]
            ws = word_stats["clean word" if clean else "word with typo"]
            if gaps:
                ws["mean gap"].append(st.mean(gaps)); ws["max gap"].append(max(gaps))
            holds = [p["up"] - p["t"] for p in w if p["up"] is not None]
            if holds: ws["mean hold"].append(st.mean(holds))
            for k, p in enumerate(w):
                n_keys += 1
                L = lab[k]
                f = feat[L]
                if p["up"] is not None: f["hold"].append(p["up"] - p["t"])
                if k > 0: f["gap before"].append(p["t"] - w[k-1]["t"])
                if k + 1 < len(w): f["gap after"].append(w[k+1]["t"] - p["t"])
                f["overlap"].append(1.0 if any(o["code"] != p["code"] for o in p["held_others"] if len(o["key"]) == 1) else 0.0)
                # distance in time from the word's start, normalised
                f["position"].append(k / max(1, len(w) - 1))

    def q(v):
        if not v: return "-"
        s = sorted(v); return f"{st.median(s):6.1f} [{s[len(s)//4]:5.1f}-{s[3*len(s)//4]:5.1f}]"

    print(f"{len(rs)} v2 runs, {n_keys} letter presses (clock resolution {sorted(set(r.get('clock_res_ms') for r in rs))} ms)")
    order = ["ok", "sub", "extra", "swap", "after_missing"]
    print(f"{'label':14} {'n':>5}  {'hold ms (median [IQR])':>24}  {'gap before':>22}  {'gap after':>22}  overlap")
    for L in order:
        f = feat[L]
        if not f["hold"] and not f["gap before"]: continue
        print(f"{L:14} {len(f['position']):5}  {q(f['hold']):>24}  {q(f['gap before']):>22}  {q(f['gap after']):>22}  {100*st.mean(f['overlap']):5.1f}%")
    print()
    for W, ws in word_stats.items():
        print(f"{W:16} n={len(ws['mean gap']):4}  mean gap {q(ws['mean gap'])}  max gap {q(ws['max gap'])}  mean hold {q(ws['mean hold'])}")


if __name__ == "__main__":
    main()
