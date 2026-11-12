"""Her app icon: the green mask slab, the blue almond eyes, the black spike. Drawn as vectors, rendered at 1024 px.

    art/.venv/Scripts/python art/icon/icon.py art/out/icon-1024.png
"""

import sys

import numpy as np
from PIL import Image, ImageDraw

S = 1024
SS = 4  # supersampling, for clean edges

INK = (11, 6, 20, 255)
GREEN = (8, 245, 98, 255)
BLUE = (62, 59, 171, 255)
BLUE_LIGHT = (96, 140, 255, 255)
WHITE = (251, 248, 252, 255)
CRIMSON = (246, 6, 85, 255)
VIOLET = (165, 49, 231, 255)


def poly(d, pts, fill):
    d.polygon([(x * SS, y * SS) for x, y in pts], fill=fill)


def main(out):
    im = Image.new("RGBA", (S * SS, S * SS), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    # a few mane shards behind
    for pts in [
        [(130, 420), (360, 300), (300, 470)],
        [(170, 640), (330, 520), (360, 640)],
        [(860, 300), (700, 330), (760, 450)],
        [(900, 600), (720, 560), (700, 680)],
        [(560, 40), (600, 260), (480, 250)],
    ]:
        poly(d, [(x + 14, y + 14) for x, y in pts], VIOLET)
        poly(d, pts, CRIMSON)
    # head: black, with the hooked spike up and to the right
    head = [(250, 300), (600, 240), (700, 120), (690, 300), (820, 330), (760, 760), (520, 900), (260, 760)]
    poly(d, [(x + 18, y + 18) for x, y in head], VIOLET)
    poly(d, head, INK)
    # the mask slab, a tilted trapezoid
    mask = [(300, 380), (760, 360), (720, 720), (330, 700)]
    poly(d, mask, GREEN)
    # eye sockets (ink) and the almond eyes
    for cx, cy in [(420, 540), (610, 532)]:
        poly(d, [(cx - 95, cy - 20), (cx - 20, cy - 85), (cx + 80, cy - 40), (cx + 90, cy + 40), (cx - 10, cy + 85), (cx - 85, cy + 50)], INK)
        d.ellipse([(cx - 58) * SS, (cy - 46) * SS, (cx + 58) * SS, (cy + 46) * SS], fill=BLUE)
        d.ellipse([(cx - 40) * SS, (cy - 32) * SS, (cx + 40) * SS, (cy + 32) * SS], fill=BLUE_LIGHT)
        d.ellipse([(cx - 20) * SS, (cy - 20) * SS, (cx + 20) * SS, (cy + 20) * SS], fill=INK)
        d.ellipse([(cx - 34) * SS, (cy - 38) * SS, (cx - 14) * SS, (cy - 18) * SS], fill=WHITE)
    # the zigzag crack across the brow, and the fang
    zig = [(380, 430), (440, 405), (470, 440), (530, 400), (560, 435), (630, 405)]
    d.line([(x * SS, y * SS) for x, y in zig], fill=WHITE, width=12 * SS, joint="curve")
    poly(d, [(520, 760), (585, 770), (548, 850)], WHITE)
    im = im.resize((S, S), Image.LANCZOS)
    im.save(out)
    print(out, np.array(im)[..., 3].mean().round(1))


if __name__ == "__main__":
    main(sys.argv[1])
