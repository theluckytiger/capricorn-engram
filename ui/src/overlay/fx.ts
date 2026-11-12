// Her effects, drawn around her body: sparkles off her mane, dust where she lands, the glitch (her body jumping
// sideways on ones with her violet copy torn away), a flash when she arrives or leaves, and the afterimages of a
// smear frame. Stepped at 24 fps with her acting, so they keep its rhythm.

import type { Graphics } from "pixi.js";
import type { Pt } from "./rig";

const GREEN = 0x08f562;
const CRIMSON = 0xf60655;
const RED = 0xf60655;
const PAPER = 0xfbf8fc;
const VIOLET = 0x8a1fd1;
const INK = 0x0b0614;

/** A click of hers on your PC: the wire from her finger and the reticle on what she pressed (CSS px). */
interface Wire {
  from: () => Pt;
  rect: [number, number, number, number];
  age: number;
}

/** The reticle comes in from wide and slams on in four drawings, bounces once, then holds (px of padding). */
const SLAM = [64, 36, 16, 4, 11, 7];
/** Drawings the wire stays out before reeling back in (over four). */
const WIRE_HOLD = 22;
const SPARK = [0xffffff, 0x9fd0ff, 0x7fb0ff];

interface Particle {
  x: number;
  y: number;
  vx: number;
  vy: number;
  size: number;
  life: number;
  max: number;
  color: number;
}

export class Fx {
  private sparks: Particle[] = [];
  private dust: Particle[] = [];
  private glitchLeft = 0;
  private twitchLeft = 0;
  private flashLeft = 0;
  private flashAt: Pt = [0, 0];
  private jitter = 0;
  private wires: Wire[] = [];
  private rings: { x: number; y: number; size: number; age: number }[] = [];
  private shards: (Particle & { a: number; spin: number })[] = [];
  private cracks: { pts: Pt[][]; age: number }[] = [];
  private lines: { x: number; y: number; r: number; age: number }[] = [];
  private streaks: { at: () => Pt; dir: Pt; len: number; age: number }[] = [];
  private notes: Particle[] = [];
  private sky: { from: Pt; stars: { at: Pt; to: Pt }[]; w: number; age: number; hold?: boolean } | null = null;
  private passing: { from: Pt; to: Pt; age: number; frames: number; flare: number; size: number; broke: number } | null = null;

  /** The satellite going over: from `from` to `to` in `frames` (at 24 fps), fading in and out, `size` times as big as
   *  in her dream. Its place `f` frames from now is `passingAt(f)`. */
  pass(from: Pt, to: Pt, frames: number, size = 1) {
    this.passing = { from, to, age: 0, frames, flare: -99, size, broke: -1 };
  }
  /** Hit by her red light: it snaps and falls apart; a while later, where it was, a star twinkles twice. */
  breakPassing() {
    if (this.passing && this.passing.broke < 0) this.passing.broke = this.passing.age;
  }
  passingAt(f = 0): Pt | null {
    const p = this.passing;
    if (!p) return null;
    const k = Math.min(1, (p.age + f) / p.frames);
    return [p.from[0] + (p.to[0] - p.from[0]) * k, p.from[1] + (p.to[1] - p.from[1]) * k];
  }
  /** It twinkles back at her. */
  twinkle() {
    if (this.passing) this.passing.flare = this.passing.age;
  }

  /** Her dream: her mane's specks rise into a constellation, a road of light runs up to the satellite crossing the
   *  sky, and its red light answers. About nine seconds (`w`: the screen's width). */
  satellite(from: Pt, w: number, hold?: number) {
    const stars = Array.from({ length: 8 }, (_, i) => {
      const u = i / 7;
      // A loose arc from above her toward the top of the screen, a little jitter on each.
      const to: Pt = [from[0] + (w * 0.5 - from[0]) * u * 0.8 + (Math.random() - 0.5) * 70, from[1] - 120 - u * (from[1] - 260) + (Math.random() - 0.5) * 50];
      return { at: [from[0] + (Math.random() - 0.5) * 80, from[1] + (Math.random() - 0.5) * 40] as Pt, to };
    });
    this.sky = { from, stars, w, age: hold ?? 0, hold: hold !== undefined };
  }
  private scans: { rect: [number, number, number, number]; age: number }[] = [];

  /** A music note popping off her, rising and fading. */
  note(at: Pt, scale: number) {
    // From above her mane (red would vanish into it): green and white.
    this.notes.push({ x: at[0] + (Math.random() - 0.5) * 200 * scale, y: at[1] - (90 + Math.random() * 40) * scale, vx: (Math.random() - 0.5) * 0.9, vy: -1.1 - Math.random() * 0.5, size: (12 + Math.random() * 6) * scale * 1.6, life: 0, max: 28, color: Math.random() < 0.6 ? GREEN : PAPER });
  }

