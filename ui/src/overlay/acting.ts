// How she moves: poses held for a number of frames, like an anime exposure sheet. Nothing is tweened: a drawing holds,
// then the next one replaces it. A move is a few key drawings: an anticipation the other way, the action (often a
// stretched or smeared frame), an overshoot, and a hold. Most of it is on twos (each drawing held 2 frames at 24 fps);
// holds are long.
//
// Angles are degrees on top of her rest pose, positive = clockwise on screen. On screen her right arm (armR) hangs
// from her near shoulder; her left arm (armL) is the far one. `root` turns her whole body around her hips.

import type { Expression } from "./face";
import type { Pose } from "./rig";

/** A key drawing, and how many frames (at 24 fps) it's held. */
export interface Drawing {
  hold: number;
  /** Her whole body moved, rig px (dy < 0 is up). */
  dx?: number;
  dy?: number;
  /** Squash and stretch around her hips (1 = as drawn). */
  sx?: number;
  sy?: number;
  angles?: Record<string, number>;
  /** Mane: how far it spreads, and its sway. */
  flare?: number;
  sway?: number;
  /** Afterimages trailing her, rig px across: a green one and a crimson one (a smear frame). */
  echo?: number;
  face?: Expression;
  /** An effect when this drawing comes up: "sparkle", "glitch", "zap", "flash", "dust". */
  cue?: string;
  /** A sound when this drawing comes up (one of `sounds.ts`'s cues: "zap-out", "zap-in"…). */
  sound?: string;
}

export interface Act {
  keys: Drawing[];
  loop?: boolean;
  /** A stronger act cuts a weaker one off; a weaker one waits its turn (or is dropped). */
  priority: number;
}

/** Her standing rest: the cut-out has one arm flung out; at rest it hangs like the other. */
export const REST: Record<string, number> = {
  armL_up: -36,
  armL_fore: -4,
  armL_hand: -12,
  armR_up: -2,
  armR_fore: 2,
  legL_thigh: 1,
  legR_thigh: -1,
};

/** Her arms sitting on a ledge, like the boot-up Capri's: both straight down and out, a hand planted on it either side. */
const SIT_HANDS: Record<string, number> = { armL_up: 26, armL_fore: 6, armL_hand: 24, armR_up: -18, armR_fore: -54, armR_hand: 56 };

const rep = (n: number, keys: Drawing[]) => Array.from({ length: n }, () => keys).flat();

