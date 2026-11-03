#!/usr/bin/env python3
"""Turn qk-bench output into the paper's LaTeX tables (numbers.tex, table_main.tex,
table_ablate.tex), so no number is copied by hand.

  python3 -I make_tables.py bench_output.txt
"""
import re, sys

levels, cur = {}, None
for line in open(sys.argv[1]):
    m = re.match(r"== (\w+) typing", line)
    if m:
        cur = m.group(1); levels[cur] = {}; continue
    p = line.split()
    if cur and p and (p[0] in ("full", "plain", "no-correct") or p[0].startswith(("no-", "word-", "always-"))) and len(p) >= 10:
        levels[cur][p[0]] = p[1:]

def col(level, cfg, i):  # fixed wrong left realword falsecorr wordacc effwpm p50 p99
    return levels.get(level, {}).get(cfg, ["?"] * 9)[i].rstrip("%")

nums = {
    "ModFix": col("moderate", "full", 0), "ModFixPlain": col("moderate", "plain", 0),
    "BlindNone": f'{float(col("blind", "no-correct", 5)):.1f}', "BlindAcc": f'{float(col("blind", "full", 5)):.1f}',
}
with open("numbers.tex", "w") as f:
    for k, v in nums.items():
        f.write(f"\\newcommand{{\\{k}}}{{{v}}}\n")

rows = []
for lvl in ["light", "moderate", "messy", "blind", "rough"]:
    if lvl not in levels: continue
    none = col(lvl, "no-correct", 5)
    rows.append(f"{lvl} & no correction & -- & -- & -- & {none}\\% & -- \\\\")
    for cfg, name in [("plain", "plain"), ("full", "\\qk{}")]:
        ms = lambda i: f"{float(col(lvl, cfg, i)) / 1000:.1f}" if col(lvl, cfg, i) != "?" else "?"
        rows.append(f" & {name} & {col(lvl, cfg, 0)}\\% & {col(lvl, cfg, 1)}\\% & {col(lvl, cfg, 4)}\\% & {col(lvl, cfg, 5)}\\% & {ms(7)} / {ms(8)} \\\\")
    rows.append("\\addlinespace")
open("table_main.tex", "w").write("\\begin{tabular}{llccccc}\n\\toprule\nLevel & System & Fixed & Wrong & False corr. & Word acc. & p50 / p99 (ms) \\\\\n\\midrule\n" + "\n".join(rows[:-1]) + "\n\\bottomrule\n\\end{tabular}\n")

abl = [("full", "\\qk{} (full)"), ("word-by-word", "word-by-word instead of lattice"), ("no-geometry", "no keyboard geometry"),
       ("no-trigram", "pairs only (no trigrams)"), ("no-boundary", "no boundary repair"),
       ("always-precise", "never switch to rough-aim model"), ("plain", "plain (frequency-only)")]
out = []
for cfg, name in abl:
    cells = []
    for lvl in ["moderate", "messy", "blind", "rough"]:
        v = col(lvl, cfg, 0)
        cells.append(f"{v}\\%")
    out.append(f"{name} & " + " & ".join(cells) + " \\\\" + ("\\midrule" if cfg == "full" else ""))
open("table_ablate.tex", "w").write("\\begin{tabular}{lcccc}\n\\toprule\nConfiguration & moderate & messy & blind & rough \\\\\n\\midrule\n" + "\n".join(out) + "\n\\bottomrule\n\\end{tabular}\n")
print(nums)
