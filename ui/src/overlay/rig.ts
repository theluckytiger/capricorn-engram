// Her puppet: the rig file (parts on bones), a pose (an angle per bone), and drawing it.
//
// Parts aren't nested under their bones in the scene: every part is posed by hand (forward kinematics below) and
// drawn into one flat list sorted by z, so a hand on one bone can sit in front of a torso on another.

import { Graphics } from "pixi.js";

export type Pt = [number, number];
type Ring = Pt[];
interface Shape { fill: string; rings: Ring[] }
interface Part { name: string; bone: string; z: number; shapes: Shape[] }
interface Bone { name: string; parent: string | null; x: number; y: number }

export interface RigFile {
  version: number;
  size: [number, number];
  palette: Record<string, string>;
  bones: Bone[];
  parts: Part[];
}

/** Angles in degrees (positive = clockwise on screen), and how far she stands from her rest pose. */
export interface Pose {
  x: number; // where her root (hips) is, in CSS px
  y: number;
  scale: number; // CSS px per rig px
  flip: boolean; // facing the other way
  /** Squash and stretch around her hips (1 = as drawn; a negative sx mirrors her, mid-turn). */
  sx: number;
  sy: number;
  angles: Record<string, number>;
  /** Mane: how much it spreads (0 = rest) and sways. */
  flare: number;
  sway: number;
  /** Which of the three boil drawings (the hand-redrawn wobble), and how strong. */
  boil: number;
  boilAmount: number;
  /** Stretched (`along` > 1) or squashed along the screen direction `u`, `across` the other way, around the screen
   *  point `at`: thrown, she's one long streak along her flight; hitting something, she flattens against it. */
  squeeze?: { u: Pt; along: number; across: number; at: Pt };
}

// 2D affine matrix [a, b, c, d, tx, ty]: x' = a x + c y + tx, y' = b x + d y + ty.
type M = [number, number, number, number, number, number];
const mul = (m: M, n: M): M => [
  m[0] * n[0] + m[2] * n[1], m[1] * n[0] + m[3] * n[1],
  m[0] * n[2] + m[2] * n[3], m[1] * n[2] + m[3] * n[3],
  m[0] * n[4] + m[2] * n[5] + m[4], m[1] * n[4] + m[3] * n[5] + m[5],
];
const translate = (x: number, y: number): M => [1, 0, 0, 1, x, y];
const rotate = (deg: number): M => {
  const r = (deg * Math.PI) / 180, c = Math.cos(r), s = Math.sin(r);
  return [c, s, -s, c, 0, 0];
};
const scale = (sx: number, sy: number): M => [sx, 0, 0, sy, 0, 0];
const apply = (m: M, [x, y]: Pt): Pt => [m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]];

// Small deterministic noise, so a vertex wobbles the same way every time drawing n comes round.
const hash = (i: number, n: number) => {
  const s = Math.sin(i * 127.1 + n * 311.7) * 43758.5453;
  return s - Math.floor(s);
};

/** A speck of her sparkle: where it sits on a shape (rig px) and the vertex nearest it (whose mane spread it shares). */
interface Speck { p: Pt; vi: number }

/** How many specks per screen px² of her crimson: a fixed field, and a pool a third of which twinkles each drawing
 *  (the earlier Capri's shader: 2.4% of pixels fixed, 0.7% twinkling). */
const FIXED = 0.024;
const TWINKLE = 0.021;

export class Rig {
  readonly file: RigFile;
  private bones: Map<string, Bone>;
  /** Per shape (in draw order): her specks, made for `speckScale` (made again when her size changes a lot). */
  private specks: { fixed: Speck[]; pool: Speck[] }[] | null = null;
  private speckScale = 0;
  /** Where her mane spreads from. */
  private maneCenter: Pt;

  constructor(file: RigFile) {
    this.file = file;
    this.bones = new Map(file.bones.map((b) => [b.name, b]));
    const head = this.bones.get("head")!;
    this.maneCenter = [head.x, head.y - 18];
  }