/** Every move she has. Original choreography; the vocabulary (squash, smear, echo) is cel animation's. */
export const ACTS: Record<string, Act> = {
  // ---- states (looping until something else happens)

  /** Idle: floating, four drawings held long, the mane breathing with them. */
  float: {
    loop: true,
    priority: 0,
    keys: [
      { hold: 14, dy: 0, angles: { head: 0, armL_up: 0, armR_up: 0 }, flare: 0.02, sway: -1.2 },
      { hold: 14, dy: -2.5, angles: { head: 1.2, armL_up: 1.5, armR_up: -1 }, flare: 0.045, sway: 0 },
      { hold: 14, dy: -4.5, angles: { head: 2, armL_up: 3, armR_up: -2 }, flare: 0.065, sway: 1.2 },
      { hold: 14, dy: -2.5, angles: { head: 1.2, armL_up: 1.5, armR_up: -1 }, flare: 0.045, sway: 0 },
    ],
  },
  /** Her name heard: she leans in, head cocked, near hand up by her ear. */
  listen: {
    loop: true,
    priority: 1,
    keys: [
      { hold: 3, dy: -1, angles: { root: -4, head: -10, neck: -4, armR_up: -118, armR_fore: -95, armR_hand: -20 }, flare: 0.08, face: "wide" },
      { hold: 16, dy: -2, angles: { root: -5, head: -12, neck: -5, armR_up: -122, armR_fore: -98, armR_hand: -22 }, flare: 0.1, sway: -1, face: "calm" },
      { hold: 16, dy: -3, angles: { root: -5, head: -9, neck: -4, armR_up: -120, armR_fore: -96, armR_hand: -18 }, flare: 0.08, sway: 1, face: "calm" },
    ],
  },
  /** Working it out: eyes up, a hand at her chin, the mane ticking. */
  think: {
    loop: true,
    priority: 1,
    keys: [
      { hold: 10, dy: -2, angles: { root: 3, head: 8, neck: 3, armR_up: -70, armR_fore: -128, armR_hand: 15 }, flare: 0.05, sway: -2, face: "smug" },
      { hold: 6, dy: -2, angles: { root: 3, head: 10, neck: 3, armR_up: -72, armR_fore: -130, armR_hand: 10 }, flare: 0.09, sway: 0, face: "smug" },
      { hold: 10, dy: -3, angles: { root: 2, head: 7, neck: 2, armR_up: -70, armR_fore: -128, armR_hand: 15 }, flare: 0.05, sway: 2, face: "closed" },
      { hold: 6, dy: -2, angles: { root: 3, head: 9, neck: 3, armR_up: -71, armR_fore: -129, armR_hand: 12 }, flare: 0.07, sway: 0, face: "smug" },
    ],
  },
  /** Held in your hand: arms up, legs dangling, swaying on twos (the swing comes from how you move her). */
  held: {
    loop: true,
    priority: 3,
    keys: [
      { hold: 2, dy: 6, sy: 1.06, sx: 0.96, angles: { armL_up: 70, armL_fore: 10, armR_up: -150, armR_fore: -12, legL_thigh: 6, legL_shin: 4, legR_thigh: -6, legR_shin: -6, head: -6 }, flare: 0.2, face: "wide" },
      { hold: 6, dy: 8, sy: 1.08, sx: 0.95, angles: { armL_up: 76, armL_fore: 14, armR_up: -156, armR_fore: -16, legL_thigh: 10, legL_shin: 8, legR_thigh: -2, legR_shin: -10, head: -8 }, flare: 0.24, sway: 3, face: "wide" },
      { hold: 6, dy: 8, sy: 1.08, sx: 0.95, angles: { armL_up: 72, armL_fore: 10, armR_up: -152, armR_fore: -12, legL_thigh: 2, legL_shin: 2, legR_thigh: -10, legR_shin: -4, head: -4 }, flare: 0.22, sway: -3, face: "wide" },
    ],
  },
  /** Asleep (late at night, or put away): slumped, eyes shut, slow breaths. */
  sleep: {
    loop: true,
    priority: 0,
    keys: [
      { hold: 30, dy: 6, sy: 0.97, angles: { root: 4, spine: 6, neck: 10, head: 16, armL_up: -8, armR_up: 10, armR_fore: 6 }, flare: -0.04, sway: -1, face: "closed" },
      { hold: 30, dy: 4, sy: 0.99, angles: { root: 4, spine: 5, neck: 9, head: 14, armL_up: -6, armR_up: 9, armR_fore: 5 }, flare: -0.02, sway: 1, face: "closed" },
    ],
  },

  /** Sitting on a window's top edge: legs over it kicking one after the other, then a pause with her head cocked (the
   *  earlier Capri's rhythm). Her hands stay put, planted on the ledge either side of her. The first drawing is the
   *  one other moves borrow her legs from while she sits. */
  sit: {
    loop: true,
    priority: 0,
    keys: [
      { hold: 4, angles: { ...SIT_HANDS, root: -4, head: 2, legL_thigh: -78, legL_shin: 75, legR_thigh: -70, legR_shin: 67 }, flare: 0.04, sway: 0 },
      { hold: 10, angles: { ...SIT_HANDS, root: -4, head: 2, legL_thigh: -78, legL_shin: 66, legR_thigh: -70, legR_shin: 74 }, flare: 0.04, sway: -1.5 },
      { hold: 4, angles: { ...SIT_HANDS, root: -4, head: 2, legL_thigh: -78, legL_shin: 75, legR_thigh: -70, legR_shin: 67 }, flare: 0.04, sway: 0 },
      { hold: 10, angles: { ...SIT_HANDS, root: -4, head: 2, legL_thigh: -78, legL_shin: 82, legR_thigh: -70, legR_shin: 58 }, flare: 0.04, sway: 1.5 },
      { hold: 4, angles: { ...SIT_HANDS, root: -4, head: 2, legL_thigh: -78, legL_shin: 75, legR_thigh: -70, legR_shin: 67 }, flare: 0.04, sway: 0 },
      { hold: 10, angles: { ...SIT_HANDS, root: -4, head: 2, legL_thigh: -78, legL_shin: 66, legR_thigh: -70, legR_shin: 74 }, flare: 0.04, sway: -1.5 },
      { hold: 4, angles: { ...SIT_HANDS, root: -4, head: 2, legL_thigh: -78, legL_shin: 75, legR_thigh: -70, legR_shin: 67 }, flare: 0.04, sway: 0 },
      { hold: 10, angles: { ...SIT_HANDS, root: -4, head: 2, legL_thigh: -78, legL_shin: 82, legR_thigh: -70, legR_shin: 58 }, flare: 0.04, sway: 1.5 },
      { hold: 20, angles: { ...SIT_HANDS, root: -4, head: 7, neck: 2, legL_thigh: -78, legL_shin: 77, legR_thigh: -70, legR_shin: 69 }, flare: 0.03, sway: 0 },
    ],
  },
  /** Thrown: flailing on ones, a trail behind her (her spin comes from how fast she's going). */
  fly: {
    loop: true,
    priority: 3,
    keys: [
      { hold: 1, sy: 1.04, echo: 7, angles: { armL_up: 110, armL_fore: 30, armR_up: -140, armR_fore: -40, legL_thigh: -24, legL_shin: 30, legR_thigh: 20, legR_shin: -20, head: -10 }, flare: 0.5, sway: 6, face: "wide" },
      { hold: 1, sy: 1.06, echo: 9, angles: { armL_up: 70, armL_fore: -10, armR_up: -100, armR_fore: 10, legL_thigh: 10, legL_shin: 6, legR_thigh: -16, legR_shin: 26, head: -4 }, flare: 0.6, sway: -6, face: "wide" },
    ],
  },
  /** Talking: little head bobs and a hand that comes up with a point she's making. */
  talk: {
    loop: true,
    priority: 0,
    keys: [
      { hold: 6, dy: -2, angles: { head: 3, neck: 1 }, flare: 0.05, sway: -1 },
      { hold: 4, dy: -3, angles: { head: -2, armR_up: -30, armR_fore: -60 }, flare: 0.08, sway: 0 },
      { hold: 6, dy: -3, angles: { head: -4, armR_up: -34, armR_fore: -66, armR_hand: 10 }, flare: 0.07, sway: 1 },
      { hold: 6, dy: -2, angles: { head: 2, armR_up: -10, armR_fore: -20 }, flare: 0.05, sway: 0 },
      { hold: 8, dy: -1, angles: { head: 4, neck: 2 }, flare: 0.04, sway: -1 },
    ],
  },

  // ---- clicks and pats

  /** Poked (a click on her): she braces, pops up stretched with a green/crimson smear, hangs there grinning, and
   *  drops back with a little squash. The click she reacts to the most. */
  poke: {
    priority: 2,
    keys: [
      { hold: 2, dy: 4, sx: 1.1, sy: 0.88, angles: { root: 3, head: 8, armL_up: -10, armR_up: 12 }, flare: -0.08, face: "closed" },
      { hold: 2, dy: 6, sx: 1.14, sy: 0.84, angles: { root: 4, head: 10, armL_up: -14, armR_up: 16, legL_thigh: -4, legR_thigh: 4 }, flare: -0.1, face: "closed" },
      { hold: 2, dy: -22, sx: 0.86, sy: 1.18, echo: 9, angles: { root: -2, head: -6, armL_up: 60, armR_up: -70, legL_thigh: 4, legR_thigh: -4 }, flare: 0.4, sway: -4, face: "wide", cue: "sparkle" },
      { hold: 2, dy: -30, sx: 0.92, sy: 1.1, echo: 4, angles: { root: -3, head: -8, armL_up: 78, armR_up: -96, armR_fore: -20, legL_thigh: 8, legR_thigh: -8 }, flare: 0.5, sway: -2, face: "wide" },
      { hold: 6, dy: -32, sx: 1, sy: 1.04, angles: { root: -3, head: -10, armL_up: 82, armR_up: -102, armR_fore: -24, legL_thigh: 9, legR_thigh: -9 }, flare: 0.46, sway: 2, face: "smug" },
      { hold: 2, dy: -18, sx: 0.96, sy: 1.06, angles: { root: -1, head: -4, armL_up: 40, armR_up: -50, legL_thigh: 4, legR_thigh: -4 }, flare: 0.3, face: "smug" },
      { hold: 2, dy: 3, sx: 1.08, sy: 0.92, angles: { root: 2, head: 6, armL_up: -6, armR_up: 8 }, flare: 0.1, face: "smug", cue: "dust" },
      { hold: 2, dy: -2, sx: 0.98, sy: 1.02, angles: { root: 0, head: -2 }, flare: 0.14, face: "smug" },
      { hold: 10, dy: 0, angles: { head: 4 }, flare: 0.1, face: "smug" },
    ],
  },
  /** A giggle: her shoulders shake on ones, head tipped back, mane bouncing. */
  giggle: {
    priority: 2,
    keys: [
      { hold: 2, dy: 2, sy: 0.96, angles: { root: 2, head: -12, armR_up: -40, armR_fore: -110 }, flare: 0.12, face: "closed" },
      ...rep(4, [
        { hold: 2, dy: -2, angles: { root: -3, head: -14, armR_up: -42, armR_fore: -112 }, flare: 0.18, sway: 3, face: "closed" },
        { hold: 2, dy: 1, angles: { root: 3, head: -10, armR_up: -38, armR_fore: -108 }, flare: 0.1, sway: -3, face: "closed" },
      ]),
      { hold: 10, dy: 0, angles: { head: -4 }, flare: 0.08, face: "smug", cue: "sparkle" },
    ],
  },
  /** A full spin in place, arms out, smeared on the turn (the flip happens on the smear frames). */
  spin: {
    priority: 2,
    keys: [
      { hold: 2, dy: 3, sx: 1.06, sy: 0.94, angles: { root: 6, armL_up: -10, armR_up: 10 }, face: "smug" },
      { hold: 2, dy: -6, sx: 0.7, sy: 1.08, echo: 12, angles: { armL_up: 70, armR_up: -80 }, flare: 0.35, sway: 6, face: "wide" },
      { hold: 2, dy: -8, sx: -0.6, sy: 1.08, echo: 14, angles: { armL_up: 80, armR_up: -90 }, flare: 0.45, sway: 8, face: "wide", cue: "flip" },
      { hold: 2, dy: -8, sx: -1, sy: 1.04, echo: 8, angles: { armL_up: 70, armR_up: -80 }, flare: 0.4, sway: 6, face: "wide" },
      { hold: 2, dy: -6, sx: -0.6, sy: 1.06, echo: 12, angles: { armL_up: 60, armR_up: -70 }, flare: 0.35, sway: -6, face: "wide", cue: "flip" },
      { hold: 2, dy: -4, sx: 0.8, sy: 1.04, echo: 6, angles: { armL_up: 40, armR_up: -50 }, flare: 0.3, sway: -4, face: "smug" },
      { hold: 2, dy: 2, sx: 1.06, sy: 0.95, angles: { root: -3 }, flare: 0.16, face: "smug" },
      { hold: 12, dy: 0, angles: { head: 3 }, flare: 0.1, face: "smug", cue: "sparkle" },
    ],
  },
  /** A hop: crouch, stretch up, hang at the top with arms up, squash on landing. */
  hop: {
    priority: 2,
    keys: [
      { hold: 3, dy: 8, sx: 1.12, sy: 0.86, angles: { root: 2, head: 6, legL_thigh: -12, legL_shin: 18, legR_thigh: 12, legR_shin: -16 }, face: "closed" },
      { hold: 2, dy: -26, sx: 0.88, sy: 1.16, echo: 6, angles: { armL_up: 40, armR_up: -50, legL_thigh: 6, legR_thigh: -6 }, flare: 0.3, face: "wide" },
      { hold: 6, dy: -38, sy: 1.02, angles: { armL_up: 90, armR_up: -110, legL_thigh: -16, legL_shin: 30, legR_thigh: 14, legR_shin: -24 }, flare: 0.4, face: "smug" },
      { hold: 2, dy: -14, sy: 1.08, angles: { armL_up: 50, armR_up: -60 }, flare: 0.25, face: "smug" },
      { hold: 3, dy: 6, sx: 1.12, sy: 0.86, angles: { root: 2, legL_thigh: -10, legL_shin: 14, legR_thigh: 10, legR_shin: -12 }, flare: 0.05, face: "closed", cue: "dust" },
      { hold: 8, dy: 0, flare: 0.08, face: "smug" },
    ],
  },
  /** Flinch: she jerks back and up, eyes wide, then eases down. */
  flinch: {
    priority: 2,
    keys: [
      { hold: 2, dx: 6, dy: -6, sx: 0.92, sy: 1.08, echo: 5, angles: { root: 10, head: 14, armL_up: 40, armR_up: -60, armR_fore: -40 }, flare: 0.35, face: "wide" },
      { hold: 8, dx: 8, dy: -8, angles: { root: 12, head: 16, armL_up: 46, armR_up: -66, armR_fore: -46 }, flare: 0.4, face: "wide" },
      { hold: 4, dx: 4, dy: -3, angles: { root: 6, head: 8, armL_up: 20, armR_up: -30 }, flare: 0.2, face: "calm" },
    ],
  },

  // ---- answers

  /** A nod: done, as asked. */
  nod: {
    priority: 1,
    keys: [
      { hold: 3, angles: { head: -6, neck: -2 }, face: "smug" },
      { hold: 3, dy: 1, angles: { head: 12, neck: 5 }, face: "closed" },
      { hold: 3, angles: { head: -3 }, face: "smug" },
      { hold: 3, dy: 1, angles: { head: 9, neck: 3 }, face: "closed" },
      { hold: 6, angles: { head: 0 }, face: "smug" },
    ],
  },
  /** Victory: a fist to the sky, stretched tall, glitching with joy. */
  victory: {
    priority: 2,
    keys: [
      { hold: 3, dy: 5, sx: 1.08, sy: 0.9, angles: { root: 4, armR_up: 20, armR_fore: 30 }, face: "closed" },
      { hold: 2, dy: -14, sx: 0.9, sy: 1.14, echo: 8, angles: { root: -6, head: -10, armR_up: -168, armR_fore: -8, armR_hand: -10, armL_up: 30 }, flare: 0.5, sway: -5, face: "wide", cue: "glitch" },
      { hold: 14, dy: -16, sy: 1.06, angles: { root: -6, head: -12, armR_up: -172, armR_fore: -4, armL_up: 34 }, flare: 0.55, sway: 3, face: "smug", cue: "sparkle" },
      { hold: 4, dy: -6, angles: { root: -2, head: -4, armR_up: -90, armL_up: 10 }, flare: 0.25, face: "smug" },
      { hold: 6, dy: 0, flare: 0.1, face: "smug" },
    ],
  },
  /** The battery running out: she sags, head down, arms limp, mane flat; a start, and she sags again. */
  drained: {
    priority: 2,
    keys: [
      { hold: 6, dy: 3, angles: { spine: 3, neck: 4, head: 7, armL_up: -6, armR_up: 6 }, flare: -0.12, face: "calm" },
      { hold: 28, dy: 8, sx: 1.02, sy: 0.96, angles: { spine: 6, neck: 8, head: 14, armL_up: -10, armR_up: 10, armR_fore: 6 }, flare: -0.26, sway: -1, face: "closed" },
      { hold: 2, dy: 2, angles: { spine: 2, neck: 2, head: 5, armL_up: -4, armR_up: 4 }, flare: -0.1, face: "wide" },
      { hold: 20, dy: 7, angles: { spine: 5, neck: 7, head: 11, armL_up: -9, armR_up: 9, armR_fore: 5 }, flare: -0.22, face: "closed" },
      { hold: 4, dy: 2, angles: { head: 6, armL_up: -4, armR_up: 4 }, flare: -0.05 },
      { hold: 2, angles: {} },
    ],
  },
  /** Into the window she's sitting on (taking it over): her head cocked down at it, her legs swing up, she coils, and
   *  stretches down into it, gone. Played as her vanish. */
  dive: {
    priority: 4,
    keys: [
      { hold: 8, angles: { root: -4, head: 12, neck: 4, legL_thigh: -78, legL_shin: 75, legR_thigh: -70, legR_shin: 67 }, flare: 0.1, face: "smug" },
      { hold: 4, dy: -6, angles: { root: -8, head: 8, legL_thigh: -110, legL_shin: 100, legR_thigh: -104, legR_shin: 96, armL_up: 30, armR_up: -30 }, flare: 0.2, face: "wide" },
      { hold: 6, dy: -12, sx: 1.08, sy: 0.85, angles: { root: -6, head: 16, neck: 6, legL_thigh: -120, legL_shin: 120, legR_thigh: -116, legR_shin: 116, armL_up: 60, armR_up: -70 }, flare: 0.32, face: "smug" },
      { hold: 2, dy: 30, sx: 0.4, sy: 1.5, echo: 12, angles: { armL_up: 150, armR_up: -160 }, flare: 0.6, face: "closed", cue: "glitch", sound: "zap-out" },
      { hold: 2, dy: 60, sx: 0.1, sy: 1.9, echo: 20, angles: { armL_up: 160, armR_up: -170 }, flare: 1, face: "closed" },
    ],
  },
  /** Into her biochip: a little cock of her head at you, then her mane and arms pulled in tight, and she's gone small. */
  fold: {
    priority: 4,
    keys: [
      { hold: 4, angles: { head: 8, neck: 3 }, flare: -0.05, face: "smug" },
      { hold: 3, sx: 0.9, angles: { head: 10, armL_up: -14, armR_up: 14 }, flare: -0.3, face: "smug" },
      { hold: 2, sx: 0.75, sy: 1.05, angles: { head: 6, armL_up: -22, armR_up: 22 }, flare: -0.5, face: "closed" },
      { hold: 2, sx: 0.5, sy: 1.1, echo: 4, angles: { armL_up: -26, armR_up: 26 }, flare: -0.6, face: "closed", cue: "glitch" },
    ],
  },
  /** It didn't work: a shrug, slumping a little, a huff. */
  shrug: {
    priority: 1,
    keys: [
      { hold: 3, dy: -3, sy: 1.04, angles: { head: -8, armL_up: 26, armL_fore: 30, armR_up: -34, armR_fore: -50 }, flare: 0.1, face: "smug" },
      { hold: 12, dy: -4, sy: 1.05, angles: { head: 10, armL_up: 30, armL_fore: 36, armR_up: -38, armR_fore: -56 }, flare: 0.12, face: "closed" },
      { hold: 4, dy: 3, sx: 1.03, sy: 0.96, angles: { root: 3, head: 6 }, flare: -0.05, face: "smug" },
      { hold: 6, dy: 0, face: "calm" },
    ],
  },
  /** Huffing: turned half away, arms folded, chin up. */
  huff: {
    priority: 2,
    keys: [
      { hold: 3, dx: -3, angles: { root: -8, head: -14, armL_up: -20, armL_fore: 60, armR_up: -60, armR_fore: -110 }, flare: 0.2, face: "smug" },
      { hold: 20, dx: -4, angles: { root: -9, head: -16, armL_up: -22, armL_fore: 64, armR_up: -64, armR_fore: -114 }, flare: 0.16, sway: -2, face: "smug" },
      { hold: 6, dx: -2, angles: { root: -4, head: -6 }, face: "calm" },
    ],
  },

  // ---- idle moments

  /** A glance one way and the other. */
  look: {
    priority: 1,
    keys: [
      { hold: 4, angles: { head: -10, neck: -4, root: -2 }, face: "calm" },
      { hold: 16, angles: { head: -12, neck: -5, root: -2 }, face: "calm" },
      { hold: 3, angles: { head: 4 }, face: "closed" },
      { hold: 16, angles: { head: 12, neck: 5, root: 2 }, face: "smug" },
      { hold: 4, angles: { head: 2 }, face: "calm" },
    ],
  },
  /** A stretch: arms up overhead, pulling tall, a slow sag back. */
  stretch: {
    priority: 1,
    keys: [
      { hold: 4, dy: 3, sy: 0.96, angles: { head: 6, armL_up: 20, armR_up: -20 }, face: "closed" },
      { hold: 4, dy: -6, sy: 1.06, angles: { head: -8, armL_up: 120, armL_fore: 30, armR_up: -150, armR_fore: -30 }, flare: 0.2, face: "closed" },
      { hold: 18, dy: -9, sy: 1.08, angles: { root: -3, head: -12, armL_up: 128, armL_fore: 36, armR_up: -158, armR_fore: -36 }, flare: 0.3, sway: 2, face: "closed" },
      { hold: 4, dy: -3, sy: 1.02, angles: { head: -2, armL_up: 50, armR_up: -60 }, flare: 0.12, face: "smug" },
      { hold: 8, dy: 0, face: "calm" },
    ],
  },
  /** A wave (hello). */
  wave: {
    priority: 1,
    keys: [
      { hold: 3, angles: { head: -6, armR_up: -140, armR_fore: -30, armR_hand: -10 }, flare: 0.1, face: "smug" },
      ...rep(3, [
        { hold: 3, angles: { head: -6, armR_up: -146, armR_fore: -46, armR_hand: -20 }, flare: 0.12, face: "smug" },
        { hold: 3, angles: { head: -4, armR_up: -138, armR_fore: -12, armR_hand: 6 }, flare: 0.1, face: "smug" },
      ]),
      { hold: 6, angles: { head: 0 }, face: "calm" },
    ],
  },
  /** Landing after being dropped: a hard squash, dust, a wobble back up. */
  land: {
    priority: 3,
    keys: [
      { hold: 2, dy: 10, sx: 1.2, sy: 0.78, angles: { root: 2, head: 10, armL_up: -20, armR_up: 24, legL_thigh: -14, legL_shin: 20, legR_thigh: 14, legR_shin: -18 }, flare: -0.1, face: "closed", cue: "dust" },
      { hold: 2, dy: -6, sx: 0.92, sy: 1.1, angles: { head: -6, armL_up: 30, armR_up: -36 }, flare: 0.3, face: "wide" },
      { hold: 2, dy: 3, sx: 1.05, sy: 0.95, angles: { head: 3 }, flare: 0.12, face: "calm" },
      { hold: 8, dy: 0, flare: 0.06, face: "smug" },
    ],
  },

  /** Shaking it off after a crash: her head side to side on ones, the mane flying, then a smug look. */
  shake: {
    priority: 2,
    keys: [
      ...rep(4, [
        { hold: 1, angles: { head: 14, neck: 4 }, flare: 0.35, sway: 7, face: "closed" },
        { hold: 1, angles: { head: -14, neck: -4 }, flare: 0.3, sway: -7, face: "closed" },
      ]),
      { hold: 8, angles: { head: 3 }, flare: 0.12, face: "smug" },
    ],
  },

  // ---- coming and going

  /** She glitches in: a sliver stretched tall, snapping into shape. */
  arrive: {
    priority: 4,
    keys: [
      { hold: 2, sx: 0.06, sy: 1.5, echo: 20, flare: 0.8, face: "closed", cue: "glitch", sound: "zap-in" },
      { hold: 2, sx: 0.4, sy: 1.25, echo: 14, flare: 0.6, face: "closed" },
      { hold: 2, sx: 1.25, sy: 0.85, echo: 6, angles: { armL_up: 40, armR_up: -50 }, flare: 0.5, face: "wide", cue: "flash" },
      { hold: 2, sx: 0.95, sy: 1.05, angles: { armL_up: 20, armR_up: -24 }, flare: 0.3, face: "wide" },
      { hold: 10, flare: 0.12, face: "smug", cue: "sparkle" },
    ],
  },
  /** She glitches out: squashed to a line, then gone (the caller hides her after). */
  vanish: {
    priority: 4,
    keys: [
      { hold: 3, dy: 4, sx: 1.08, sy: 0.92, angles: { armL_up: -10, armR_up: 12 }, face: "smug" },
      { hold: 2, dy: -10, sx: 0.6, sy: 1.3, echo: 12, flare: 0.6, face: "closed", cue: "glitch", sound: "zap-out" },
      { hold: 2, dy: -14, sx: 0.12, sy: 1.6, echo: 20, flare: 0.8, face: "closed" },
      { hold: 2, dy: -14, sx: 0.02, sy: 1.8, echo: 26, flare: 1, face: "closed", cue: "flash" },
    ],
  },
};