  /** Her looking at a window: a scanline sweeps it top to bottom, a fading band behind it. */
  scan(rect: [number, number, number, number]) {
    this.scans.push({ rect, age: 0 });
  }

  /** Thrown: long tapered lines trailing behind her for two drawings, the way she's going. */
  streak(at: () => Pt, dir: Pt, len: number) {
    this.streaks.push({ at, dir, len, age: 0 });
  }
  private stars: { center: () => Pt; age: number; life: number }[] = [];
  private bolts: { a: () => Pt; b: Pt; age: number }[] = [];
  private flakes: Particle[] = [];

  /** Red lightning from `a` (her finger, wherever it goes) to `b`, forking, re-drawn on twos. */
  bolt(a: () => Pt, b: Pt) {
    this.bolts.push({ a, b, age: 0 });
  }

  private drawPassing(g: Graphics, p: NonNullable<Fx["passing"]>) {
    const fade = Math.min(1, p.age / 12, (p.frames - p.age) / 18);
    const k = p.size;
    if (p.broke >= 0) {
      this.drawBroken(g, p, fade);
      return;
    }
    const [x, y] = this.passingAt()!;
    // Its body and two panels, each with a dark edge so it shows on a bright wallpaper too.
    for (const [rx, ry, rw, rh, color] of [[-7, -4, 14, 8, PAPER], [-30, -2, 20, 4, 0x9fd0ff], [10, -2, 20, 4, 0x9fd0ff]] as const) {
      g.rect(x + (rx - 1.5) * k, y + (ry - 1.5) * k, (rw + 3) * k, (rh + 3) * k).fill({ color: INK, alpha: 0.75 * fade });
      g.rect(x + rx * k, y + ry * k, rw * k, rh * k).fill({ color, alpha: 0.95 * fade });
    }
    if (Math.floor(p.age / 12) % 2 === 0) g.circle(x, y + 7 * k, 3.5 * k).fill({ color: RED, alpha: fade });
    // Twinkling back: a four-pointed flash that grows and fades over a second.
    const t = p.age - p.flare;
    if (t >= 0 && t < 24) {
      const s = (t < 4 ? t / 4 : 1 - (t - 4) / 20) * 30 * k;
      g.poly([x, y - s, x + s * 0.18, y - s * 0.18, x + s, y, x + s * 0.18, y + s * 0.18, x, y + s, x - s * 0.18, y + s * 0.18, x - s, y, x - s * 0.18, y - s * 0.18]).fill({ color: PAPER, alpha: 0.95 * fade });
    }
  }

  /** The satellite snapped: its pieces tumbling down and fading, then a star where it was, twinkling twice. */
  private drawBroken(g: Graphics, p: NonNullable<Fx["passing"]>, fade: number) {
    const k = p.size;
    const t = p.age - p.broke;
    const k0 = Math.min(1, p.broke / p.frames);
    const [x, y]: Pt = [p.from[0] + (p.to[0] - p.from[0]) * k0, p.from[1] + (p.to[1] - p.from[1]) * k0];
    if (t < 40) {
      const pieces: [number, number, number, number, number][] = [[-30, -2, 20, 4, 0x9fd0ff], [10, -2, 20, 4, 0x9fd0ff], [-7, -4, 7, 8, PAPER], [0, -4, 7, 8, PAPER], [-3, 5, 6, 4, RED], [4, 1, 5, 3, PAPER]];
      pieces.forEach(([rx, ry, rw, rh, color], i) => {
        const vx = (rx + rw / 2) * 0.12 + (i % 2 ? 0.6 : -0.6);
        const [px, py] = [x + (rx + vx * t * 1.6) * k, y + (ry - 2.2 * t + 0.16 * t * t) * k];
        g.rect(px, py, rw * k, rh * k).fill({ color, alpha: (1 - t / 40) * fade });
      });
    }
    for (const at of [36, 60]) {
      const u = t - at;
      if (u < 0 || u >= 16) continue;
      const s = (u < 3 ? u / 3 : 1 - (u - 3) / 13) * 18 * k;
      g.poly([x, y - s, x + s * 0.18, y - s * 0.18, x + s, y, x + s * 0.18, y + s * 0.18, x, y + s, x - s * 0.18, y + s * 0.18, x - s, y, x - s * 0.18, y - s * 0.18]).fill({ color: PAPER, alpha: fade });
    }
  }

