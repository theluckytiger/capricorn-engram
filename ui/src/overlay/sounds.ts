// Her sound effects, made on the spot with Web Audio (nothing to download or ship): air rushing past when she's
// thrown, the slam when she hits something, a thump when she lands, the whoosh of springing back up, a tick when
// she's picked up, the zap of her teleporting, her panel's chatter, red lightning, a hiss when she's jealous, the
// satellite's ping, and the chimes for Claude finishing, needing you, or failing. The same sounds as the boot-up
// Capri's, made again here. Quiet, under her voice; off in Settings › Voice.

let ctx: AudioContext | null = null;
let out: GainNode | null = null;
let noise: AudioBuffer | null = null;
let on = true;

/** Sounds on or off (Settings › Voice › Sounds). */
export function setSounds(enabled: boolean) {
  on = enabled;
}

function start(): AudioContext | null {
  if (!on) return null;
  if (!ctx) {
    ctx = new AudioContext();
    out = ctx.createGain();
    out.gain.value = 0.55;
    // A soft clip, so several at once never get harsh.
    const clip = ctx.createWaveShaper();
    clip.curve = Float32Array.from({ length: 512 }, (_, i) => Math.tanh(((i / 511) * 2 - 1) * 1.8) / Math.tanh(1.8));
    out.connect(clip).connect(ctx.destination);
    noise = ctx.createBuffer(1, ctx.sampleRate, ctx.sampleRate);
    const d = noise.getChannelData(0);
    for (let i = 0; i < d.length; i++) d[i] = Math.random() * 2 - 1;
  }
  if (ctx.state === "suspended") void ctx.resume();
  return ctx;
}

/** A gain that rises fast to `peak` and dies away over `len` seconds. */
function envelope(c: AudioContext, at: number, peak: number, len: number): GainNode {
  const g = c.createGain();
  g.gain.setValueAtTime(0.0001, at);
  g.gain.exponentialRampToValueAtTime(Math.max(0.0002, peak), at + 0.006);
  g.gain.exponentialRampToValueAtTime(0.0001, at + len);
  g.connect(out!);
  return g;
}

/** A tone sliding from `from` to `to` Hz. */
function tone(c: AudioContext, type: OscillatorType, from: number, to: number, at: number, len: number, peak: number) {
  const o = c.createOscillator();
  o.type = type;
  o.frequency.setValueAtTime(from, at);
  if (to !== from) o.frequency.exponentialRampToValueAtTime(to, at + len * 0.9);
  o.connect(envelope(c, at, peak, len));
  o.start(at);
  o.stop(at + len + 0.05);
}

/** Noise through a band that sweeps from `from` to `to` Hz. */
function hiss(c: AudioContext, at: number, len: number, peak: number, from: number, to: number, q = 1.5) {
  const src = c.createBufferSource();
  src.buffer = noise;
  const band = c.createBiquadFilter();
  band.type = "bandpass";
  band.Q.value = q;
  band.frequency.setValueAtTime(from, at);
  band.frequency.exponentialRampToValueAtTime(to, at + len);
  src.connect(band).connect(envelope(c, at, peak, len));
  src.start(at, Math.random() * 0.5);
  src.stop(at + len + 0.05);
}

/** Thrown (`power` 0..1): air tearing past her, rising with how hard, a falling buzz under it. */
export function whoosh(power: number) {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  const k = 0.45 + power * 0.55;
  hiss(c, t, 0.3, 0.2 * k, 400, 2800 + power * 3500, 1.2);
  tone(c, "sawtooth", 380 + power * 260, 85, t, 0.24, 0.045 * k);
}

/** A hit (`power` 0..1): a deep thump, a crunch of glass on the hard ones, her signal stuttering. */
export function crash(power: number) {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  const k = 0.35 + power * 0.65;
  tone(c, "triangle", 140, 36, t, 0.24, 0.36 * k);
  hiss(c, t, 0.14, 0.22 * k, 2600, 280, 1.1);
  if (power > 0.25) for (let i = 0; i < 6; i++) hiss(c, t + 0.015 + i * (0.012 + Math.random() * 0.02), 0.025, 0.11 * k, 5500 + Math.random() * 3500, 7000 + Math.random() * 3000, 9);
  for (let i = 0; i < 3; i++) tone(c, "square", 800 + Math.random() * 1800, 800 + Math.random() * 1800, t + 0.05 + i * 0.03, 0.02, 0.035 * k);
}

/** Landing: a soft, low thump. */
export function land() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  tone(c, "triangle", 160, 50, t, 0.15, 0.22);
  hiss(c, t, 0.07, 0.07, 1600, 420, 1.4);
}