/** Pointing at a place (`dx`, `dy` from her shoulder, any units): a quick reach, a hold, back. Things to her right get
 *  her near arm, things to her left her far one (the near one would cross behind her mane). */
/** Which arm points at (`dx`, `dy`) from her shoulder (the far one for anything well off to her left), and how far its
 *  upper arm turns to do it. */
function aim(dx: number, dy: number, far = dx < -30) {
  // Turning a straight-down arm clockwise by `t` points it at (-sin t, cos t). In the cut-out her near arm hangs 13.7°
  // off straight down and her far one 56° (it's flung out); their rest angles turn them a little more.
  const t = (Math.atan2(-dx, dy) * 180) / Math.PI;
  const [up, fore, hand] = far ? ["armL_up", "armL_fore", "armL_hand"] : ["armR_up", "armR_fore", "armR_hand"];
  return { up, fore, hand, a: far ? t - 56 - (REST.armL_up ?? 0) : t + 13.7 - (REST.armR_up ?? 0) };
}

/** The satellite (`at(frame)`: where it is from her shoulder, frames from now, at 24 fps): a glint, she snaps to it,
 *  her specks catch its light, her hand comes up and follows it as it goes, it twinkles back, her hand drops and she
 *  sighs (the boot-up Capri's gaze). About five seconds. Cues: "starlight" (her specks flare), "glimmer" (it twinkles). */
