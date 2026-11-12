"""The installer's art: the tall picture on its first and last pages, and the small one in the corner of the others.

    python art/installer/installer.py

Writes app/installer/sidebar.bmp (164x314) and app/installer/header.bmp (150x57), the sizes and 24-bit BMP format
the NSIS installer wants, from her cut-out and her icon.
"""

import os

from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.join(os.path.dirname(__file__), "..", "..")
OUT = os.path.join(ROOT, "app", "installer")
INK = (11, 6, 20)
PANEL = (21, 12, 34)
CRIMSON = (246, 6, 85)
VIOLET = (138, 31, 209)
GREEN = (8, 245, 98)
DIM = (169, 155, 184)


def font(size, bold=True):
    for name in (["bahnschrift.ttf", "segoeuib.ttf"] if bold else ["bahnschrift.ttf", "segoeui.ttf"]):
        path = os.path.join(os.environ.get("WINDIR", "C:\\Windows"), "Fonts", name)
        if os.path.exists(path):
            f = ImageFont.truetype(path, size)
            if bold and name == "bahnschrift.ttf":
                try:
                    f.set_variation_by_name("Bold")
                except Exception:
                    pass
            return f
    return ImageFont.load_default()


def backdrop(w, h):
    img = Image.new("RGB", (w, h), INK)
    px = img.load()
    for y in range(h):
        t = y / max(1, h - 1)
        for x in range(w):
            px[x, y] = tuple(int(INK[i] + (PANEL[i] - INK[i]) * t) for i in range(3))
    # Faint scanlines, like her panels.
    d = ImageDraw.Draw(img)
    for y in range(0, h, 3):
        d.line([(0, y), (w, y)], fill=(16, 9, 28))
    return img


def glow(size, center, radius, color):
    layer = Image.new("RGBA", size, (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    cx, cy = center
    d.ellipse([cx - radius, cy - radius, cx + radius, cy + radius], fill=color + (150,))
    return layer.filter(ImageFilter.GaussianBlur(radius * 0.6))


def sidebar():
    w, h = 164, 314
    img = backdrop(w, h).convert("RGBA")
    img.alpha_composite(glow((w, h), (96, 210), 70, VIOLET))
    img.alpha_composite(glow((w, h), (70, 120), 50, CRIMSON))
    capri = Image.open(os.path.join(ROOT, "art", "reference", "capri-cutout.png")).convert("RGBA")
    scale = 262 / capri.height
    capri = capri.resize((round(capri.width * scale), 262), Image.LANCZOS)
    # A hard violet shadow behind her, offset, like the rest of her look.
    shadow = Image.new("RGBA", capri.size, VIOLET + (0,))
    shadow.putalpha(capri.getchannel("A").point(lambda a: 170 if a > 40 else 0))
    x, y = w - capri.width + 6, h - capri.height
    img.alpha_composite(shadow, (x + 4, y + 3))
    img.alpha_composite(capri, (x, y))
    d = ImageDraw.Draw(img)
    d.rectangle([0, 0, 3, h], fill=CRIMSON)
    d.text((14, 16), "CAPRICORN", font=font(19), fill=CRIMSON)
    d.text((15, 40), "your desktop agent", font=font(11, bold=False), fill=DIM)
    d.rectangle([14, 58, 46, 60], fill=GREEN)
    return img.convert("RGB")


def header():
    w, h = 150, 57
    img = backdrop(w, h).convert("RGBA")
    icon = Image.open(os.path.join(ROOT, "app", "icons", "128x128@2x.png")).convert("RGBA").resize((46, 46), Image.LANCZOS)
    img.alpha_composite(glow((w, h), (124, 28), 22, VIOLET))
    img.alpha_composite(icon, (w - 52, 5))
    d = ImageDraw.Draw(img)
    d.text((8, 14), "CAPRICORN", font=font(15), fill=CRIMSON)
    d.text((9, 33), "v1.0", font=font(10, bold=False), fill=DIM)
    d.rectangle([0, h - 2, w, h], fill=CRIMSON)
    return img.convert("RGB")


if __name__ == "__main__":
    os.makedirs(OUT, exist_ok=True)
    sidebar().save(os.path.join(OUT, "sidebar.bmp"))
    header().save(os.path.join(OUT, "header.bmp"))
    print("wrote", OUT)