  static async load(url: string) {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`rig: ${res.status}`);
    return new Rig(await res.json());
  }

  /** World matrix of every bone for this pose. */
  private solve(pose: Pose): Map<string, M> {
    const root = this.bones.get("root")!;
    const sx = pose.scale * (pose.flip ? -1 : 1) * (pose.sx ?? 1);
    let base = mul(mul(translate(pose.x, pose.y), scale(sx, pose.scale * (pose.sy ?? 1))), translate(-root.x, -root.y));
    if (pose.squeeze) {
      const { u, along, across, at } = pose.squeeze;
      const [nx, ny] = [-u[1], u[0]];
      // across·n nᵀ + along·u uᵀ, around `at`.
      const m: M = [across * nx * nx + along * u[0] * u[0], across * nx * ny + along * u[0] * u[1], across * nx * ny + along * u[0] * u[1], across * ny * ny + along * u[1] * u[1], 0, 0];
      base = mul(mul(mul(translate(at[0], at[1]), m), translate(-at[0], -at[1])), base);
    }
    const world = new Map<string, M>();
    const visit = (b: Bone) => {
      if (world.has(b.name)) return;
      const parent = b.parent ? (visit(this.bones.get(b.parent)!), world.get(b.parent)!) : base;
      // Turn around the bone's own pivot: move there, rotate, move back.
      const local = mul(mul(translate(b.x, b.y), rotate(pose.angles[b.name] ?? 0)), translate(-b.x, -b.y));
      world.set(b.name, mul(parent, local));
    };
    this.file.bones.forEach(visit);
    return world;
  }

  /** The bone matrices of the last `posed()` call, for drawing things that ride on a bone (her face). */
  private last = new Map<string, M>();

  /** Points given in rig space on `bone` (rest coordinates), as they are on screen in the last pose. */
  onBone(bone: string, points: Pt[]): Pt[] {
    const m = this.last.get(bone);
    return m ? points.map((p) => apply(m, p)) : points;
  }

  /** Every ring of every part, posed and deformed, in CSS px, in draw order. */
  posed(pose: Pose): { fill: string; rings: Ring[]; part: string; specks?: Pt[]; pool?: Pt[] }[] {
    const world = this.solve(pose);
    this.last = world;
    if (!this.specks || Math.abs(pose.scale - this.speckScale) > this.speckScale * 0.25) this.makeSpecks(pose.scale);
    const out: { fill: string; rings: Ring[]; part: string; specks?: Pt[]; pool?: Pt[] }[] = [];
    let vi = 0;
    let si = 0;
    for (const part of this.file.parts) {
      const m = world.get(part.bone)!;
      for (const shape of part.shapes) {
        const sp = this.specks![si++];
        const place = (s: Speck): Pt => apply(m, part.name === "mane" ? this.mane(s.p, s.vi, pose) : s.p);
        const rings = shape.rings.map((ring) =>
          ring.map((v) => {
            vi++;
            let p: Pt = v;
            if (part.name === "mane") p = this.mane(p, vi, pose);
            if (pose.boilAmount > 0) {
              p = [p[0] + (hash(vi, pose.boil) - 0.5) * pose.boilAmount, p[1] + (hash(vi + 7, pose.boil) - 0.5) * pose.boilAmount];
            }
            return apply(m, p);
          }),
        );
        out.push({ fill: shape.fill, rings, part: part.name, specks: sp.fixed.length ? sp.fixed.map(place) : undefined, pool: sp.pool.length ? sp.pool.map(place) : undefined });
      }
    }
    return out;
  }

  /** Scatters her specks over every crimson shape (holes left out), as many per screen px² at `scale` as the earlier
   *  Capri had; each remembers its nearest vertex. Same specks every time (a seeded random), so they stay put. */
  private makeSpecks(scale: number) {
    this.speckScale = scale;
    const perRig = scale * scale;
    let vi = 0;
    let seed = 7;
    const rand = () => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      return seed / 0x7fffffff;
    };
    const inside = (rings: Ring[], x: number, y: number) => {
      let n = false;
      for (const r of rings)
        for (let i = 0, j = r.length - 1; i < r.length; j = i++) {
          const [xi, yi] = r[i], [xj, yj] = r[j];
          if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) n = !n;
        }
      return n;
    };
    const area = (r: Ring) => Math.abs(r.reduce((a, [x, y], i) => { const [x2, y2] = r[(i + 1) % r.length]; return a + x * y2 - x2 * y; }, 0)) / 2;
    this.specks = [];
    for (const part of this.file.parts) {
      for (const shape of part.shapes) {
        const first = vi + 1;
        const verts = shape.rings.flat();
        vi += verts.length;
        if (shape.fill !== "crimson") {
          this.specks.push({ fixed: [], pool: [] });
          continue;
        }
        const [outer, ...holes] = shape.rings;
        const a = Math.max(0, area(outer) - holes.reduce((s, h) => s + area(h), 0));
        const xs = outer.map((p) => p[0]), ys = outer.map((p) => p[1]);
        const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
        const scatter = (count: number) => {
          const got: Speck[] = [];
          for (let tries = 0; got.length < count && tries < count * 30; tries++) {
            const p: Pt = [x0 + rand() * (x1 - x0), y0 + rand() * (y1 - y0)];
            if (!inside(shape.rings, p[0], p[1])) continue;
            let best = 0;
            for (let k = 1; k < verts.length; k++) if (Math.hypot(verts[k][0] - p[0], verts[k][1] - p[1]) < Math.hypot(verts[best][0] - p[0], verts[best][1] - p[1])) best = k;
            got.push({ p, vi: first + best });
          }
          return got;
        };
        this.specks.push({ fixed: scatter(Math.round(a * perRig * FIXED)), pool: scatter(Math.round(a * perRig * TWINKLE)) });
      }
    }
  }

  /** The mane spreads unevenly from around her head and sways more the further out a point is. */
  private mane([x, y]: Pt, i: number, pose: Pose): Pt {
    const [cx, cy] = this.maneCenter;
    const dx = x - cx, dy = y - cy;
    const r = Math.hypot(dx, dy);
    const spread = 1 + pose.flare * (0.55 + 0.9 * hash(i, 0));
    const a = ((pose.sway * Math.min(r / 140, 1.2)) * Math.PI) / 180;
    const c = Math.cos(a), s = Math.sin(a);
    const ex = dx * spread, ey = dy * spread;
    return [cx + ex * c - ey * s, cy + ex * s + ey * c];
  }

  /** The rest pose at a place and size. */
  restPose(x: number, y: number, scale: number): Pose {
    return { x, y, scale, flip: false, sx: 1, sy: 1, angles: {}, flare: 0, sway: 0, boil: 0, boilAmount: 0 };
  }
}