export function gazeAt(at: (frame: number) => [number, number]): Act {
  const [x0] = at(0);
  const far = x0 < -30;
  const look = (k: number) => ({ head: -20 * k, neck: -6 * k });
  const keys: Drawing[] = [
    { hold: 6, angles: {}, sound: "star" },
    { hold: 2, dy: 1, angles: { head: 3 }, flare: 0.02 },
    { hold: 2, echo: 4, angles: look(0.7), flare: 0.14, sway: -2, face: "wide" },
    { hold: 12, angles: look(1), flare: 0.1, face: "wide" },
    { hold: 12, angles: look(1.1), flare: 0.14, face: "calm", cue: "starlight" },
  ];
  // Her hand comes up toward it, then follows it, a drawing every few frames.
  let f = 34;
  const reach = (k: number, hold: number, extra: Partial<Drawing> = {}) => {
    const [dx, dy] = at(f);
    const r = aim(dx, dy, far);
    keys.push({ hold, dy: -Math.round(4 * k), angles: { ...look(1.15), [r.up]: r.a * k, [r.fore]: -(REST[r.fore] ?? 0) * k - 6 * (1 - k), [r.hand]: -4 * k }, flare: 0.15 + 0.03 * k, face: "calm", ...extra });
    f += hold;
  };
  reach(0.25, 6);
  reach(0.55, 6);
  reach(0.8, 4);
  reach(0.93, 4);
  for (let i = 0; i < 4; i++) reach(1, 7);
  reach(1.02, 2, { cue: "glimmer", flare: 0.24 });
  reach(1, 8);
  reach(1, 8);
  reach(0.45, 4, { face: "smug" });
  reach(0.12, 4, { face: "closed" });
  keys.push({ hold: 16, dy: 1, angles: { head: 4, neck: 1 }, flare: -0.04, face: "closed" });
  keys.push({ hold: 2, angles: {} });
  return { priority: 1, keys };
}

