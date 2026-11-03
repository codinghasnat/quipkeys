#!/usr/bin/env python3
"""Score real Typing Lab runs: replay the exact keystrokes (with timing) through the
engine and compare raw vs corrected word accuracy against the target text.

  cargo build --release --example replay
  python3 -I scripts/real/score_runs.py <dir of run .json files> ./target/release/examples/replay
  (QK_PERSONAL=1 to use the fitted personal error model)
"""
import json, glob, re, subprocess, sys, difflib, statistics as st
rs = [json.load(open(f)) for f in glob.glob(sys.argv[1] + "/*.json")]
rs.sort(key=lambda r: r["at"])
def runs_tokens(r):
    toks, cur, times = [], "", []
    for t, k in r["keys"]:
        if k == " ":
            if cur: toks.append((cur, times)); cur, times = "", []
        elif k == "Backspace": cur = cur[:-1]; times = times[:-1]
        elif len(k) == 1: cur += k; times.append(t)
    if cur: toks.append((cur, times))
    return toks
core = lambda w: re.sub(r"[^a-z']", "", w.lower())
inp = []
for r in rs:
    for tok, ts in runs_tokens(r):
        inp.append(tok + "\t" + ",".join(str(x) for x in ts))
    inp.append("")
out = subprocess.run([sys.argv[2]], input="\n".join(inp) + "\n", capture_output=True, text=True).stdout
outs = [blk.split("\n") for blk in out.strip("\n").split("\n\n")]
def score(words, target):
    a = [core(w) for w in " ".join(words).split()]
    b = [core(w) for w in target.split()]
    sm = difflib.SequenceMatcher(a=a, b=b, autojunk=False)
    return sum(m.size for m in sm.get_matching_blocks()) / len(b)
tot_raw = tot_qk = 0
for r, o in zip(rs, outs):
    typed = [t for t, _ in runs_tokens(r)]
    caps = sum(1 for t in typed if t[1:].isupper()) > len(typed) / 2
    s_raw, s_qk = score(typed, r["target"]), score(o, r["target"])
    tot_raw += s_raw; tot_qk += s_qk
    print(f'{r["at"][11:16]} {round(r["raw_wpm"]):>3} wpm  raw {100*s_raw:5.1f}%  QuipKeys {100*s_qk:5.1f}%{"  (caps lock on)" if caps else ""}')
print(f"MEAN raw {100*tot_raw/len(rs):.1f}%  QuipKeys {100*tot_qk/len(rs):.1f}%")
# show a run's words side by side
for r, o in list(zip(rs, outs))[:3]:
    print("TYPED :", " ".join(t for t, _ in runs_tokens(r)))
    print("OUTPUT:", " ".join(x for x in o if x))
    print("TARGET:", r["target"]); print()
