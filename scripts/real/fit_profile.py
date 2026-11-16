#!/usr/bin/env python3
"""Fit a personal typing profile (a "heat map" of one typist's key slips) from
recorded runs, for the engine to load (ChannelParams::apply_profile).

  python3 -I scripts/real/fit_profile.py profile.txt run1.json run2.json ...
  QK_PROFILE=profile.txt ./target/release/examples/replay < tokens

Each run has the target text and the keys typed. Target and typed text are
aligned letter by letter; every slip is counted (aimed at N, hit M; a letter
left out; an extra key; two keys swapped). Counts become costs in nats,
-ln(probability), blended with the engine's built-in costs so that keys with
little data stay close to the average typist:

    cost = -ln( (count + K * exp(-default cost)) / (opportunities + K) )

Only tallies of key slips are used. No words or sentences are written out.
"""
import json, math, sys

K = 150.0  # pseudo-count: how much the average typist is trusted over the data

ROWS = [("1234567890-=", -0.5), ("qwertyuiop[]", 0.0), ("asdfghjkl;'", 0.25), ("zxcvbnm,./", 0.75)]
POS = {ch: (i + off, r - 1.0) for r, (keys, off) in enumerate(ROWS) for i, ch in enumerate(keys)}
LETTERS = "abcdefghijklmnopqrstuvwxyz"


def dist(a, b):
    if a not in POS or b not in POS:
        return 10.0
    (ax, ay), (bx, by) = POS[a], POS[b]
    return math.hypot(ax - bx, ay - by)


def adjacent(a, b):
    return a != b and dist(a, b) <= 1.3


def default_sub(a, b):
    return min(5.5 + 3.0 * max(dist(a, b) - 1.0, 0.0), 13.0)


def typed_text(run):
    out = []
    for _, k in run["keys"]:
        if k == "Backspace":
            if out:
                out.pop()
        elif len(k) == 1:
            out.append(k)
    return "".join(out)


def align(target, typed):
    """Cheapest alignment (substitutions cheaper on adjacent keys, swaps
    allowed). Returns a list of ops: ('m', c), ('s', want, got), ('d', want,
    prev_want, next_want), ('i', got, prev_typed, next_typed), ('t', a, b)."""
    n, m = len(target), len(typed)
    INF = 1e9
    d = [[INF] * (m + 1) for _ in range(n + 1)]
    d[0][0] = 0.0
    for i in range(n + 1):
        for j in range(m + 1):
            c = d[i][j]
            if c >= INF:
                continue
            if i < n and j < m:
                a, b = target[i], typed[j]
                s = 0.0 if a == b else (0.6 if adjacent(a, b) else 1.0)
                if c + s < d[i + 1][j + 1]:
                    d[i + 1][j + 1] = c + s
            if i < n and c + 1.0 < d[i + 1][j]:
                d[i + 1][j] = c + 1.0
            if j < m and c + 1.0 < d[i][j + 1]:
                d[i][j + 1] = c + 1.0
            if i + 1 < n and j + 1 < m and target[i] == typed[j + 1] and target[i + 1] == typed[j] and target[i] != target[i + 1]:
                if c + 0.8 < d[i + 2][j + 2]:
                    d[i + 2][j + 2] = c + 0.8
    ops, i, j = [], n, m
    while i > 0 or j > 0:
        c = d[i][j]
        if i > 1 and j > 1 and target[i - 2] == typed[j - 1] and target[i - 1] == typed[j - 2] and target[i - 2] != target[i - 1] and abs(d[i - 2][j - 2] + 0.8 - c) < 1e-9:
            ops.append(("t", target[i - 2], target[i - 1])); i -= 2; j -= 2; continue
        if i > 0 and j > 0:
            a, b = target[i - 1], typed[j - 1]
            s = 0.0 if a == b else (0.6 if adjacent(a, b) else 1.0)
            if abs(d[i - 1][j - 1] + s - c) < 1e-9:
                ops.append(("m", a) if a == b else ("s", a, b)); i -= 1; j -= 1; continue
        if i > 0 and abs(d[i - 1][j] + 1.0 - c) < 1e-9:
            ops.append(("d", target[i - 1], target[i - 2] if i > 1 else "", target[i] if i < n else "")); i -= 1; continue
        ops.append(("i", typed[j - 1], typed[j - 2] if j > 1 else "", typed[j] if j < m else "")); j -= 1
    ops.reverse()
    return ops