  private drawSky(g: Graphics, sky: NonNullable<Fx["sky"]>) {
    const f = sky.age;
    const fade = f > 196 ? 1 - (f - 196) / 20 : 1;
    // The satellite crosses the top of the screen the whole time; its red light blinks, and answers at 5 s.
    const sx = -60 + (sky.w + 120) * (f / 216);
    const sy = 90 + Math.sin(f / 30) * 6;
    g.rect(sx - 7, sy - 4, 14, 8).fill({ color: PAPER, alpha: 0.9 * fade });
    g.rect(sx - 30, sy - 2, 20, 4).fill({ color: 0x9fd0ff, alpha: 0.8 * fade });
    g.rect(sx + 10, sy - 2, 20, 4).fill({ color: 0x9fd0ff, alpha: 0.8 * fade });
    const blink = Math.floor(f / 12) % 2 === 0;
    if (blink || (f > 120 && f < 150)) g.circle(sx, sy + 7, 3.5).fill({ color: RED, alpha: fade });
    if (f > 120 && f < 160) {
      for (let k = 0; k < 3; k++) {
        const r = ((f - 120 + k * 8) % 24) * 3;
        g.circle(sx, sy + 7, r).stroke({ color: RED, width: 2, alpha: (1 - r / 72) * fade });
      }
    }
    // Her specks rising to their places (first second and a half), twinkling there.
    const rise = Math.min(1, f / 36);
    const pts: Pt[] = sky.stars.map((s) => [s.at[0] + (s.to[0] - s.at[0]) * rise, s.at[1] + (s.to[1] - s.at[1]) * rise]);
    pts.forEach(([x, y], i) => {
      const tw = 3 + Math.abs(Math.sin(f / 5 + i)) * 3;
      g.poly([x, y - tw * 1.6, x + tw * 0.35, y - tw * 0.35, x + tw * 1.6, y, x + tw * 0.35, y + tw * 0.35, x, y + tw * 1.6, x - tw * 0.35, y + tw * 0.35, x - tw * 1.6, y, x - tw * 0.35, y - tw * 0.35]).fill({ color: PAPER, alpha: fade });
    });
    // The constellation drawn in, star to star; then the road from the last star up to the satellite.
    if (f > 36) {
      const k = Math.min(pts.length - 1, Math.floor((f - 36) / 4));
      for (let i = 0; i < k; i++) g.poly([...pts[i], ...pts[i + 1]], false).stroke({ color: 0x9fd0ff, width: 1.5, alpha: 0.7 * fade });
    }
    if (f > 72) {
      const last = pts[pts.length - 1];
      const u = Math.min(1, (f - 72) / 40);
      const [ex, ey] = [last[0] + (sx - last[0]) * u, last[1] + (sy - last[1]) * u];
      // Dotted: a short dash every few pixels.
      const n = Math.floor(Math.hypot(ex - last[0], ey - last[1]) / 10);
      for (let i = 0; i < n; i++) {
        const v = i / Math.max(1, n);
        const [x, y] = [last[0] + (ex - last[0]) * v, last[1] + (ey - last[1]) * v];
        g.circle(x, y, 1.6).fill({ color: GREEN, alpha: 0.8 * fade });
      }
    }
  }

  /** Sparks kicked up off the floor where she scrapes along it, flying back the way `side` points (-1, 1). */
  kick(at: Pt, side: number, scale: number, count = 3) {
    for (let i = 0; i < count; i++) {
      this.shards.push({ x: at[0] + (Math.random() - 0.5) * 20 * scale, y: at[1] - 2, vx: side * (3 + Math.random() * 7), vy: -(1.5 + Math.random() * 4), size: (2 + Math.random() * 3) * scale * 2, life: 0, max: 6 + Math.floor(Math.random() * 6), color: Math.random() < 0.6 ? 0xffc04d : PAPER, a: Math.random() * 6, spin: (Math.random() - 0.5) * 0.6 });
    }
  }

  /** Bits of her flying off in every direction (`power` 0..1). */
  burst(at: Pt, scale: number, count = 8, power = 0.5) {
    for (let i = 0; i < count; i++) {
      const a = Math.random() * Math.PI * 2;
      const v = (2 + Math.random() * 6) * (0.4 + power);
      this.shards.push({ x: at[0], y: at[1], vx: Math.cos(a) * v, vy: Math.sin(a) * v - 1.5, size: (3 + Math.random() * 6) * scale * 2, life: 0, max: 8 + Math.floor(Math.random() * 7), color: [RED, PAPER, VIOLET, GREEN][i % 4], a: Math.random() * 6, spin: (Math.random() - 0.5) * 0.8 });
    }
  }

  /** Red sparks off her (a coding AI running a command, going online). */
  redSparks(at: Pt, scale: number) {
    for (let i = 0; i < 6; i++) {
      const a = Math.random() * Math.PI * 2;
      const r = (40 + Math.random() * 60) * scale;
      this.sparks.push({ x: at[0] + Math.cos(a) * r, y: at[1] + Math.sin(a) * r * 0.8, vx: Math.cos(a) * 1.4, vy: Math.sin(a) * 1.2 - 0.5, size: (5 + Math.random() * 6) * scale, life: 0, max: 8 + Math.floor(Math.random() * 6), color: i % 2 ? RED : CRIMSON });
    }
  }

