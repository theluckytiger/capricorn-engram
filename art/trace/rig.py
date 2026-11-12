"""Split the trace into her rig: parts on a skeleton.

Each part is a region of the reference (source pixels, 220x420) and the colour classes it takes. Parts are cut in
order: what an earlier part takes, later ones don't. Bones are the joints the parts turn around. Joint patches (a
little of the parent's shapes around each joint, drawn under the child) keep seams closed when a limb turns.

    art/.venv/Scripts/python art/trace/rig.py art/out/trace.json ui/public/rig/capri.rig.json art/out
"""

import json
import sys
from pathlib import Path

import cv2
import numpy as np
from PIL import Image
from shapely.geometry import MultiPolygon, Point, Polygon
from shapely.ops import unary_union

ALL = ["crimson", "ink", "green"]

# name, parent, rest pivot (x, y). y grows downward, like the image.
BONES = [
    ("root", None, (108, 218)),
    ("spine", "root", (112, 198)),
    ("neck", "spine", (120, 153)),
    ("head", "neck", (121, 146)),
    ("armL_up", "spine", (102, 163)),
    ("armL_fore", "armL_up", (71, 184)),
    ("armL_hand", "armL_fore", (42, 202)),
    ("armR_up", "spine", (134, 174)),
    ("armR_fore", "armR_up", (143, 211)),
    ("armR_hand", "armR_fore", (151, 250)),
    ("legL_thigh", "root", (99, 220)),
    ("legL_shin", "legL_thigh", (98, 300)),
    ("legL_foot", "legL_shin", (104, 386)),
    ("legR_thigh", "root", (118, 228)),
    ("legR_shin", "legR_thigh", (141, 308)),
]

# Cut order matters: earlier parts take their region first. z is the draw order (low = behind).
PARTS = [
    ("head", "head", 90, ALL[1:], [(106, 104), (170, 104), (170, 127), (145, 137), (130, 148), (124, 153), (110, 151), (105, 135)]),
    ("torso", "spine", 60, ALL, [(95, 149), (126, 149), (142, 151), (143, 172), (133, 180), (128, 197), (100, 197), (96, 170)]),
    ("armL_hand", "armL_hand", 80, ALL, [(2, 188), (46, 193), (51, 214), (36, 236), (2, 236)]),
    ("armL_fore", "armL_fore", 79, ALL, [(40, 191), (74, 171), (81, 190), (47, 214)]),
    ("armL_up", "armL_up", 78, ALL, [(72, 171), (98, 151), (107, 162), (109, 177), (81, 192)]),
    ("armR_up", "armR_up", 20, ALL, [(126, 177), (147, 169), (153, 212), (136, 215)]),
    ("armR_fore", "armR_fore", 21, ALL, [(134, 207), (153, 205), (161, 252), (146, 255)]),
    ("armR_hand", "armR_hand", 22, ALL, [(143, 247), (163, 243), (184, 262), (182, 290), (143, 290)]),
    ("legL_shard_a", "legL_shin", 45, ["green"], [(72, 262), (87, 262), (105, 362), (84, 365), (71, 300)]),
    ("legL_shard_b", "legL_shin", 44, ["green"], [(66, 338), (93, 338), (95, 394), (66, 394)]),
    ("legR_shard", "legR_shin", 30, ALL, [(156, 288), (194, 288), (194, 398), (156, 398)]),
    ("pelvis", "root", 55, ALL, [(82, 190), (133, 190), (131, 226), (84, 222)]),
    ("legL_thigh", "legL_thigh", 50, ALL, [(80, 216), (110, 221), (109, 262), (107, 302), (85, 302), (82, 250)]),
    ("legR_thigh", "legR_thigh", 35, ALL, [(108, 221), (133, 221), (141, 262), (154, 313), (131, 317), (111, 266)]),
    ("legL_shin", "legL_shin", 49, ALL, [(80, 295), (116, 295), (122, 384), (96, 389), (80, 386)]),
    ("legL_foot", "legL_foot", 48, ALL, [(90, 379), (126, 379), (128, 421), (88, 421)]),
    ("legR_shin", "legR_shin", 34, ALL, [(124, 305), (156, 305), (176, 421), (140, 421), (127, 350)]),
    ("mane", "head", 0, ALL, [(0, 0), (220, 0), (220, 202), (0, 202)]),
]

PATCH_RADIUS = 5


def to_shapely(p):
    poly = Polygon(p["points"], p["holes"])
    return poly if poly.is_valid else poly.buffer(0)


