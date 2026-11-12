// What she's doing: a state she stays in (floating, listening, thinking, held, asleep) and one-off acts played over
// it (a poke, a nod, a spin…). Acts run on her 24 fps clock whatever the screen's frame rate (she's drawn at 60 while
// you drag her), so their timing never changes.

import { log } from "../shared/log";
import { type Act, ACTS, type Drawing, Player } from "./acting";

const FRAME_MS = 1000 / 24;

export class Actor {
  private state = "float";
  private statePlayer: Player;
  private shot: Player | null = null;
  private done: (() => void) | null = null;
  private acc = 0;
  private last: Drawing;
  /** Glitched out (a vanish just ended): nothing of her is drawn until she's brought back by any act or state. (The
   *  drawing after a vanish used to be her standing there whole again, for a frame, before she moved.) */
  gone = false;
  private goneAt = 0;

  constructor(private onCue: (cue: string) => void) {
    this.statePlayer = new Player("float", ACTS.float, onCue);
    this.last = ACTS.float.keys[0];
  }

  /** The looping state she's in. */
  get current() {
    return this.state;
  }

  /** The act her state plays now (the sit itself, or a dance made for the music). */
  get stateAct() {
    return this.statePlayer.act;
  }

  /** The one-off act playing, if any. */
  get acting() {
    return this.shot?.name ?? null;
  }

  setState(name: string) {
    this.gone = false;
    if (name === this.state || !ACTS[name]) return;
    this.setStateAct(name, ACTS[name]);
  }

  /** A state made on the spot (dancing at a tempo). */
  setStateAct(name: string, act: Act) {
    if (name !== this.state) log("state", { name });
    this.state = name;
    this.statePlayer = new Player(name, act, this.onCue);
  }

  /** Plays an act over her state; resolves when it's over (or cut off by a stronger one). A weaker act than the one
   *  playing is dropped. */
  play(name: string): Promise<void> {
    const act = ACTS[name];
    return act ? this.playAct(name, act) : Promise.resolve();
  }

  /** Plays a one-off act made on the spot (pointing at a place). */
  playAct(name: string, act: Act): Promise<void> {
    if (this.shot && this.shot.act.priority > act.priority) return Promise.resolve();
    log("act", { name });
    this.gone = false;
    this.finish();
    this.shot = new Player(name, act, this.onCue);
    return new Promise((resolve) => (this.done = resolve));
  }

  /** Moves her clock on by `ms` and gives the drawing that's up. Effects step with each frame (`step`). */
  tick(ms: number, step: () => void): Drawing {
    this.acc += Math.min(ms, 250);
    // Never lost for good: whatever forgot to bring her back, she's back after a few seconds.
    if (this.gone && Date.now() - this.goneAt > 4000) this.gone = false;
    while (this.acc >= FRAME_MS) {
      this.acc -= FRAME_MS;
      this.last = this.advance();
      step();
    }
    return this.last;
  }

  private advance(): Drawing {
    if (this.shot) {
      const d = this.shot.frame();
      if (d) return d;
      if (this.shot.name === "vanish") {
        this.gone = true;
        this.goneAt = Date.now();
      }
      this.finish();
    }
    return this.statePlayer.frame() ?? ACTS.float.keys[0];
  }

  private finish() {
    this.shot = null;
    const done = this.done;
    this.done = null;
    done?.();
  }
}