  /** Her specks lighting up (a head pat): little lights winking on across her mane and drifting up. */
  glow(at: Pt, scale: number) {
    for (let i = 0; i < 10; i++) {
      this.flakes.push({ x: at[0] + (Math.random() - 0.5) * 200 * scale, y: at[1] + (Math.random() - 0.6) * 120 * scale, vx: (Math.random() - 0.5) * 0.3, vy: -0.3 - Math.random() * 0.4, size: (3 + Math.random() * 3) * scale * 2, life: 0, max: 24 + Math.floor(Math.random() * 16), color: i % 3 ? 0x9fd0ff : PAPER });
    }
  }

  /** Data shed off her as she reads along: little green bits falling and blinking out. */
  bits(at: Pt, scale: number) {
    for (let i = 0; i < 3; i++) {
      this.flakes.push({ x: at[0] + (Math.random() - 0.5) * 160 * scale, y: at[1] + (Math.random() - 0.2) * 60 * scale, vx: 0, vy: 0.8 + Math.random() * 0.8, size: (2 + Math.random() * 2.5) * scale * 2, life: 0, max: 18 + Math.floor(Math.random() * 10), color: Math.random() < 0.8 ? GREEN : PAPER });
    }
  }

  /** Heat coming off her mane: embers rising and wavering. */
  heat(at: Pt, scale: number) {
    for (let i = 0; i < 4; i++) {
      this.flakes.push({ x: at[0] + (Math.random() - 0.5) * 140 * scale, y: at[1] - Math.random() * 40 * scale, vx: (Math.random() - 0.5) * 0.4, vy: -(0.9 + Math.random() * 0.9), size: (3 + Math.random() * 4) * scale * 2, life: 0, max: 26 + Math.floor(Math.random() * 14), color: Math.random() < 0.6 ? 0xff7a1a : RED });
    }
  }

  /** A shard or two off her mane, drifting down. */
  shed(at: Pt, scale: number) {
    for (let i = 0; i < 1 + Math.round(Math.random()); i++) {
      this.flakes.push({ x: at[0] + (Math.random() - 0.5) * 120 * scale * 2, y: at[1] + (Math.random() - 0.3) * 60 * scale * 2, vx: (Math.random() - 0.5) * 0.6, vy: 0.4 + Math.random() * 0.5, size: (3 + Math.random() * 4) * scale * 2, life: 0, max: 40 + Math.floor(Math.random() * 20), color: Math.random() < 0.8 ? RED : PAPER });
    }
  }

  /** A hit (`hard` 0..1) at `at`, `dir` the way she was going: a ring, a burst of shards, cracks in the glass when it's
   *  hard, speed lines and a glitch tear when it's very hard. */
  impact(at: Pt, dir: Pt, hard: number, scale: number) {
    this.rings.push({ x: at[0], y: at[1], size: 0.25 + hard * 0.8, age: 0 });
    const n = Math.round(5 + hard * 16);
    for (let i = 0; i < n; i++) {
      // Back off the surface, in a fan.
      const a = Math.atan2(-dir[1], -dir[0]) + (Math.random() - 0.5) * 2.4;
      const v = (3 + Math.random() * 9) * (0.4 + hard);
      this.shards.push({ x: at[0], y: at[1], vx: Math.cos(a) * v, vy: Math.sin(a) * v, size: (4 + Math.random() * 9) * scale * 2, life: 0, max: 8 + Math.floor(Math.random() * 8), color: [RED, PAPER, VIOLET, 0x0b0614][i % 4], a: Math.random() * 6, spin: (Math.random() - 0.5) * 0.8 });
    }
    if (hard > 0.22) {
      // Cracks: jagged lines out from the point, branching once.
      const size = 50 + hard * 150;
      const pts: Pt[][] = [];
      for (let i = 0; i < 6 + Math.round(hard * 4); i++) {
        const a = (i / 7) * Math.PI * 2 + Math.random() * 0.6;
        const line: Pt[] = [[at[0], at[1]]];
        let [x, y, ang] = [at[0], at[1], a];
        const len = size * (0.5 + Math.random() * 0.6);
        for (let s = 0; s < 5; s++) {
          ang += (Math.random() - 0.5) * 0.7;
          x += (Math.cos(ang) * len) / 5;
          y += (Math.sin(ang) * len) / 5;
          line.push([x, y]);
          if (s === 2 && Math.random() < 0.6) {
            const b = ang + (Math.random() < 0.5 ? 0.7 : -0.7);
            pts.push([[x, y], [x + (Math.cos(b) * len) / 3, y + (Math.sin(b) * len) / 3]]);
          }
        }
        pts.push(line);
      }
      this.cracks.push({ pts, age: 0 });
    }
    if (hard > 0.6) this.lines.push({ x: at[0], y: at[1], r: 120 + hard * 80, age: 0 });
    if (hard > 0.3) this.glitchLeft = Math.max(this.glitchLeft, Math.round(4 + hard * 6));
  }

  /** Stars going round her head (knocked down): `center` is where her head is now. */
  dizzy(center: () => Pt, frames = 58) {
    this.stars.push({ center, age: 0, life: frames });
  }

