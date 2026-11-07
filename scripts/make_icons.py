#!/usr/bin/env python3
"""Draw the QuipKeys app icon (white "Q" mark on a blue rounded square) and
write app/macos/QuipKeys.icns and app/windows/quipkeys.ico.

  python3 scripts/make_icons.py      (needs Pillow)
"""
from PIL import Image, ImageDraw

S = 1024
SS = 4  # supersampling


def icon(size=S):
    n = size * SS
    img = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    # macOS icon grid: the tile is ~80% of the canvas, corner radius ~22.5%.
    m = int(n * 0.1)
    tile = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    grad = Image.new("RGBA", (n, n))
    top, bot = (0x5a, 0x86, 0xff), (0x2f, 0x58, 0xe0)
    gd = ImageDraw.Draw(grad)
    for y in range(n):
        t = y / (n - 1)
        gd.line([(0, y), (n, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(top, bot)) + (255,))
    mask = Image.new("L", (n, n), 0)
    ImageDraw.Draw(mask).rounded_rectangle([m, m, n - m, n - m], radius=int((n - 2 * m) * 0.225), fill=255)
    tile.paste(grad, (0, 0), mask)
    img.alpha_composite(tile)
    # The Q: same geometry as the menu-bar icon (ring + crossing tail), on a 64 grid.
    d = ImageDraw.Draw(img)
    k = (n - 2 * m) / 64 * 0.62
    ox = oy = n / 2 - 32 * k + 2 * k
    cx, cy = ox + 28 * k, oy + 28 * k
    ro, ri = 24 * k, 15 * k
    white = (255, 255, 255, 255)
    d.ellipse([cx - ro, cy - ro, cx + ro, cy + ro], fill=white)
    d.ellipse([cx - ri, cy - ri, cx + ri, cy + ri], fill=(0, 0, 0, 0))
    # punch the hole through to the tile colour
    hole = Image.new("L", (n, n), 0)
    ImageDraw.Draw(hole).ellipse([cx - ri, cy - ri, cx + ri, cy + ri], fill=255)
    img.paste(tile, (0, 0), hole)
    d = ImageDraw.Draw(img)
    w = 10 * k
    a, b = (ox + 35 * k, oy + 35 * k), (ox + 56 * k, oy + 56 * k)
    steps = 200  # a round-capped stroke: stamp discs along the segment
    for i in range(steps + 1):
        t = i / steps
        x, y = a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t
        d.ellipse([x - w / 2, y - w / 2, x + w / 2, y + w / 2], fill=white)
    return img.resize((size, size), Image.LANCZOS)


if __name__ == "__main__":
    import os
    big = icon()
    os.makedirs("app/macos", exist_ok=True)
    os.makedirs("app/windows", exist_ok=True)
    big.save("app/macos/QuipKeys.icns", sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)])
    os.makedirs("docs/assets", exist_ok=True)
    icon(256).save("docs/assets/icon.png")
    big.save("app/windows/quipkeys.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    print("wrote app/macos/QuipKeys.icns, app/windows/quipkeys.ico")