/** Jealous (you're with another AI), at you (`dir` 1 right, -1 left): she snaps to you with a twitch, leans in, her
 *  mane bristling, her arms pulled in, holds it, hisses, holds, and lets it go. */
export function glareAt(dir: number): Act {
  return {
    priority: 2,
    keys: [
      { hold: 2, echo: 4, angles: { head: 6 * dir, root: 2 * dir }, flare: 0.32, cue: "twitch", face: "wide" },
      { hold: 18, angles: { head: 10 * dir, root: 3 * dir, armL_up: -13, armR_up: 13 }, flare: 0.36, face: "smug" },
      { hold: 2, echo: 4, angles: { head: 13 * dir, root: 3.6 * dir, armL_up: -17, armR_up: 17 }, flare: 0.46, face: "wide", sound: "jealous" },
      { hold: 24, angles: { head: 11 * dir, root: 3 * dir, armL_up: -14, armR_up: 14 }, flare: 0.38, face: "smug" },
      { hold: 4, angles: { head: 6 * dir, root: 1 * dir }, flare: 0.2 },
      { hold: 2, angles: {} },
    ],
  };
}

/** Alarm (a reminder, an alert), leaning toward you (`dir`): bristling and flickering on ones, her arms flared, her
 *  minds splitting off her. About two and a half seconds. */