  /** Her click: a wire from `from` (her finger, wherever it goes) to `rect`, and the reticle on it. */
  wire(from: () => Pt, rect: [number, number, number, number]) {
    this.wires.push({ from, rect, age: 0 });
  }

  /** Fires an effect: `head` is where her head is, `feet` under her, `scale` her size (CSS px per rig px). */
  cue(name: string, head: Pt, feet: Pt, scale: number) {
    switch (name) {
      case "sparkle":
        for (let i = 0; i < 7; i++) {
          const a = Math.random() * Math.PI * 2;
          const r = (30 + Math.random() * 50) * scale;
          this.sparks.push({ x: head[0] + Math.cos(a) * r, y: head[1] + Math.sin(a) * r * 0.8, vx: Math.cos(a) * 0.6, vy: -0.8 - Math.random(), size: (5 + Math.random() * 7) * scale, life: 0, max: 10 + Math.floor(Math.random() * 8), color: SPARK[i % SPARK.length] });
        }
        break;
      case "dust":
        for (let i = 0; i < 8; i++) {
          const side = i % 2 ? 1 : -1;
          this.dust.push({ x: feet[0] + side * (6 + Math.random() * 14) * scale, y: feet[1], vx: side * (1.5 + Math.random() * 2.5) * scale, vy: -(0.3 + Math.random() * 0.8) * scale, size: (4 + Math.random() * 6) * scale, life: 0, max: 9 + Math.floor(Math.random() * 5), color: 0xd8c8ff });
        }
        break;
      case "glitch":
        this.glitchLeft = 8;
        break;
      case "flash":
        this.flashLeft = 5;
        this.flashAt = head;
        break;
    }
  }

  /** One frame (24 fps). */
  step() {
    for (const list of [this.sparks, this.dust]) {
      for (const p of list) {
        p.life++;
        p.x += p.vx;
        p.y += p.vy;
        p.vy *= 0.92;
        p.vx *= 0.9;
      }
    }
    this.sparks = this.sparks.filter((p) => p.life < p.max);
    this.dust = this.dust.filter((p) => p.life < p.max);
    for (const r of this.rings) r.age++;
    this.rings = this.rings.filter((r) => r.age < 7);
    for (const p of this.shards) {
      p.life++;
      p.x += p.vx;
      p.y += p.vy;
      p.vy += 0.6;
      p.vx *= 0.92;
      p.a += p.spin;
    }
    this.shards = this.shards.filter((p) => p.life < p.max);
    for (const c of this.cracks) c.age++;
    this.cracks = this.cracks.filter((c) => c.age < 36);
    for (const l of this.lines) l.age++;
    this.lines = this.lines.filter((l) => l.age < 8);
    for (const bo of this.bolts) bo.age++;
    this.bolts = this.bolts.filter((bo) => bo.age < 9);
    if (this.sky && !this.sky.hold && ++this.sky.age > 216) this.sky = null;
    if (this.passing && ++this.passing.age > this.passing.frames) this.passing = null;
    for (const s of this.scans) s.age++;
    this.scans = this.scans.filter((s) => s.age < 20);
    for (const p of this.notes) {
      p.life++;
      p.x += p.vx + Math.sin(p.life / 3) * 0.6;
      p.y += p.vy;
    }
    this.notes = this.notes.filter((p) => p.life < p.max);
    for (const s of this.streaks) s.age++;
    this.streaks = this.streaks.filter((s) => s.age < 3);
    for (const p of this.flakes) {
      p.life++;
      p.x += p.vx + Math.sin(p.life / 6) * 0.4;
      p.y += p.vy;
    }
    this.flakes = this.flakes.filter((p) => p.life < p.max);
    for (const st of this.stars) st.age++;
    this.stars = this.stars.filter((st) => st.age < st.life);
    for (const w of this.wires) w.age++;
    this.wires = this.wires.filter((w) => w.age <= WIRE_HOLD + 4);
    if (this.glitchLeft > 0) this.glitchLeft--;
    if (this.twitchLeft > 0) this.twitchLeft--;
    if (this.flashLeft > 0) this.flashLeft--;
    // The glitch jumps on ones: a new sideways offset every frame, zero between bursts. A twitch is a small one.
    this.jitter = this.glitchLeft > 0 && this.glitchLeft % 3 !== 0 ? (Math.random() - 0.5) * 18 : this.twitchLeft > 0 ? (Math.random() < 0.5 ? -1 : 1) * (2 + Math.random() * 3) : 0;
  }

  /** A twitch: a small glitch through her for two drawings (every few seconds while she's awake). */
  twitch() {
    this.twitchLeft = 4;
  }

  /** How far her body jumps sideways this frame (CSS px). */
  get offset() {
    return this.jitter;
  }