/** Springing back up: a quick rising sweep. */
export function rise() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  tone(c, "sawtooth", 170, 1300, t, 0.2, 0.06);
  hiss(c, t, 0.18, 0.1, 600, 5500, 2);
}

/** Picked up: a tiny digital tick. */
export function tick() {
  const c = start();
  if (!c) return;
  hiss(c, c.currentTime, 0.035, 0.07, 3200, 8500, 7);
}

/** Feet scraping along the floor. */
export function scrape(power: number) {
  const c = start();
  if (!c) return;
  hiss(c, c.currentTime, 0.12, 0.05 * (0.4 + power * 0.6), 1800, 900, 3);
}

/** A MIDI note's frequency. */
const hz = (n: number) => 440 * 2 ** ((n - 69) / 12);

/** Teleporting: a buzz sweeping up and away as she goes, down and in as she comes (with a thud), a bit-crushed
 *  stutter over it. */
export function zap(arriving: boolean) {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  tone(c, "sawtooth", arriving ? 1600 : 180, arriving ? 140 : 2400, t, 0.22, 0.11);
  hiss(c, t, 0.2, 0.2, arriving ? 6000 : 400, arriving ? 500 : 7000, 3);
  for (let i = 0; i < 4; i++) {
    const f = 800 + Math.random() * 2400;
    tone(c, "square", f, f, t + 0.02 + i * 0.035, 0.02, 0.045);
  }
  if (arriving) tone(c, "triangle", 70, 70, t + 0.16, 0.18, 0.28);
}

// Her chatter: notes of a minor pentatonic, each three voices at once (her minds), a little higher when she's alarmed
// and lower when something broke. Never closer together than 45 ms.
const CHATTER = [62, 65, 67, 69, 72, 74, 77, 79];
let chattered = 0;
export function blip(mood: "talk" | "alert" | "error" = "talk") {
  const now = performance.now();
  if (now - chattered < 45) return;
  chattered = now;
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  const f = hz(CHATTER[Math.floor(Math.random() * CHATTER.length)] + (mood === "alert" ? 5 : mood === "error" ? -7 : 0));
  tone(c, "square", f, f, t, 0.035, 0.03);
  tone(c, "triangle", f * 1.498, f * 1.498, t + 0.004, 0.03, 0.022);
  tone(c, "square", f * 0.501, f * 0.501, t + 0.008, 0.03, 0.016);
}

/** Red lightning: a low buzz under a crackle (`soft`: the small sparks while she works). */
export function bolt(soft = false) {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  const k = soft ? 0.35 : 1;
  tone(c, "sawtooth", 62, 58, t, 0.3 * k + 0.05, 0.07 * k);
  for (let i = 0, at = t; i < (soft ? 3 : 8); i++, at += 0.02 + Math.random() * 0.025) hiss(c, at, 0.025, 0.13 * k, 2500 + Math.random() * 3500, 4000 + Math.random() * 5000, 6);
}

/** Charging up (before the uplink): a buzz swelling and climbing, crackling more and more. */
export function charge() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  const o = c.createOscillator();
  o.type = "sawtooth";
  o.frequency.setValueAtTime(55, t);
  o.frequency.exponentialRampToValueAtTime(240, t + 0.6);
  const g = c.createGain();
  g.gain.setValueAtTime(0.0001, t);
  g.gain.exponentialRampToValueAtTime(0.065, t + 0.55);
  g.gain.exponentialRampToValueAtTime(0.0001, t + 0.66);
  o.connect(g).connect(out!);
  o.start(t);
  o.stop(t + 0.7);
  for (let i = 0; i < 6; i++) hiss(c, t + 0.08 + i * 0.09, 0.02, 0.04 + i * 0.016, 3000, 7000, 6);
}

/** A window she's taken over laughing with all her voices: three falling chirps, each a detuned chord, over a hiss. */
export function possess() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  [79, 76, 72].forEach((n, i) => {
    const at = t + i * 0.11;
    tone(c, "square", hz(n), hz(n), at, 0.07, 0.045);
    tone(c, "square", hz(n) * 1.012, hz(n) * 1.012, at + 0.004, 0.06, 0.032);
    tone(c, "triangle", hz(n - 12), hz(n - 12), at + 0.008, 0.07, 0.036);
  });
  hiss(c, t, 0.3, 0.045, 1200, 5000, 3);
}