def fit(runs):
    aimed = {c: 0 for c in LETTERS}          # times each letter was intended
    sub = {}                                   # (want, got) -> count
    n = dict(letters=0, doubles=0, del_double=0, del_other=0, ins_double=0, ins_adjacent=0, ins_other=0, transpose=0, pairs=0)
    for run in runs:
        target, typed = run["target"].lower(), typed_text(run).lower()
        # A run is cut off wherever the typist stopped: only score what they reached.
        target = target[: len(typed) + 5]
        prev = ""
        for op in align(target, typed):
            kind = op[0]
            if kind in ("m", "s", "d") and op[1] in aimed:
                aimed[op[1]] += 1
                n["letters"] += 1
                if op[1] == prev:
                    n["doubles"] += 1
            if kind == "s" and op[1] in aimed and op[2] in aimed:
                sub[(op[1], op[2])] = sub.get((op[1], op[2]), 0) + 1
            elif kind == "d" and op[1] in aimed:
                # One of a doubled pair left out ("helo"): either neighbour may match.
                n["del_double" if op[1] in (op[2], op[3]) else "del_other"] += 1
            elif kind == "i" and op[1] in aimed:
                g, p, q = op[1], op[2], op[3]
                if g in (p, q):
                    n["ins_double"] += 1
                elif adjacent(g, p) or adjacent(g, q):
                    n["ins_adjacent"] += 1
                else:
                    n["ins_other"] += 1
            elif kind == "t":
                n["transpose"] += 1
                n["letters"] += 2
                for c in op[1:]:
                    if c in aimed:
                        aimed[c] += 1
            if kind in ("m", "s", "d"):
                prev = op[1]
            elif kind == "t":
                prev = op[2]
    cost = lambda count, opportunities, c0: min(max(-math.log((count + K * math.exp(-c0)) / (opportunities + K)), 1.5), 13.0)
    lines = ["# QuipKeys personal typing profile: costs (nats) fitted from %d runs, %d letters." % (len(runs), n["letters"]),
             "# s <intended> <typed> <cost>   p <slip> <cost>. Tallies of key slips only; no text is stored."]
    for a in LETTERS:
        for b in LETTERS:
            if a == b:
                continue
            c0 = default_sub(a, b)
            c = cost(sub.get((a, b), 0), aimed[a], c0)
            if abs(c - c0) > 0.15:
                lines.append("s %s %s %.2f" % (a, b, c))
    L = n["letters"]
    for name, count, opp, c0 in [("ins_double", n["ins_double"], L, 5.5), ("ins_adjacent", n["ins_adjacent"], L, 6.0),
                                 ("ins_other", n["ins_other"], L, 11.0), ("del_double", n["del_double"], n["doubles"], 4.0),
                                 ("del_other", n["del_other"], L, 6.5), ("transpose", n["transpose"], L, 5.0)]:
        lines.append("p %s %.2f" % (name, cost(count, opp, c0)))
    return "\n".join(lines) + "\n", n, sub


if __name__ == "__main__":
    out, files = sys.argv[1], sys.argv[2:]
    runs = [json.load(open(f)) for f in files]
    text, n, sub = fit(runs)
    open(out, "w").write(text)
    top = sorted(sub.items(), key=lambda kv: -kv[1])[:12]
    print("%d runs, %d letters; slips: %s" % (len(runs), n["letters"], {k: v for k, v in n.items() if k not in ("letters", "doubles", "pairs")}), file=sys.stderr)
    print("most common key confusions (intended>typed): " + ", ".join("%s>%s x%d" % (a, b, c) for (a, b), c in top), file=sys.stderr)
