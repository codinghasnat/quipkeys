#!/usr/bin/env python3
"""Write docs/assets/demo.svg: an animated (CSS, loops) terminal card where
sloppy lines type out and then snap to QuipKeys' correction. Pure SVG + CSS, so
it animates in GitHub READMEs.  python3 scripts/make_demo_svg.py"""
from html import escape

LINES = [  # (typed: [(text, typo?)], fixed: [(text, fixed?)])
    ([("I ", 0), ("wamt", 1), (" to ", 0), ("gp", 1), (" to the ", 0), ("stpre", 1)],
     [("I ", 0), ("want", 1), (" to ", 0), ("go", 1), (" to the ", 0), ("store", 1)]),
    ([("Hellon ice", 1), (" to ", 0), ("mee tyou", 1)], [("Hello nice", 1), (" to ", 0), ("meet you", 1)]),
    ([("thsi", 1), (" is ", 0), ("teh", 1), (" best ", 0), ("keybaord", 1)],
     [("this", 1), (" is ", 0), ("the", 1), (" best ", 0), ("keyboard", 1)]),
    ([("I need more ", 0), ("nollij", 1)], [("I need more ", 0), ("knowledge", 1)]),
]
W, CH, X0, Y0, LH = 760, 13.2, 64, 92, 44
CYCLE = 16.0
TYPE, GAP = 2.0, 0.55   # seconds to type a line; pause before the fix
starts = [0.4 + i * (TYPE + GAP + 0.9) for i in range(len(LINES))]
pct = lambda t: f"{100 * t / CYCLE:.2f}%"
END = CYCLE - 1.2

css = [f"""
text {{ font-family: 'JetBrains Mono', ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 22px; }}
.typed {{ fill: #c9d1d9; }} .typo {{ fill: #ff7b72; }} .fixed {{ fill: #c9d1d9; }} .ok {{ fill: #56d364; font-weight: 700; }}
.prompt {{ fill: #6b8ffa; font-weight: 700; }}
.label {{ font-family: -apple-system, 'Segoe UI', sans-serif; font-size: 13px; fill: #8b949e; letter-spacing: .08em; }}
@media (prefers-reduced-motion: reduce) {{ * {{ animation: none !important; }} .typed-g {{ opacity: 0; }} .fixed-g {{ opacity: 1 !important; }} .clip rect {{ width: {W}px; }} }}
"""]
body = []
for i, (typed, fixed) in enumerate(LINES):
    y = Y0 + i * LH
    plain = "".join(x for x, _ in typed)
    s = starts[i]
    w = len(plain) * CH + 4
    t_fix = s + TYPE + GAP
    n = len(plain)
    css.append(f"""
@keyframes type{i} {{ 0% {{ width: 0 }} {pct(s)} {{ width: 0 }} {pct(s + TYPE)} {{ width: {w}px }} 100% {{ width: {w}px }} }}
#c{i} rect {{ animation: type{i} {CYCLE}s steps(1) infinite; animation-timing-function: steps({n}, end); }}
@keyframes swapout{i} {{ 0%, {pct(t_fix)} {{ opacity: 1 }} {pct(t_fix + 0.25)}, 100% {{ opacity: 0 }} }}
@keyframes swapin{i} {{ 0%, {pct(t_fix)} {{ opacity: 0 }} {pct(t_fix + 0.25)}, {pct(END)} {{ opacity: 1 }} {pct(END + 0.6)}, 100% {{ opacity: 0 }} }}
@keyframes hide{i} {{ 0%, {pct(END)} {{ opacity: 1 }} {pct(END + 0.6)}, 100% {{ opacity: 0 }} }}
#t{i} {{ animation: swapout{i} {CYCLE}s infinite; }} #f{i} {{ animation: swapin{i} {CYCLE}s infinite; opacity: 0; }}
#p{i} {{ animation: hide{i} {CYCLE}s infinite; }}
""")
    typed_svg = "".join(f'<tspan class="{"typo" if t else "typed"}">{escape(x)}</tspan>' for x, t in typed)
    fixed_svg = "".join(f'<tspan class="{"ok" if f else "fixed"}">{escape(t)}</tspan>' for t, f in fixed)
    body.append(f"""
  <clipPath id="c{i}" class="clip"><rect x="{X0}" y="{y - 26}" height="36" width="0"/></clipPath>
  <g id="p{i}"><text x="{X0 - 30}" y="{y}" class="prompt">›</text></g>
  <g id="t{i}" class="typed-g" clip-path="url(#c{i})"><text x="{X0}" y="{y}" xml:space="preserve">{typed_svg}</text></g>
  <g id="f{i}" class="fixed-g"><text x="{X0}" y="{y}" xml:space="preserve">{fixed_svg}</text></g>""")

H = Y0 + LH * len(LINES) + 8
svg = f"""<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" role="img" aria-label="Sloppy typing corrected as you type by QuipKeys">
<style>{''.join(css)}</style>
<defs><linearGradient id="bg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#161b22"/><stop offset="1" stop-color="#0d1117"/></linearGradient></defs>
<rect x="1" y="1" width="{W - 2}" height="{H - 2}" rx="14" fill="url(#bg)" stroke="#30363d"/>
<circle cx="26" cy="26" r="6" fill="#ff5f57"/><circle cx="46" cy="26" r="6" fill="#febc2e"/><circle cx="66" cy="26" r="6" fill="#28c840"/>
<text x="{W - 20}" y="31" text-anchor="end" class="label">QUIPKEYS · FIXED AS YOU PRESS SPACE</text>
{''.join(body)}
</svg>
"""
open("docs/assets/demo.svg", "w").write(svg)
print("wrote docs/assets/demo.svg", len(svg), "bytes")