/** A reminder: three bright notes going up, softer than an alert, and a shimmer. */
export function remind() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  [76, 81, 88].forEach((n, i) => {
    tone(c, "triangle", hz(n), hz(n), t + i * 0.12, 0.35, 0.075);
    tone(c, "sine", hz(n + 12), hz(n + 12), t + i * 0.12 + 0.01, 0.25, 0.024);
  });
  hiss(c, t + 0.3, 0.4, 0.02, 8000, 12000, 6);
}

/** Jealous: a hiss over a low growl that wobbles, like a cat about to swipe. */
export function jealous() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  hiss(c, t, 0.45, 0.13, 3500, 6500, 1.2);
  const o = c.createOscillator();
  o.type = "sawtooth";
  for (let i = 0; i <= 8; i++) o.frequency.setValueAtTime(i === 0 ? 72 : i % 2 ? 64 : 80, t + i * 0.045);
  o.connect(envelope(c, t, 0.065, 0.4));
  o.start(t);
  o.stop(t + 0.45);
}

/** The satellite twinkling: two soft, high pings and a shimmer. */
export function star() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  tone(c, "sine", hz(93), hz(93), t, 0.9, 0.045);
  tone(c, "sine", hz(100), hz(100), t + 0.14, 1.2, 0.032);
  hiss(c, t + 0.05, 0.6, 0.015, 9000, 12000, 6);
}

/** Waking up / saying hi: three quick notes up and a fizz. */
export function hello() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  [69, 76, 81].forEach((n, i) => tone(c, "square", hz(n), hz(n), t + i * 0.06, 0.07, 0.055));
  hiss(c, t + 0.18, 0.08, 0.055, 2000, 8000, 5);
}

/** Claude finished: a bright arpeggio and a glitchy tail. */
export function done() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  [64, 68, 71, 75, 76].forEach((n, i) => {
    tone(c, "triangle", hz(n + 12), hz(n + 12), t + i * 0.07, 0.28, 0.08);
    tone(c, "square", hz(n), hz(n), t + i * 0.07, 0.12, 0.032);
  });
  for (let i = 0; i < 7; i++) {
    const f = 1200 + Math.random() * 3000;
    tone(c, "square", f, f, t + 0.4 + i * 0.03, 0.018, 0.032);
  }
}

/** Claude needs you: two high-low pairs. */
export function alert() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  for (let r = 0; r < 2; r++) {
    tone(c, "square", 988, 988, t + r * 0.28, 0.09, 0.065);
    tone(c, "square", 740, 740, t + r * 0.28 + 0.11, 0.11, 0.065);
  }
}

/** Something failed: a falling growl, a dull thud, a stutter. */
export function error() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  tone(c, "sawtooth", 110, 55, t, 0.32, 0.12);
  hiss(c, t, 0.25, 0.14, 900, 200, 1.2);
  for (let i = 0; i < 5; i++) hiss(c, t + 0.05 + i * 0.05, 0.02, 0.11, 4000, 4000, 10);
}

/** Plays one by name (from a drawing's `sound`); false if there's no such sound. */
/** Petted: a purr, a low rumble throbbing about 24 times a second, with a little crackle of her circuits in it. */
export function purr() {
  const c = start();
  if (!c) return;
  const t = c.currentTime;
  const len = 1.6;
  const src = c.createBufferSource();
  src.buffer = noise;
  const low = c.createBiquadFilter();
  low.type = "lowpass";
  low.frequency.value = 170;
  const throb = c.createGain();
  throb.gain.value = 0.5;
  const lfo = c.createOscillator();
  lfo.frequency.setValueAtTime(22, t);
  lfo.frequency.linearRampToValueAtTime(26, t + len);
  const depth = c.createGain();
  depth.gain.value = 0.5;
  lfo.connect(depth).connect(throb.gain);
  src.connect(low).connect(throb).connect(envelope(c, t, 0.55, len));
  src.start(t, Math.random() * 0.5);
  src.stop(t + len + 0.05);
  lfo.start(t);
  lfo.stop(t + len + 0.05);
  tone(c, "triangle", 62, 58, t, len, 0.05);
  hiss(c, t + 0.3, 0.08, 0.03, 3200, 2400, 6);
  hiss(c, t + 0.9, 0.06, 0.025, 2800, 3600, 6);
}

export function play(name: string): boolean {
  const all: Record<string, () => void> = {
    "zap-out": () => zap(false),
    "zap-in": () => zap(true),
    bolt: () => bolt(),
    spark: () => bolt(true),
    charge,
    possess,
    remind,
    jealous,
    star,
    hello,
    done,
    alert,
    error,
    land,
    rise,
    tick,
    purr,
  };
  all[name]?.();
  return name in all;
}