  /** How far her violet copy is torn from her this frame (CSS px). */
  get shadowOffset() {
    return this.glitchLeft > 0 ? 6 + Math.random() * 10 : this.twitchLeft > 0 ? 5 : 2.6;
  }

  get busy() {
    return this.sparks.length > 0 || this.dust.length > 0 || this.glitchLeft > 0 || this.twitchLeft > 0 || this.flashLeft > 0 || this.wires.length > 0 || this.rings.length > 0 || this.shards.length > 0 || this.cracks.length > 0 || this.stars.length > 0 || this.bolts.length > 0 || this.flakes.length > 0 || this.streaks.length > 0 || this.notes.length > 0 || this.scans.length > 0 || this.sky !== null || this.passing !== null;
  }

  /** Draws the particles and the flash (in front of her). */
  draw(g: Graphics) {
    g.clear();
    for (const p of this.sparks) {
      // A four-point star that grows, then shrinks away.
      const t = p.life / p.max;
      const s = p.size * Math.sin(Math.PI * Math.min(1, t * 1.2));
      if (s <= 0.2) continue;
      g.poly([p.x, p.y - s, p.x + s * 0.28, p.y - s * 0.28, p.x + s, p.y, p.x + s * 0.28, p.y + s * 0.28, p.x, p.y + s, p.x - s * 0.28, p.y + s * 0.28, p.x - s, p.y, p.x - s * 0.28, p.y - s * 0.28]);
      g.fill({ color: p.color, alpha: 1 - t * 0.5 });
    }
    for (const p of this.dust) {
      const t = p.life / p.max;
      g.circle(p.x, p.y, p.size * (0.6 + t));
      g.fill({ color: p.color, alpha: 0.5 * (1 - t) });
    }
    for (const c of this.cracks) {
      // White for a drawing, then the glass's own crack fading out.
      const alpha = c.age < 2 ? 1 : Math.max(0, 1 - (c.age - 2) / 34);
      for (const line of c.pts) {
        g.poly(line.flat(), false).stroke({ color: VIOLET, width: 4, alpha: alpha * 0.8 });
        g.poly(line.flat(), false).stroke({ color: c.age < 2 ? PAPER : 0xdcd2ff, width: 1.6, alpha });
      }
    }
    for (const r of this.rings) {
      const t = r.age / 7;
      g.circle(r.x, r.y, (30 + t * 150) * r.size).stroke({ color: t < 0.3 ? PAPER : RED, width: 6 * (1 - t) + 1, alpha: 1 - t });
    }
    for (const l of this.lines) {
      // Speed lines: spokes round the hit, jumping every drawing.
      for (let i = 0; i < 18; i++) {
        const a = (i / 18) * Math.PI * 2 + Math.random() * 0.2;
        const [r0, r1] = [l.r * (1 + Math.random() * 0.3), l.r * 2.4];
        g.poly([l.x + Math.cos(a) * r0, l.y + Math.sin(a) * r0, l.x + Math.cos(a + 0.03) * r1, l.y + Math.sin(a + 0.03) * r1, l.x + Math.cos(a - 0.03) * r1, l.y + Math.sin(a - 0.03) * r1]).fill({ color: PAPER, alpha: 0.75 });
      }
    }
    for (const p of this.shards) {
      const t = p.life / p.max;
      const [c, sn] = [Math.cos(p.a), Math.sin(p.a)];
      const sz = p.size * (1 - t * 0.5);
      g.poly([p.x + c * sz, p.y + sn * sz, p.x - sn * sz * 0.35, p.y + c * sz * 0.35, p.x - c * sz * 0.6, p.y - sn * sz * 0.6]).fill({ color: p.color, alpha: 1 - t * 0.4 });
    }
    for (const st of this.stars) {
      const [cx, cy] = st.center();
      for (let i = 0; i < 4; i++) {
        // Round her head on an ellipse, on twos.
        const a = Math.floor(st.age / 2) * 0.5 + (i / 4) * Math.PI * 2;
        const [x, y, s2] = [cx + Math.cos(a) * 46, cy - 30 + Math.sin(a) * 14, 7];
        g.poly([x, y - s2, x + s2 * 0.3, y - s2 * 0.3, x + s2, y, x + s2 * 0.3, y + s2 * 0.3, x, y + s2, x - s2 * 0.3, y + s2 * 0.3, x - s2, y, x - s2 * 0.3, y - s2 * 0.3]).fill({ color: i % 2 ? 0xffe14d : PAPER });
      }
    }
    if (this.sky) this.drawSky(g, this.sky);
    if (this.passing) this.drawPassing(g, this.passing);
    for (const s of this.scans) {
      const [l, t, r, b] = s.rect;
      const y = t + ((b - t) * Math.min(s.age, 16)) / 16;
      const fade = s.age < 16 ? 1 : 1 - (s.age - 16) / 4;
      // The band it has read, faint, with a few lines of "text" picked out; the line itself, bright, with a glow.
      g.rect(l, Math.max(t, y - 90), r - l, Math.min(90, y - t)).fill({ color: GREEN, alpha: 0.06 * fade });
      for (let k = 0; k < 5; k++) {
        const ly = y - 12 - k * 17;
        if (ly > t) g.rect(l + 20 + ((k * 97) % 60), ly, (r - l) * (0.3 + ((k * 37) % 50) / 100), 2).fill({ color: GREEN, alpha: 0.25 * fade * (1 - k / 5) });
      }
      g.rect(l, y - 4, r - l, 8).fill({ color: GREEN, alpha: 0.18 * fade });
      g.rect(l, y - 1, r - l, 2.5).fill({ color: PAPER, alpha: 0.9 * fade });
      // Brackets on its corners.
      for (const [cx, cy, dx, dy] of [[l, t, 1, 1], [r, t, -1, 1], [l, b, 1, -1], [r, b, -1, -1]] as const) {
        g.poly([cx + dx * 22, cy, cx, cy, cx, cy + dy * 22], false).stroke({ color: GREEN, width: 3, alpha: fade });
      }
    }
    for (const p of this.notes) {
      // ♪: a slanted head, a stem, a flag.
      const s = p.size;
      const a = 1 - p.life / p.max;
      g.ellipse(p.x, p.y, s * 0.55, s * 0.4).fill({ color: p.color, alpha: a });
      g.rect(p.x + s * 0.4, p.y - s * 1.7, s * 0.16, s * 1.7).fill({ color: p.color, alpha: a });
      g.poly([p.x + s * 0.56, p.y - s * 1.7, p.x + s * 1.2, p.y - s * 1.1, p.x + s * 0.56, p.y - s * 1.25]).fill({ color: p.color, alpha: a });
    }
    for (const s of this.streaks) {
      const [x, y] = s.at();
      const [dx, dy] = s.dir;
      const [nx, ny] = [-dy, dx];
      // Seven lines behind her, staggered, thinning to a point; the last drawing fades.
      for (let i = 0; i < 7; i++) {
        const off = (i - 3) * 22 + Math.sin(i * 12.9) * 8;
        const len = s.len * (0.55 + 0.45 * Math.abs(Math.sin(i * 7.3)));
        const [sx, sy] = [x + nx * off - dx * 30, y + ny * off - dy * 30];
        const w = 5 - Math.abs(i - 3);
        g.poly([sx + nx * w, sy + ny * w, sx - nx * w, sy - ny * w, sx - dx * len, sy - dy * len]).fill({ color: i % 3 ? PAPER : RED, alpha: s.age < 2 ? 0.9 : 0.4 });
      }
    }
    for (const p of this.flakes) {
      const t = p.life / p.max;
      g.poly([p.x, p.y - p.size, p.x + p.size * 0.4, p.y + p.size * 0.6, p.x - p.size * 0.4, p.y + p.size * 0.3]).fill({ color: p.color, alpha: 1 - t });
    }
    for (const bo of this.bolts) {
      const a = bo.a();
      // A new jag every other drawing; a fork or two off it; fading over the last three.
      let seed = Math.floor(bo.age / 2) * 7919 + 13;
      const rand = () => ((seed = (seed * 16807) % 2147483647) / 2147483647) - 0.5;
      const jag = (from: Pt, to: Pt, n: number, amp: number): Pt[] => {
        const [dx, dy] = [to[0] - from[0], to[1] - from[1]];
        const len = Math.hypot(dx, dy) || 1;
        const [nx, ny] = [-dy / len, dx / len];
        return Array.from({ length: n + 1 }, (_, i) => {
          const u = i / n;
          const off = i === 0 || i === n ? 0 : rand() * amp;
          return [from[0] + dx * u + nx * off, from[1] + dy * u + ny * off] as Pt;
        });
      };
      const main = jag(a, bo.b, 9, 26);
      const forks = [3, 6].map((i) => jag(main[i], [main[i][0] + rand() * 90, main[i][1] + rand() * 90], 4, 14));
      const alpha = bo.age < 6 ? 1 : 1 - (bo.age - 5) / 4;
      for (const line of [main, ...forks]) {
        const flat = line.flat();
        g.poly(flat.map((v, i) => v + (i % 2 ? 2 : -2)), false).stroke({ color: VIOLET, width: 5, alpha });
        g.poly(flat, false).stroke({ color: RED, width: 3.5, alpha });
        g.poly(flat, false).stroke({ color: PAPER, width: 1.2, alpha });
      }
    }
    for (const w of this.wires) drawWire(g, w);
    if (this.flashLeft > 0) {
      const t = 1 - this.flashLeft / 5;
      const [x, y] = this.flashAt;
      g.circle(x, y, 40 + t * 160);
      g.stroke({ color: 0xffffff, width: 6 * (1 - t) + 1, alpha: 1 - t });
      g.circle(x, y, 20 + t * 110);
      g.stroke({ color: GREEN, width: 3 * (1 - t) + 1, alpha: 0.8 * (1 - t) });
    }
  }
}

