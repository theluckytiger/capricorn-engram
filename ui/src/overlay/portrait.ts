// Her face up close: a portrait in her panel with eight expressions, picked per line and glitching from one to the
// next, and cut-ins (anime close-ups by her head) on big moments. Drawn from her own rig (mane and mask at rest) with
// the features drawn bigger than on her body, where they're a few pixels wide.

import type { Pt, Rig } from "./rig";

/** Watching, turned away, fang grin, round eyes, looking at you, laughing open-mouthed, all her eyes, zigzag grin. */
export type Look = "watching" | "away" | "fang" | "round" | "look" | "laugh" | "eyes" | "zigzag";

const INK = "#0b0614";
const VIOLET = "#8a1fd1";
const WHITE = "#fbf8fc";
const IRIS = "#3e5bd8";
const IRIS_LIGHT = "#7fb0ff";
const GREEN = "#08f562";
const CRIMSON = "#f60655";
// Deep red: the dark of her body's mouth disappears on her black mask up close.
const MOUTH = "#a3053c";

// Her two eyes and mouth on the mask (rig px), as on her body; drawn half again as big.
const EYES = [
  { c: [128.6, 131.4] as Pt, w: 5.2 * 1.5, h: 3.6 * 1.5 },
  { c: [116.9, 132.2] as Pt, w: 3.2 * 1.5, h: 2.4 * 1.5 },
];
const MOUTH_AT: Pt = [123.5, 141.2];
// Where her other eyes open, on her mane and mask, for "all her eyes".
const MORE_EYES: Pt[] = [[104, 112], [149, 107], [99, 139], [157, 142], [139, 99], [113, 99], [146, 124], [108, 125]];