def rings(geom):
    """shapely geometry -> list of [outer, *holes] rings, as plain lists."""
    if geom.is_empty:
        return []
    geoms = geom.geoms if hasattr(geom, "geoms") else [geom]
    out = []
    for g in geoms:
        if not isinstance(g, Polygon) or g.area < 0.6:
            continue
        g = g.simplify(0.15)
        out.append([[[round(x, 2), round(y, 2)] for x, y in list(r.coords)[:-1]] for r in [g.exterior, *g.interiors]])
    return out


def main():
    trace = json.loads(Path(sys.argv[1]).read_text())
    out_path, preview_dir = Path(sys.argv[2]), Path(sys.argv[3])
    by_class = {c: unary_union([to_shapely(p) for p in trace["polygons"] if p["class"] == c]) for c in ALL}
    remaining = dict(by_class)
    bones = {name: {"name": name, "parent": parent, "x": x, "y": y} for name, parent, (x, y) in BONES}

    parts = []
    for name, bone, z, classes, region in PARTS:
        reg = Polygon(region)
        shapes = []
        for c in classes:
            taken = remaining[c].intersection(reg)
            remaining[c] = remaining[c].difference(reg)
            for r in rings(taken):
                shapes.append({"fill": c, "rings": r})
        parts.append({"name": name, "bone": bone, "z": z, "shapes": shapes})

    # Joint patches: what's around each child joint, from the parent's shapes, drawn just under the child.
    by_name = {p["name"]: p for p in parts}
    patches = []
    for name, parent, (x, y) in BONES:
        if parent is None or name not in by_name or parent not in bones:
            continue
        parent_part = next((p for p in parts if p["bone"] == parent and p["shapes"]), None)
        child_part = by_name[name]
        if parent_part is None:
            continue
        disc = Point(x, y).buffer(PATCH_RADIUS)
        shapes = []
        for c in ALL:
            piece = by_class[c].intersection(disc)
            for r in rings(piece):
                shapes.append({"fill": c, "rings": r})
        if shapes:
            patches.append({"name": f"{name}_patch", "bone": parent, "z": child_part["z"] - 0.5, "shapes": shapes})
    parts += patches

    left = {c: g.area for c, g in remaining.items() if g.area > 1}
    if left:
        print("unassigned area (px²):", {c: round(a, 1) for c, a in left.items()})

    rig = {
        "version": 1,
        "size": [trace["width"], trace["height"]],
        "palette": {k: v for k, v in trace["palette"].items() if k in ALL},
        "bones": list(bones.values()),
        "parts": sorted(parts, key=lambda p: p["z"]),
    }
    out_path.parent.mkdir(parents=True, exist_ok=True)
    out_path.write_text(json.dumps(rig, separators=(",", ":")))
    print(f"{len(rig['parts'])} parts, {sum(len(p['shapes']) for p in rig['parts'])} shapes -> {out_path} ({out_path.stat().st_size // 1024} KB)")
    preview(rig, preview_dir / "rig-parts.png", remaining)


def preview(rig, path, remaining, zoom=3):
    """Each part in its own hue, joints as dots, unassigned leftovers in white."""
    w, h = rig["size"]
    img = np.full((h * zoom, w * zoom, 3), 40, np.uint8)
    rng = np.random.default_rng(3)
    for part in rig["parts"]:
        hue = tuple(int(v) for v in rng.integers(60, 255, 3))
        for s in part["shapes"]:
            layer = np.zeros((h * zoom, w * zoom), np.uint8)
            outer, *holes = s["rings"]
            cv2.fillPoly(layer, [(np.array(outer) * zoom).astype(np.int32)], 255)
            for hole in holes:
                cv2.fillPoly(layer, [(np.array(hole) * zoom).astype(np.int32)], 0)
            img[layer > 0] = hue
    for c, g in remaining.items():
        for r in rings(g):
            cv2.fillPoly(img, [(np.array(r[0]) * zoom).astype(np.int32)], (255, 255, 255))
    for b in rig["bones"]:
        cv2.circle(img, (int(b["x"] * zoom), int(b["y"] * zoom)), 4, (255, 255, 0), -1)
        cv2.putText(img, b["name"], (int(b["x"] * zoom) + 6, int(b["y"] * zoom) - 4), cv2.FONT_HERSHEY_SIMPLEX, 0.35, (255, 255, 0), 1)
    Image.fromarray(img).save(path)


if __name__ == "__main__":
    main()