/** A smear frame's afterimages: her outline again, offset either side, in green and crimson. */
export function ghosts(g: Graphics, shapes: { rings: Pt[][] }[], spread: number, dir: Pt = [1, 0]) {
  g.clear();
  if (spread <= 0) return;
  // Her green copy behind her (along `dir`), her red one ahead.
  for (const [k, color] of [[-spread, GREEN], [spread, CRIMSON]] as const) {
    const [dx, dy] = [dir[0] * k, dir[1] * k];
    for (const s of shapes) {
      g.poly(s.rings[0].flatMap(([x, y]) => [x + dx, y + dy]));
      g.fill({ color, alpha: 0.42 });
    }
  }
}

/** A dashed line from `a` to `b`, its dashes crawling toward `b` as `crawl` grows. */
function dashed(g: Graphics, a: Pt, b: Pt, crawl: number, color: number, width: number, dx = 0, dy = 0) {
  const len = Math.hypot(b[0] - a[0], b[1] - a[1]);
  if (len < 1) return;
  const [ux, uy] = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
  for (let s = (crawl % 15) - 15; s < len; s += 15) {
    const [s0, s1] = [Math.max(0, s), Math.min(len, s + 9)];
    if (s1 <= s0) continue;
    g.moveTo(a[0] + ux * s0 + dx, a[1] + uy * s0 + dy).lineTo(a[0] + ux * s1 + dx, a[1] + uy * s1 + dy);
  }
  g.stroke({ color, width, cap: "butt" });
}