export function alarmAt(dir: number): Act {
  const a: Drawing = { hold: 2, angles: { head: 6 * dir, root: 3 * dir, armL_up: 21, armR_up: -21 }, flare: 0.34, face: "wide" };
  const b: Drawing = { hold: 2, angles: { head: 6 * dir, root: 3 * dir, armL_up: 14, armR_up: -14 }, flare: 0.12, face: "wide" };
  return { priority: 2, keys: [{ ...a, cue: "minds" }, b, ...rep(15, [a, b])] };
}

/** Curious about something to her side (`dir` 1 right, -1 left): a dip the other way, her head snapping over to it
 *  with a smear, a long hold with her head cocked, and back. */
export function curiousAt(dir: number): Act {
  return {
    priority: 1,
    keys: [
      { hold: 2, angles: { head: -3 * dir, root: -1 * dir }, flare: 0.04 },
      { hold: 2, echo: 5, angles: { head: 12 * dir, neck: 4 * dir, root: 2 * dir }, flare: 0.15, sway: -2 * dir, face: "wide" },
      { hold: 20, angles: { head: 18 * dir, neck: 6 * dir, root: 3 * dir }, flare: 0.14, face: "wide" },
      { hold: 12, angles: { head: 15 * dir, neck: 5 * dir, root: 2 * dir }, flare: 0.1, face: "calm" },
      { hold: 2, angles: { head: 6 * dir, root: 1 * dir }, flare: 0.05, face: "smug" },
    ],
  };
}

