#!/usr/bin/env python3
"""Render the QuipKeys logo (docs/assets/logo.svg) into every icon the project
ships: app/macos/QuipKeys.icns, app/windows/quipkeys.ico, docs/assets/icon.png.
Needs Pillow and Node with Playwright (headless Chromium renders the SVG).

  python3 scripts/make_icons.py
"""
import os, subprocess, tempfile
from PIL import Image

svg = open("docs/assets/logo.svg").read()
tmp = tempfile.mkdtemp()
png = os.path.join(tmp, "logo-1024.png")
js = os.path.join(tmp, "r.js")
open(js, "w").write("""
const { chromium } = require('playwright');
(async () => {
  const b = await chromium.launch();
  const p = await b.newPage({ viewport: { width: 1024, height: 1024 } });
  await p.setContent(`<style>html,body{margin:0;background:transparent}</style>` + process.argv[2].replace('width="64" height="64"', 'width="1024" height="1024"'));
  await p.screenshot({ path: process.argv[3], omitBackground: true });
  await b.close();
})();
""")
subprocess.run(["node", js, svg, png], check=True)
big = Image.open(png).convert("RGBA")
# macOS icons sit inside a ~10% margin on the canvas.
mac = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
mac.alpha_composite(big.resize((824, 824), Image.LANCZOS), (100, 100))
os.makedirs("app/macos", exist_ok=True)
os.makedirs("app/windows", exist_ok=True)
mac.save("app/macos/QuipKeys.icns", sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)])
big.save("app/windows/quipkeys.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
big.resize((256, 256), Image.LANCZOS).save("docs/assets/icon.png")
print("wrote app/macos/QuipKeys.icns, app/windows/quipkeys.ico, docs/assets/icon.png")