/** Draws posed shapes: first the violet misregistered copy, then her colours on top, and on her crimson the specks of
 *  her sparkle (`sparkle`: 1 as usual, brighter when patted, dim asleep; a third of the twinkling pool shows each
 *  drawing, by `seed`), each right after its own shape, so whatever is in front of it covers them. */
export function draw(g: Graphics, shadow: Graphics, shapes: ReturnType<Rig["posed"]>, palette: Record<string, string>, offset: number, sparkle = 1, seed = 0) {
  g.clear();
  shadow.clear();
  for (const s of shapes) {
    const [outer, ...holes] = s.rings;
    shadow.poly(outer.flatMap(([x, y]) => [x + offset, y + offset]));
    shadow.fill({ color: VIOLET });
    for (const h of holes) {
      shadow.poly(h.flatMap(([x, y]) => [x + offset, y + offset]));
      shadow.cut();
    }
    g.poly(outer.flat());
    g.fill({ color: palette[s.fill] });
    for (const h of holes) {
      g.poly(h.flat());
      g.cut();
    }
    if (sparkle > 0 && (s.specks || s.pool) && s.fill === "crimson") {
      // One screen pixel each, whatever her size.
      let any = false;
      for (const [x, y] of s.specks ?? []) {
        g.rect(Math.round(x), Math.round(y), 1, 1);
        any = true;
      }
      if (any) g.fill({ color: SPECK, alpha: Math.min(1, 0.85 * sparkle) });
      any = false;
      (s.pool ?? []).forEach(([x, y], i) => {
        if (hash(i, seed) < 0.33) {
          g.rect(Math.round(x), Math.round(y), 1, 1);
          any = true;
        }
      });
      if (any) g.fill({ color: SPECK, alpha: Math.min(1, sparkle) });
    }
  }
}

/** Her specks' colour: nearly white with a touch of pink. */
const SPECK = 0xfff2f7;

export const VIOLET = 0x8a1fd1;

/** Convex hull (monotone chain), for the click-through outline: one small polygon per part. */
export function hull(points: Pt[]): Pt[] {
  if (points.length < 3) return points;
  const p = [...points].sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const cross = (o: Pt, a: Pt, b: Pt) => (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
  const lower: Pt[] = [], upper: Pt[] = [];
  for (const q of p) {
    while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], q) <= 0) lower.pop();
    lower.push(q);
  }
  for (let i = p.length - 1; i >= 0; i--) {
    const q = p[i];
    while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], q) <= 0) upper.pop();
    upper.push(q);
  }
  return lower.slice(0, -1).concat(upper.slice(0, -1));
}
