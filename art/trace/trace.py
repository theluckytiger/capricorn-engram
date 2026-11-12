"""Trace the cut-out into flat colour polygons.

Step one of her rig: every flat region of the reference becomes a polygon in the show's palette. The output is a
reference layer (SVG + a big PNG preview); splitting it into parts and bones happens in rig.py.

    art/.venv/Scripts/python art/trace/trace.py <cutout.png> art/out
"""

import json
import sys
from pathlib import Path

import cv2
import numpy as np
from PIL import Image

# What the cut-out's muted colours mean, and the show colour each one becomes (sampled from frames of the show).
CLASSES = {
    "crimson": {"ref": [(150, 12, 65), (155, 46, 93), (160, 103, 131), (109, 12, 71)], "show": "#f60655"},
    "ink": {"ref": [(14, 9, 25)], "show": "#0b0614"},
    "shadow": {"ref": [(58, 7, 67), (77, 8, 69)], "show": "#48096c"},
    "green": {"ref": [(13, 132, 68)], "show": "#08f562"},
}
SCALE = 4  # traced at 4x so simplified corners stay sharp
EPSILON = 0.9  # polygon simplification, in source pixels
MIN_AREA = 6  # source pixels; smaller specks are dropped (the mane's specks are drawn by the shader instead)


def classify(rgba: np.ndarray) -> np.ndarray:
    """Per pixel: index into CLASSES, or -1 where she isn't."""
    names = list(CLASSES)
    refs, owner = [], []
    for i, name in enumerate(names):
        for c in CLASSES[name]["ref"]:
            refs.append(c)
            owner.append(i)
    refs = np.array(refs, dtype=np.float32)
    owner = np.array(owner)
    rgb = rgba[..., :3].astype(np.float32)
    dist = ((rgb[..., None, :] - refs[None, None]) ** 2).sum(-1)
    out = owner[dist.argmin(-1)]
    out[rgba[..., 3] < 128] = -1
    return out


def fold_shadow(labels: np.ndarray) -> np.ndarray:
    """The cut-out has the violet offset baked in. Her renderer draws it live (a misregistered copy under her), so
    here it goes away: shadow touching the background becomes background, shadow inside her takes the nearest colour."""
    shadow = list(CLASSES).index("shadow")
    out = labels.copy()
    sh = labels == shadow
    bg = (labels == -1).astype(np.uint8)
    near_bg = cv2.dilate(bg, np.ones((3, 3), np.uint8)).astype(bool) & sh
    out[near_bg] = -1
    inner = sh & ~near_bg
    if inner.any():
        dists = []
        for i in range(len(CLASSES)):
            if i == shadow:
                dists.append(np.full(labels.shape, np.inf, np.float32))
                continue
            not_i = (labels != i).astype(np.uint8)
            dists.append(cv2.distanceTransform(not_i, cv2.DIST_L2, 3))
        nearest = np.argmin(np.array(dists), axis=0)
        out[inner] = nearest[inner]
    return out


def smooth_upscale(labels: np.ndarray) -> np.ndarray:
    """Upscale the label map and take a per-class majority, so edges come out as lines instead of stairs."""
    h, w = labels.shape
    votes = []
    for i in range(-1, len(CLASSES)):
        m = (labels == i).astype(np.float32)
        m = cv2.resize(m, (w * SCALE, h * SCALE), interpolation=cv2.INTER_CUBIC)
        votes.append(cv2.GaussianBlur(m, (0, 0), SCALE * 0.45))
    return np.array(votes).argmax(0) - 1


def simplify(contour) -> list:
    approx = cv2.approxPolyDP(contour, EPSILON * SCALE, True)[:, 0, :] / SCALE
    return [[round(float(x), 2), round(float(y), 2)] for x, y in approx]


def polygons(big: np.ndarray):
    """Each region as {class, points, holes}. RETR_CCOMP gives outer boundaries and their holes one level down."""
    out = []
    for i, name in enumerate(CLASSES):
        mask = (big == i).astype(np.uint8) * 255
        contours, hierarchy = cv2.findContours(mask, cv2.RETR_CCOMP, cv2.CHAIN_APPROX_NONE)
        if hierarchy is None:
            continue
        hierarchy = hierarchy[0]
        for k, c in enumerate(contours):
            if hierarchy[k][3] >= 0 or cv2.contourArea(c) < MIN_AREA * SCALE * SCALE:
                continue
            holes = [
                simplify(contours[j])
                for j in range(len(contours))
                if hierarchy[j][3] == k and cv2.contourArea(contours[j]) >= MIN_AREA * SCALE * SCALE
            ]
            out.append({"class": name, "points": simplify(c), "holes": holes})
    # Big shapes first so small ones draw on top.
    out.sort(key=lambda p: -abs(cv2.contourArea(np.array(p["points"], dtype=np.float32))))
    return out


def write_svg(polys, w, h, path: Path):
    body = []
    for p in polys:
        rings = [p["points"], *p["holes"]]
        d = " ".join("M" + " L".join(f"{x},{y}" for x, y in ring) + " Z" for ring in rings)
        body.append(f'<path d="{d}" fill="{CLASSES[p["class"]]["show"]}" fill-rule="evenodd"/>')
    path.write_text(f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}">\n' + "\n".join(body) + "\n</svg>\n")


def write_preview(polys, w, h, path: Path, ref: np.ndarray, zoom=3):
    """Reference on the left, trace on the right, on a mid grey so every colour reads."""
    canvas = np.full((h * zoom, w * zoom * 2, 3), 64, np.uint8)
    canvas[:, : w * zoom] = 64
    refbig = cv2.resize(ref, (w * zoom, h * zoom), interpolation=cv2.INTER_NEAREST)
    a = refbig[..., 3:4] / 255.0
    canvas[:, : w * zoom] = (refbig[..., :3] * a + 64 * (1 - a)).astype(np.uint8)
    right = canvas[:, w * zoom :].copy()
    for p in polys:
        col = tuple(int(CLASSES[p["class"]]["show"][k : k + 2], 16) for k in (1, 3, 5))
        layer = np.zeros(right.shape[:2], np.uint8)
        cv2.fillPoly(layer, [(np.array(p["points"]) * zoom).astype(np.int32)], 255, lineType=cv2.LINE_AA)
        for hole in p["holes"]:
            cv2.fillPoly(layer, [(np.array(hole) * zoom).astype(np.int32)], 0, lineType=cv2.LINE_AA)
        a = layer[..., None] / 255.0
        right = (np.array(col) * a + right * (1 - a)).astype(np.uint8)
    canvas[:, w * zoom :] = right
    Image.fromarray(canvas).save(path)


def main():
    src, out = Path(sys.argv[1]), Path(sys.argv[2])
    keep_shadow = "--keep-shadow" in sys.argv
    out.mkdir(parents=True, exist_ok=True)
    rgba = np.array(Image.open(src).convert("RGBA"))
    h, w = rgba.shape[:2]
    labels = classify(rgba)
    if not keep_shadow:
        labels = fold_shadow(labels)
    polys = polygons(smooth_upscale(labels))
    (out / "trace.json").write_text(json.dumps({"width": w, "height": h, "palette": {k: v["show"] for k, v in CLASSES.items()}, "polygons": polys}))
    write_svg(polys, w, h, out / "trace.svg")
    write_preview(polys, w, h, out / "trace-preview.png", rgba)
    counts = {k: sum(1 for p in polys if p["class"] == k) for k in CLASSES}
    print(f"{len(polys)} polygons, {sum(len(p['points']) for p in polys)} points", counts)


if __name__ == "__main__":
    main()