/** Which face fits a line of hers. */
export function lookFor(line: string): Look {
  const l = line.toLowerCase();
  if (/\b(ha(ha)+|hah|lol|lmao|heh heh)\b|😂/.test(l)) return "laugh";
  if (l === "…" || l === "...") return "eyes";
  if (/listening/.test(l)) return "look";
  if (/\b(zap|chaos|trouble|break(ing)?|burn|fry|mess with)\b/.test(l)) return "zigzag";
  if (/\b(sorry|can't|couldn't|failed|broke|nothing|didn't work|no luck|gone)\b/.test(l)) return "away";
  if (/\b(ow|whoa|wait what|hey!|what the|stop it)\b/.test(l)) return "round";
  if (/\b(obviously|you're welcome|nailed|easy|mine|choom|done\.|of course|told you)\b/.test(l) || /!$/.test(l)) return "fang";
  if (/\?$/.test(l)) return "look";
  if (/\b(found|looking|watching|checking|searching|reading)\b/.test(l)) return "watching";
  return Math.random() < 0.15 ? "fang" : "look";
}

/** Draws her face in `look` into `ctx` (w×h device px): `half` rig px either side of `center`, `t` a frame count. */
function paint(ctx: CanvasRenderingContext2D, w: number, h: number, rig: Rig, look: Look, o: { center: Pt; half: number; t: number; talk: number; blink: boolean; open: number }) {
  const s = w / (o.half * 2);
  const away = look === "away";
  // Laughing, she shakes.
  const shake = look === "laugh" ? Math.sin(o.t * 2.1) * 0.8 : 0;
  const [x0, y0] = [o.center[0] - o.half + shake, o.center[1] - (o.half * h) / w];
  ctx.setTransform(away ? -s : s, 0, 0, s, away ? w + x0 * s : -x0 * s, -y0 * s);
  for (const part of rig.file.parts) {
    if (part.name !== "mane" && part.name !== "head") continue;
    for (const [dx, color] of [[1.4, VIOLET], [0, ""]] as const) {
      for (const shape of part.shapes) {
        const p = new Path2D();
        for (const ring of shape.rings) {
          ring.forEach(([x, y], i) => (i ? p.lineTo(x + dx, y + dx) : p.moveTo(x + dx, y + dx)));
          p.closePath();
        }
        ctx.fillStyle = color || rig.file.palette[shape.fill];
        ctx.fill(p, "evenodd");
      }
    }
  }
  const almond = ([cx, cy]: Pt, ew: number, eh: number) => {
    ctx.beginPath();
    ctx.moveTo(cx - ew / 2, cy);
    ctx.quadraticCurveTo(cx, cy - eh, cx + ew / 2, cy);
    ctx.quadraticCurveTo(cx, cy + eh, cx - ew / 2, cy);
    ctx.closePath();
  };
  const eye = (c: Pt, ew: number, eh: number, lid: number, pupil: number, gaze: Pt, iris = IRIS, light = IRIS_LIGHT) => {
    const hh = eh * lid;
    almond(c, ew + 1.4, hh + 1.2);
    ctx.fillStyle = INK;
    ctx.fill();
    if (lid < 0.2) {
      ctx.strokeStyle = WHITE;
      ctx.lineWidth = 0.5;
      ctx.beginPath();
      ctx.moveTo(c[0] - ew / 2, c[1]);
      ctx.lineTo(c[0] + ew / 2, c[1]);
      ctx.stroke();
      return;
    }
    almond(c, ew, hh);
    ctx.fillStyle = iris;
    ctx.fill();
    const p: Pt = [c[0] + gaze[0] * ew * 0.18, c[1] + gaze[1] * hh * 0.18];
    almond(p, ew * 0.55 * pupil, hh * 0.6 * pupil);
    ctx.fillStyle = light;
    ctx.fill();
    almond(p, ew * 0.3 * pupil, hh * 0.45 * pupil);
    ctx.fillStyle = INK;
    ctx.fill();
    almond([p[0] - ew * 0.14, p[1] - hh * 0.2], ew * 0.16, hh * 0.22);
    ctx.fillStyle = WHITE;
    ctx.fill();
  };
  const arc = ([cx, cy]: Pt, ew: number) => {
    // A closed, laughing eye: ^
    ctx.strokeStyle = WHITE;
    ctx.lineWidth = 0.9;
    ctx.beginPath();
    ctx.moveTo(cx - ew / 2, cy + 0.8);
    ctx.lineTo(cx, cy - 1.2);
    ctx.lineTo(cx + ew / 2, cy + 0.8);
    ctx.stroke();
  };
  const lid = o.blink ? 0.1 : { watching: 0.5, away: 0.6, fang: 0.6, round: 1.45, look: 1, laugh: 1, eyes: 1, zigzag: 1.2 }[look];
  const pupil = look === "round" || look === "zigzag" ? 0.6 : 1;
  const gaze: Pt = look === "watching" ? [-1, 0.2] : look === "away" ? [1, 0.6] : [0, 0];
  for (const e of EYES) {
    if (look === "laugh") arc(e.c, e.w);
    else eye(e.c, e.w, e.h, lid, pupil, gaze);
  }
  if (look === "eyes") {
    // Her other eyes opening one after another, green and red, all looking at you.
    MORE_EYES.forEach((c, i) => {
      const open = Math.max(0, Math.min(1, o.open * 8 - i));
      if (open > 0) eye(c, 5, 3.4, 0.15 + open * 0.95, 0.9, [0, 0], i % 2 ? GREEN : CRIMSON, WHITE);
    });
  }
  // The mouth.
  const [mx, my] = MOUTH_AT;
  const talk = Math.min(1, o.talk);
  ctx.fillStyle = MOUTH;
  ctx.beginPath();
  switch (look) {
    case "fang": {
      // A crescent grin, up at the near corner, her fang hanging down.
      ctx.moveTo(mx - 5, my - 1.4);
      ctx.quadraticCurveTo(mx, my + 2.8 + talk * 2, mx + 5.6, my - 2.6);
      ctx.quadraticCurveTo(mx, my + 0.6, mx - 5, my - 1.4);
      ctx.fill();
      ctx.fillStyle = WHITE;
      ctx.beginPath();
      ctx.moveTo(mx + 1.2, my - 0.4);
      ctx.lineTo(mx + 2.8, my - 0.8);
      ctx.lineTo(mx + 2, my + 1.9);
      ctx.fill();
      break;
    }
    case "laugh": {
      const open = 3.5 + Math.abs(Math.sin(o.t * 1.3)) * 2;
      ctx.moveTo(mx - 4.6, my - 1);
      ctx.lineTo(mx + 4.6, my - 1.8);
      ctx.quadraticCurveTo(mx + 3, my + open, mx - 0.5, my + open);
      ctx.quadraticCurveTo(mx - 4, my + open * 0.8, mx - 4.6, my - 1);
      ctx.fill();
      ctx.fillStyle = WHITE;
      ctx.beginPath();
      ctx.moveTo(mx + 1, my - 1.3);
      ctx.lineTo(mx + 2.8, my - 1.6);
      ctx.lineTo(mx + 1.9, my + 1.6);
      ctx.fill();
      break;
    }
    case "zigzag": {
      // A wide grin of teeth, a zigzag between them.
      ctx.moveTo(mx - 6, my - 2);
      ctx.quadraticCurveTo(mx, my + 3.4 + talk * 2, mx + 6.4, my - 3);
      ctx.quadraticCurveTo(mx, my + 0.4, mx - 6, my - 2);
      ctx.fill();
      ctx.strokeStyle = WHITE;
      ctx.lineWidth = 0.7;
      ctx.beginPath();
      for (let i = 0; i <= 8; i++) {
        const u = i / 8;
        const x = mx - 5.6 + u * 11.6;
        const base = my - 1.6 + Math.sin(u * Math.PI) * 2.4 - u * 0.8;
        ctx.lineTo(x, base + (i % 2 ? 0.9 : -0.9));
      }
      ctx.stroke();
      break;
    }
    case "round": {
      ctx.ellipse(mx, my + 0.6, 1.6, 1.4 + talk * 1.5, 0, 0, Math.PI * 2);
      ctx.fill();
      break;
    }
    default: {
      const open = 0.6 + talk * 3.4;
      const down = look === "away" ? 0.8 : 0;
      ctx.moveTo(mx - 3.4, my - 0.4 + down);
      ctx.lineTo(mx + 3.2, my - 1.1);
      ctx.lineTo(mx + 2.6, my - 1.1 + open);
      ctx.lineTo(mx - 2.8, my - 0.4 + open * 0.8 + down);
      ctx.fill();
      ctx.fillStyle = WHITE;
      ctx.beginPath();
      ctx.moveTo(mx + 0.4, my - 0.9);
      ctx.lineTo(mx + 1.7, my - 1);
      ctx.lineTo(mx + 1, my + 0.6 + open * 0.25);
      ctx.fill();
    }
  }
  ctx.setTransform(1, 0, 0, 1, 0, 0);
}

/** Slices of the picture pushed sideways, and colour fringes: a glitch, `k` 0..1. */
function glitch(ctx: CanvasRenderingContext2D, w: number, h: number, k: number) {
  const band = Math.max(3, Math.round(h / 9));
  for (let y = 0; y < h; y += band) {
    if (Math.random() > 0.55) continue;
    const dx = (Math.random() - 0.5) * w * 0.25 * k;
    ctx.drawImage(ctx.canvas, 0, y, w, band, dx, y, w, band);
  }
  ctx.globalCompositeOperation = "screen";
  ctx.fillStyle = Math.random() < 0.5 ? "rgba(8,245,98,0.25)" : "rgba(246,6,85,0.25)";
  ctx.fillRect(0, Math.random() * h, w, h * 0.2);
  ctx.globalCompositeOperation = "source-over";
}

/** The backdrop: violet into ink, scanlines. */
function backdrop(ctx: CanvasRenderingContext2D, w: number, h: number, lines = false, t = 0) {
  const g = ctx.createLinearGradient(0, 0, w, h);
  g.addColorStop(0, "#3a0d5c");
  g.addColorStop(1, INK);
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, w, h);
  if (lines) {
    // Speed lines from off to one side, redrawn on twos.
    let seed = Math.floor(t / 2) * 7919 + 3;
    const rand = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
    for (let i = 0; i < 26; i++) {
      const y = rand() * h;
      const len = w * (0.3 + rand() * 0.6);
      ctx.fillStyle = i % 4 ? "rgba(251,248,252,0.18)" : "rgba(246,6,85,0.45)";
      ctx.beginPath();
      ctx.moveTo(w, y - 2 - rand() * 3);
      ctx.lineTo(w, y + 2 + rand() * 3);
      ctx.lineTo(w - len, y);
      ctx.fill();
    }
  }
  ctx.fillStyle = "rgba(0,0,0,0.18)";
  for (let y = 0; y < h; y += 3) ctx.fillRect(0, y, w, 1);
}

/** Her portrait in her panel. Redraws at 12 drawings a second while it's on screen. */
export class Portrait {
  private ctx: CanvasRenderingContext2D;
  private look: Look = "look";
  private glitching = 0;
  private t = 0;
  private since = 0;
  private nextBlink = performance.now() + 2500;
  private blinkUntil = 0;
  /** How loud she is right now (her mouth). */
  talk = 0;

  constructor(
    readonly canvas: HTMLCanvasElement,
    private rig: Rig,
  ) {
    this.ctx = canvas.getContext("2d")!;
    setInterval(() => canvas.getClientRects().length > 0 && this.draw(), 83);
  }

  set(look: Look) {
    if (look === this.look) return;
    this.look = look;
    this.glitching = 4;
    this.since = this.t;
  }

  private draw() {
    this.t++;
    const now = performance.now();
    if (now > this.nextBlink) {
      this.blinkUntil = now + 120;
      this.nextBlink = now + 2400 + Math.random() * 4000;
    }
    const { width: w, height: h } = this.canvas;
    backdrop(this.ctx, w, h);
    paint(this.ctx, w, h, this.rig, this.look, { center: [124, 132], half: 20, t: this.t, talk: this.talk, blink: now < this.blinkUntil, open: (this.t - this.since) / 12 });
    if (this.glitching > 0) glitch(this.ctx, w, h, this.glitching-- / 4);
    // Her frame: crimson, a notch cut at the bottom right like her panel.
    this.ctx.strokeStyle = CRIMSON;
    this.ctx.lineWidth = 2;
    this.ctx.strokeRect(1, 1, w - 2, h - 2);
  }
}

/** A cut-in: her face in close-up in a slanted band by her head, speed lines behind, for a moment. */
export function cutIn(rig: Rig, look: Look, at: Pt, ms = 950) {
  const [cw, ch] = [300, 150];
  const canvas = document.createElement("canvas");
  canvas.className = "cutin";
  canvas.width = cw * devicePixelRatio;
  canvas.height = ch * devicePixelRatio;
  // Above her head, clear of her panel (which is to her left).
  const left = Math.max(8, Math.min(innerWidth - cw - 8, at[0] - cw / 2 - 40));
  const top = Math.max(8, Math.min(innerHeight - ch - 8, at[1] - ch - 90));
  Object.assign(canvas.style, { width: `${cw}px`, height: `${ch}px`, left: `${left}px`, top: `${top}px` });
  document.body.appendChild(canvas);
  const ctx = canvas.getContext("2d")!;
  const { width: w, height: h } = canvas;
  let t = 0;
  let out = 0;
  // In over three drawings (sliding, opening up), out over two.
  const IN = ["translateX(60px) scaleY(0.2)", "translateX(24px) scaleY(0.7)"];
  const OUT = ["translateX(-20px) scaleY(0.5)", "translateX(-40px) scaleY(0.1)"];
  const frame = () => {
    t++;
    const step = out ? OUT[out - 1] : IN[t - 1];
    canvas.style.transform = step ?? "none";
    canvas.style.opacity = out ? String(1 - out * 0.45) : t <= IN.length ? String(t * 0.4) : "1";
    backdrop(ctx, w, h, true, t);
    paint(ctx, w, h, rig, look, { center: [124, 136], half: 15, t, talk: look === "laugh" ? 1 : 0.2, blink: false, open: t / 10 });
    if (t <= 2) glitch(ctx, w, h, 1);
  };
  frame();
  const timer = setInterval(frame, 83);
  setTimeout(() => {
    out = 1;
    setTimeout(() => (out = 2), 83);
    setTimeout(() => {
      clearInterval(timer);
      canvas.remove();
    }, 170);
  }, ms);
}