/** Her click, one drawing of it: the wire growing out in two drawings with data running down it and an arrowhead,
 *  the reticle slamming on (white as it lands), pulsing on the tap, then the wire reeling back and the reticle
 *  opening out. */
function drawWire(g: Graphics, w: Wire) {
  const f = w.age;
  const back = f > WIRE_HOLD ? (f - WIRE_HOLD) / 4 : 0;
  if (back >= 1) return;
  const a = w.from();
  const [l, t, r, b] = w.rect;
  const aim: Pt = [(l + r) / 2, (t + b) / 2];
  const grow = Math.min(1, (f + 1) / 2) * (1 - back);
  const end: Pt = [a[0] + (aim[0] - a[0]) * grow, a[1] + (aim[1] - a[1]) * grow];
  const crawl = Math.floor(f / 2) * 5;
  dashed(g, a, end, crawl, VIOLET, 4, -2, 2);
  dashed(g, a, end, crawl, PAPER, 2);
  if (!back) {
    for (let i = 0; i < 3; i++) {
      const u = (Math.floor(f / 2) * 0.06 + i / 3) % 1;
      if (u > grow) continue;
      const [px, py] = [a[0] + (aim[0] - a[0]) * u, a[1] + (aim[1] - a[1]) * u];
      g.rect(Math.round(px) - 3, Math.round(py) - 1, 5, 5).fill({ color: VIOLET });
      g.rect(Math.round(px) - 2, Math.round(py) - 2, 5, 5).fill({ color: i ? GREEN : PAPER });
    }
  }
  if (grow >= 1) {
    const ang = Math.atan2(aim[1] - a[1], aim[0] - a[0]);
    for (const [dx, dy, color] of [[-2, 2, VIOLET], [0, 0, RED]] as const) {
      g.poly([aim[0] + dx, aim[1] + dy, aim[0] - Math.cos(ang - 0.45) * 16 + dx, aim[1] - Math.sin(ang - 0.45) * 16 + dy, aim[0] - Math.cos(ang + 0.45) * 16 + dx, aim[1] - Math.sin(ang + 0.45) * 16 + dy]).fill({ color });
    }
  }
  // The tap is two drawings after it lands; the reticle blinks off every eighth drawing once it's on.
  const tap = f === 6 || f === 7;
  let pad = f < SLAM.length ? SLAM[f] : 7;
  if (tap) pad += 6;
  pad += back * 60;
  if (f % 8 === 7 && f >= 8) return;
  const [x0, y0, x1, y1] = [l - pad, t - pad, r + pad, b + pad];
  const arm = Math.min(28, (x1 - x0) / 3, (y1 - y0) / 3);
  const flash = f < 5 || tap;
  for (const [dx, dy, color, width] of [[-2, 2, VIOLET, 5], [0, 0, flash ? PAPER : RED, 4]] as const) {
    for (const [cx, cy, sx, sy] of [[x0, y0, 1, 1], [x1, y0, -1, 1], [x0, y1, 1, -1], [x1, y1, -1, -1]]) {
      g.moveTo(cx + dx, cy + sy * arm + dy).lineTo(cx + dx, cy + dy).lineTo(cx + sx * arm + dx, cy + dy);
    }
    g.stroke({ color, width, join: "miter" });
  }
}
