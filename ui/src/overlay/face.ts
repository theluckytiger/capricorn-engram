// Her face on the mask: blue almond eyes that look, blink and narrow, and a mouth with her white fang that opens
// with her voice. Drawn in the head bone's space (rest coordinates of the traced cut-out), on top of her body, so it
// turns with her head.

import type { Graphics } from "pixi.js";
import type { Pt, Rig } from "./rig";

const IRIS = 0x3e5bd8;
const IRIS_LIGHT = 0x7fb0ff;
const INK = 0x0b0614;
const WHITE = 0xfbf8fc;
const MOUTH = 0x2a0a35;

// The two sockets in the cut-out: the near eye is bigger (she's turned three quarters).
const EYES = [
  { c: [128.6, 131.4] as Pt, w: 5.2, h: 3.6 },
  { c: [116.9, 132.2] as Pt, w: 3.2, h: 2.4 },
];
const MOUTH_AT: Pt = [123.5, 141.2];

/** "peek": asleep, one eye cracked open. */
export type Expression = "calm" | "smug" | "wide" | "closed" | "peek";

export class Face {
  private nextBlink = performance.now() + 2500;
  private blinkUntil = 0;
  /** Mouth openness, smoothed so it doesn't flicker on every voice level. */
  private open = 0;
  expression: Expression = "calm";

  /** `talk` 0..1 (her loudness), `look` -1..1 in x and y (where she looks). */
  draw(g: Graphics, rig: Rig, now: number, talk: number, look: Pt) {
    if (now > this.nextBlink) {
      this.blinkUntil = now + 110;
      this.nextBlink = now + 2200 + Math.random() * 4200;
    }
    const blinking = now < this.blinkUntil;
    const lids = EYES.map((_, i) =>
      this.expression === "peek" ? (i === 0 ? 0.5 : 0.12) : this.expression === "closed" || blinking ? 0.12 : this.expression === "smug" ? 0.55 : this.expression === "wide" ? 1.25 : 1,
    );
    for (const [i, e] of EYES.entries()) {
      const lid = lids[i];
      const h = e.h * lid;
      const almond = (w: number, hh: number, [cx, cy]: Pt): Pt[] => [
        [cx - w / 2, cy],
        [cx - w / 4, cy - hh / 2],
        [cx + w / 4, cy - hh / 2],
        [cx + w / 2, cy],
        [cx + w / 4, cy + hh / 2],
        [cx - w / 4, cy + hh / 2],
      ];
      // The socket's dark rim, then the iris, then the pupil and its glint, which move a little toward `look`.
      poly(g, rig.onBone("head", almond(e.w + 1.2, h + 1.2, e.c)), INK);
      if (lid < 0.2) continue;
      poly(g, rig.onBone("head", almond(e.w, h, e.c)), IRIS);
      const p: Pt = [e.c[0] + look[0] * e.w * 0.12, e.c[1] + look[1] * h * 0.12];
      poly(g, rig.onBone("head", almond(e.w * 0.55, h * 0.6, p)), IRIS_LIGHT);
      poly(g, rig.onBone("head", almond(e.w * 0.28, h * 0.42, p)), INK);
      poly(g, rig.onBone("head", almond(e.w * 0.16, h * 0.22, [p[0] - e.w * 0.14, p[1] - h * 0.16])), WHITE);
    }
    // The mouth: a slit that opens with her voice, her fang hanging from its top edge.
    this.open += (Math.min(1, talk) - this.open) * 0.5;
    const o = 0.5 + this.open * 3.2;
    const [mx, my] = MOUTH_AT;
    poly(g, rig.onBone("head", [[mx - 3.4, my - 0.4], [mx + 3.2, my - 1.1], [mx + 2.6, my - 1.1 + o], [mx - 2.8, my - 0.4 + o * 0.8]]), MOUTH);
    poly(g, rig.onBone("head", [[mx + 0.4, my - 0.9], [mx + 1.7, my - 1.0], [mx + 1.0, my + 0.6 + o * 0.25]]), WHITE);
  }
}

function poly(g: Graphics, pts: Pt[], color: number) {
  g.poly(pts.flat());
  g.fill({ color });
}