/** The uplink: the satellite (`at(frame)` from her shoulder, as for `gazeAt`) and everything she has. She notices it,
 *  reaches, charges (cue "charge"), the red light goes up her arm to it (cue "surge") and lifts her, it hits (cue
 *  "hit"), she drops back, a silence while it falls apart and twinkles back twice, and her sparkle comes home (cue
 *  "home"). About seven seconds. */
export function uplinkAt(at: (frame: number) => [number, number]): Act {
  const far = at(0)[0] < -30;
  const look = (k: number) => ({ head: -20 * k, neck: -6 * k });
  const keys: Drawing[] = [
    { hold: 6, angles: {}, sound: "star" },
    { hold: 2, dy: 1, angles: { head: 3 }, flare: 0.02 },
    { hold: 2, echo: 4, angles: look(0.7), flare: 0.14, sway: -2, face: "wide", cue: "twitch" },
    { hold: 12, angles: look(1), flare: 0.1, face: "wide" },
    { hold: 12, angles: look(1.1), flare: 0.14, face: "calm", cue: "starlight" },
  ];
  let f = 34;
  const arm = (k: number, hold: number, extra: Partial<Drawing> = {}) => {
    const [dx, dy] = at(f);
    const r = aim(dx, dy, far);
    keys.push({ hold, angles: { ...look(1.15), [r.up]: r.a * k, [r.fore]: -(REST[r.fore] ?? 0) * k - 6 * (1 - k), [r.hand]: -4 * k }, flare: 0.15, face: "calm", ...extra });
    f += hold;
  };
  arm(0.25, 6);
  arm(0.55, 6);
  arm(0.8, 4);
  arm(1, 4);
  arm(1, 14, { dy: -2, flare: 0.3, sway: 2, cue: "charge", face: "smug" });
  arm(1.04, 2, { dy: -12, echo: 6, sy: 1.08, sx: 0.94, flare: 0.6, face: "wide", cue: "surge" });
  arm(1, 16, { dy: -16, flare: 0.5, sway: 3, face: "wide" });
  arm(1, 2, { dy: -18, echo: 4, flare: 0.7, face: "wide", cue: "hit" });
  arm(1, 10, { dy: -10, flare: 0.4, face: "wide" });
  arm(0.4, 4, { dy: -2, flare: 0.1 });
  keys.push({ hold: 24, angles: look(1), flare: 0.04, face: "calm" });
  keys.push({ hold: 30, angles: look(0.9), flare: 0.06, face: "calm" });
  keys.push({ hold: 12, dy: -1, angles: look(0.4), flare: 0.18, face: "smug", cue: "home" });
  keys.push({ hold: 2, angles: {} });
  return { priority: 2, keys };
}

export function pointAt(dx: number, dy: number): Act {
  const { up, fore, hand, a } = aim(dx, dy);
  const lean = Math.max(-6, Math.min(6, dx / 60));
  return {
    priority: 1,
    keys: [
      { hold: 2, angles: { [up]: a * 0.6, [fore]: -20, root: -lean * 0.5 }, face: "calm" },
      { hold: 2, echo: 3, angles: { [up]: a * 1.05, [fore]: 4, [hand]: 0, root: -lean, head: -lean }, flare: 0.12, face: "wide" },
      { hold: 20, angles: { [up]: a, [fore]: -(REST[fore] ?? 0), [hand]: -4, root: -lean, head: -lean }, flare: 0.08, face: "smug" },
      { hold: 3, angles: { [up]: a * 0.4, [fore]: -10 }, face: "smug" },
    ],
  };
}

/** Dancing at `bpm`: a bob down on each beat, side to side every other one, the mane bouncing with it. Each drawing
 *  is held half a beat (on her 24 fps clock). */
export function dance(bpm: number): Act {
  const half = Math.max(2, Math.round((24 * 60) / bpm / 2));
  return {
    loop: true,
    priority: 0,
    keys: [
      { hold: half, dy: 5, sx: 1.03, sy: 0.95, angles: { root: -5, head: 7, armL_up: 22, armR_up: -26, armR_fore: -34, legL_thigh: 4, legR_thigh: -2 }, flare: 0.1, sway: -4 },
      { hold: half, dy: -3, sy: 1.03, angles: { root: -1, head: -3, armL_up: 8, armR_up: -10, armR_fore: -12 }, flare: 0.22, sway: -1 },
      { hold: half, dy: 5, sx: 1.03, sy: 0.95, angles: { root: 5, head: -5, armL_up: 34, armR_up: -44, armR_fore: -50, legL_thigh: -4, legR_thigh: 6 }, flare: 0.1, sway: 4 },
      { hold: half, dy: -3, sy: 1.03, angles: { root: 1, head: 3, armL_up: 10, armR_up: -14, armR_fore: -16 }, flare: 0.22, sway: 1 },
    ],
  };
}

/** Walking: a stride on threes, contact and passing drawings, her arms swinging against her legs, a bob. */
export const WALK: Act = {
  loop: true,
  priority: 0,
  keys: [
    { hold: 3, dy: 3, angles: { root: -3, legL_thigh: 24, legL_shin: 6, legR_thigh: -22, legR_shin: 18, armL_up: -16, armR_up: 18, head: -2 }, flare: 0.08, sway: 2 },
    { hold: 3, dy: -2, angles: { root: -3, legL_thigh: 4, legL_shin: 22, legR_thigh: -2, legR_shin: 4, armL_up: -4, armR_up: 4, head: 0 }, flare: 0.12, sway: 0 },
    { hold: 3, dy: 3, angles: { root: -3, legL_thigh: -20, legL_shin: 16, legR_thigh: 24, legR_shin: 6, armL_up: 10, armR_up: -14, head: 2 }, flare: 0.08, sway: -2 },
    { hold: 3, dy: -2, angles: { root: -3, legL_thigh: -2, legL_shin: 4, legR_thigh: 4, legR_shin: 22, armL_up: -4, armR_up: 4, head: 0 }, flare: 0.12, sway: 0 },
  ],
};

