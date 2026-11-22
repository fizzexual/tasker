"""Draws Tasker's logo and writes every icon file into src-tauri/icons.

The mark: a green T followed by a white terminal block cursor, on a black tile. It is defined on
a 16-unit grid and drawn separately for each size, snapped to that size's own pixels, so the
16 px icon is as sharp as the 256 px one (a big picture scaled down turns to a smudge).

Run from the project root:  python tools/make-logo.py        (needs Pillow)
"""
from pathlib import Path

from PIL import Image, ImageDraw

BLACK = (0, 0, 0, 255)
RIM = (58, 58, 58, 255)      # keeps the black tile visible on a dark taskbar
WHITE = (244, 244, 244, 255)
GREEN = (115, 210, 22, 255)  # Tango chameleon: the app's green, one step brighter for small sizes

SS = 8  # drawn 8x larger, then scaled down: smooth corners
OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"


def snap(units: float, size: int) -> int:
    """Grid units (0..16) to drawing pixels; on small icons edges land on whole pixels."""
    px = units * size / 16
    if size <= 48:
        px = round(px)
    return int(round(px * SS))


def block(d: ImageDraw.ImageDraw, size: int, x0: float, y0: float, x1: float, y1: float, fill) -> None:
    a, b, c, e = snap(x0, size), snap(y0, size), snap(x1, size), snap(y1, size)
    if c > a and e > b:
        d.rectangle([a, b, c - 1, e - 1], fill=fill)


def render(size: int) -> Image.Image:
    full = size * SS
    img = Image.new("RGBA", (full, full), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    radius = int(full * 0.20)
    rim = max(SS, int(round(size / 64)) * SS)  # 1 px up to 64 px icons, thicker above
    d.rounded_rectangle([0, 0, full - 1, full - 1], radius=radius, fill=RIM)
    d.rounded_rectangle([rim, rim, full - 1 - rim, full - 1 - rim], radius=max(0, radius - rim), fill=BLACK)
    block(d, size, 2.5, 3.5, 10.5, 6, GREEN)   # T: cap
    block(d, size, 5, 6, 8, 13, GREEN)         # T: stem
    block(d, size, 11, 9, 13.5, 13, WHITE)     # the cursor after it
    return img.resize((size, size), Image.LANCZOS)


PNG = {
    "32x32.png": 32, "64x64.png": 64, "128x128.png": 128, "128x128@2x.png": 256, "icon.png": 512,
    "Square30x30Logo.png": 30, "Square44x44Logo.png": 44, "Square71x71Logo.png": 71,
    "Square89x89Logo.png": 89, "Square107x107Logo.png": 107, "Square142x142Logo.png": 142,
    "Square150x150Logo.png": 150, "Square284x284Logo.png": 284, "Square310x310Logo.png": 310,
    "StoreLogo.png": 50,
}
ICO = [16, 20, 24, 32, 40, 48, 64, 128, 256]
ICNS = [16, 32, 64, 128, 256, 512, 1024]

if __name__ == "__main__":
    for name, size in PNG.items():
        render(size).save(OUT / name)
    frames = [render(s) for s in ICO]
    frames[-1].save(OUT / "icon.ico", format="ICO", append_images=frames[:-1], sizes=[(s, s) for s in ICO])
    big = [render(s) for s in ICNS]
    big[-1].save(OUT / "icon.icns", format="ICNS", append_images=big[:-1])
    print(f"wrote {len(PNG)} png, icon.ico {ICO}, icon.icns to {OUT}")