/** Dancing sitting down (on a window's edge): her head pumping on every beat, her legs kicking in turn, her near hand
 *  staying on the ledge and her far fist punching up on the beat (high, then a shorter pump), back down on the ledge
 *  between (the far arm: dark, it shows against her mane). */
export function sitDance(bpm: number): Act {
  const half = Math.max(2, Math.round((24 * 60) / bpm / 2));
  const legs = { legL_thigh: -78, legL_shin: 80, legR_thigh: -70, legR_shin: 66 };
  return {
    loop: true,
    priority: 0,
    keys: [
      { hold: half, dy: 2, angles: { ...SIT_HANDS, ...legs, root: -4, head: 12, neck: 4, legL_shin: 40, armL_up: 120, armL_fore: 30, armL_hand: 0 }, flare: 0.16, sway: -3 },
      { hold: half, dy: -1, angles: { ...SIT_HANDS, ...legs, root: -4, head: -4 }, flare: 0.06, sway: 0 },
      { hold: half, dy: 2, angles: { ...SIT_HANDS, ...legs, root: -4, head: 12, neck: 4, legR_shin: 30, armL_up: 90, armL_fore: 60, armL_hand: 0 }, flare: 0.16, sway: 3 },
      { hold: half, dy: -1, angles: { ...SIT_HANDS, ...legs, root: -4, head: -4 }, flare: 0.06, sway: 0 },
    ],
  };
}

/** A drawing made for standing, done sitting (on a window's edge, on your pointer): her legs stay over the edge, she
 *  never sinks into the ledge, and a hand that isn't doing anything (no big angle in that arm) stays where it rests on
 *  the ledge; a hand that is (a wave, a hand at her chin) does it. */
export function seated(d: Drawing): Drawing {
  const a = d.angles ?? {};
  const busy = (side: string) => ["up", "fore", "hand"].some((p) => Math.abs(a[`arm${side}_${p}`] ?? 0) >= 25);
  const hands = Object.fromEntries(Object.entries(SIT_HANDS).filter(([k]) => !busy(k[3])));
  const legs = Object.fromEntries(Object.entries(ACTS.sit.keys[0].angles ?? {}).filter(([k]) => k.startsWith("leg")));
  return { ...d, dy: Math.min(d.dy ?? 0, 0), angles: { ...a, ...hands, ...legs, root: a.root ?? -4 } };
}

/** Knocked down: lying on her side on the floor (`side` 1 or -1, the way she was flying), now and then a twitch. */
export function lyingDown(side: number): Act {
  const base = { root: 88 * side, head: -10 * side, armL_up: 30, armR_up: -20, legL_thigh: 8, legR_thigh: -6 };
  return {
    loop: true,
    priority: 3,
    keys: [
      { hold: 22, dy: 190, angles: base, flare: -0.15, face: "closed" },
      { hold: 2, dy: 188, angles: { ...base, armR_fore: -30, legL_shin: 22 }, flare: -0.05, face: "wide" },
      { hold: 16, dy: 190, angles: base, flare: -0.15, face: "closed" },
    ],
  };
}

/** Getting up after a crash: curling up a little further, springing up past upright on ones, landing, settling. */
export function gettingUp(side: number): Act {
  return {
    priority: 3,
    keys: [
      { hold: 3, dy: 186, sx: 1.05, sy: 0.93, angles: { root: 97 * side, legL_thigh: -30, legL_shin: 40, legR_thigh: -24, legR_shin: 34, armL_up: -20, armR_up: 20 }, flare: -0.28, face: "closed" },
      { hold: 1, dy: 120, sx: 0.9, sy: 1.15, echo: 8, angles: { root: 62 * side, armL_up: 60, armR_up: -50 }, flare: 0.3, face: "wide" },
      { hold: 1, dy: 50, sx: 0.94, sy: 1.08, echo: 5, angles: { root: 28 * side, armL_up: 40, armR_up: -36 }, flare: 0.4, face: "wide" },
      { hold: 2, dy: -4, sx: 1.12, sy: 0.86, angles: { root: -8 * side, armL_up: 6, armR_up: -6 }, flare: 0.22, face: "closed", cue: "dust" },
      { hold: 2, sx: 0.98, sy: 1.03, angles: { root: 3 * side }, flare: 0.14, face: "calm" },
      { hold: 4, flare: 0.1, face: "calm" },
    ],
  };
}

/** Plays one act. `frame()` is called at 24 fps: the drawing that's up, or `null` once a one-shot act is over. */
export class Player {
  private i = 0;
  private held = 0;
  private fired = -1;
  constructor(
    readonly name: string,
    readonly act: Act,
    private onCue: (cue: string) => void = () => {},
  ) {}

  frame(): Drawing | null {
    if (this.i >= this.act.keys.length) return null;
    const d = this.act.keys[this.i];
    if (this.fired !== this.i) {
      this.fired = this.i;
      if (d.cue) this.onCue(d.cue);
      if (d.sound) this.onCue(`sound:${d.sound}`);
    }
    if (++this.held >= d.hold) {
      this.held = 0;
      this.i++;
      if (this.act.loop && this.i >= this.act.keys.length) this.i = 0;
    }
    return d;
  }
}

/** Builds the pose for a drawing on top of her rest, plus any extra angles (looking at the pointer, say). */
export function compose(base: Pose, d: Drawing, extra: Record<string, number> = {}): Pose {
  const angles: Record<string, number> = { ...REST };
  for (const [k, v] of Object.entries(d.angles ?? {})) angles[k] = (angles[k] ?? 0) + v;
  for (const [k, v] of Object.entries(extra)) angles[k] = (angles[k] ?? 0) + v;
  return {
    ...base,
    x: base.x + (d.dx ?? 0) * base.scale * (base.flip ? -1 : 1),
    y: base.y + (d.dy ?? 0) * base.scale,
    sx: d.sx ?? 1,
    sy: d.sy ?? 1,
    angles,
    flare: d.flare ?? 0,
    sway: d.sway ?? 0,
  };
}
