// Her music mini player: what's playing (Windows' own media session, the one its volume flyout shows), and back,
// play/pause and next for it, in a little deck above the taskbar on her screen while music plays. She sits on it.

import { invoke } from "../shared/tauri";
import { log } from "../shared/log";

export interface Playing {
  title: string;
  artist: string;
  app: string;
  playing: boolean;
}

/** Apps that play videos as often as music. */
const BROWSERS = ["brave", "chrome", "edge", "firefox", "opera", "vivaldi", "arc", "msedge"];

/** How long it stays up paused (ms), so you can press play on it again. */
const PAUSED_FOR = 3 * 60_000;

export class Player {
  readonly el: HTMLDivElement;
  private title: HTMLDivElement;
  private artist: HTMLDivElement;
  private app: HTMLSpanElement;
  private toggle: HTMLButtonElement;
  /** What it showed last, to tell a new song. */
  private last = "";
  /** When it last saw something playing (ms), for hiding a while after the music stops. */
  private heard = 0;
  /** Shut by you from her menu, until the next song. */
  private shut = "";
  /** Browsers that have played music lately, and when: back from a pause they don't have to prove it again (with a song
   *  her ear can't find a steady beat in, the deck never came back). */
  private trusted = new Map<string, number>();
  /** She's just put music on herself: until then (ms) a browser needs no beat. */
  private expected = 0;
  /** Reads in a row with nothing playing: one gap between songs isn't the music stopping. */
  private misses = 0;
  /** Where you dragged it (CSS px); null = default bottom-right above the taskbar. */
  private pinned: { x: number; y: number } | null = null;
  /** Something changed that her hit area should know about. */
  onChange = () => {};
  /** A new song came on. */
  onSong = (_p: Playing) => {};
  /** Whether there's a steady beat on the speakers right now (her ears): a browser playing shows the deck only then,
   *  so a video someone's watching doesn't get one. */
  beat = () => false;

  constructor() {
    this.el = document.createElement("div");
    this.el.className = "player";
    this.el.hidden = true;
    this.el.innerHTML = `
      <div class="player-top"><span class="player-app"></span><button class="player-x" title="Hide">✕</button></div>
      <div class="player-title"></div>
      <div class="player-artist"></div>
      <div class="player-keys">
        <button data-k="previous" title="Back">⏮</button>
        <button data-k="toggle" class="player-play" title="Play / pause">⏯</button>
        <button data-k="next" title="Next">⏭</button>
      </div>`;
    this.title = this.el.querySelector(".player-title")!;
    this.artist = this.el.querySelector(".player-artist")!;
    this.app = this.el.querySelector(".player-app")!;
    this.toggle = this.el.querySelector(".player-play")!;
    for (const b of this.el.querySelectorAll<HTMLButtonElement>("[data-k]")) {
      b.onclick = () => void invoke("media_key", { action: b.dataset.k }).then(() => setTimeout(() => void this.refresh(), 400));
    }
    this.el.querySelector<HTMLButtonElement>(".player-x")!.onclick = () => this.hide(true);
    document.body.appendChild(this.el);
  }

  get open() {
    return !this.el.hidden;
  }

  /** A press on one of its buttons (it's drawn behind her canvas, so presses come from there): true when there was one.
   *  It wins over her, even with her legs hanging in front of it. */
  pressButton(x: number, y: number): boolean {
    if (this.el.hidden) return false;
    for (const b of this.el.querySelectorAll<HTMLButtonElement>("button")) {
      const br = b.getBoundingClientRect();
      if (x >= br.left && x <= br.right && y >= br.top && y <= br.bottom) {
        b.click();
        return true;
      }
    }
    return false;
  }

  /** Drag from the top bar (not the buttons): true when a drag started. Presses come from her canvas over it. */
  beginDrag(e: PointerEvent): boolean {
    if (this.el.hidden || e.button !== 0) return false;
    const top = this.el.querySelector(".player-top")!.getBoundingClientRect();
    if (e.clientX < top.left || e.clientX > top.right || e.clientY < top.top || e.clientY > top.bottom) return false;
    const xBtn = this.el.querySelector(".player-x")!.getBoundingClientRect();
    if (e.clientX >= xBtn.left && e.clientX <= xBtn.right && e.clientY >= xBtn.top && e.clientY <= xBtn.bottom) return false;
    const r = this.rect();
    if (!r) return false;
    const ox = e.clientX - r.left;
    const oy = e.clientY - r.top;
    const move = (ev: PointerEvent) => {
      const w = r.width || 280;
      const h = r.height || 96;
      const x = Math.max(8, Math.min(innerWidth - w - 8, ev.clientX - ox));
      const y = Math.max(8, Math.min(innerHeight - h - 8, ev.clientY - oy));
      this.pinned = { x, y };
      this.apply(x, y);
      this.onChange();
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      log("player", { op: "moved", x: this.pinned?.x, y: this.pinned?.y });
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return true;
  }

  /** Whether (x, y) is on it at all. */
  covers(x: number, y: number): boolean {
    const r = this.rect();
    return !!r && x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  }

  /** She put music on: the deck comes up for it whatever her ear makes of the beat. */
  expectMusic() {
    this.expected = performance.now() + 60_000;
  }

  /** Its rect (CSS px of this screen), for her hit area and for sitting on. */
  rect(): DOMRect | null {
    return this.el.hidden ? null : this.el.getBoundingClientRect();
  }

  /** Bottom right of her screen, just above the taskbar (`floorY`), or where you left it if you dragged it. */
  place(floorY: number) {
    if (this.pinned) {
      this.apply(this.pinned.x, this.pinned.y);
      return;
    }
    const r = this.el.getBoundingClientRect();
    const x = Math.max(12, innerWidth - (r.width || 280) - 120);
    const y = Math.max(12, floorY - (r.height || 96) - 10);
    this.apply(x, y);
  }

  private apply(x: number, y: number) {
    const w = this.el.offsetWidth || 280;
    const h = this.el.offsetHeight || 96;
    const cx = Math.max(8, Math.min(innerWidth - w - 8, x));
    const cy = Math.max(8, Math.min(innerHeight - h - 8, y));
    this.el.style.transform = `translate(${Math.round(cx)}px, ${Math.round(cy)}px)`;
  }

  hide(byYou = false, why = "her menu") {
    if (byYou) this.shut = this.last;
    if (this.el.hidden) return;
    this.el.hidden = true;
    log("player", { op: "hidden", why: byYou ? "closed by you" : why });
    this.onChange();
  }

  /** Reads what's playing and shows it (or hides a while after it stops). */
  async refresh(): Promise<Playing | null> {
    const p = (await invoke<Playing | null>("now_playing").catch(() => null)) ?? null;
    const now = performance.now();
    if (p?.playing) this.heard = now;
    this.misses = p ? 0 : this.misses + 1;
    // Windows has nothing for a moment between songs: only a few reads in a row (about 10 s) mean it's stopped.
    if (!p && this.misses < 4) return p;
    const key = p ? `${p.app}|${p.title}|${p.artist}` : "";
    const app = p?.app.toLowerCase() ?? "";
    const browser = BROWSERS.some((b) => app.includes(b));
    // A music app always; a browser (a video, or YouTube Music) only with music on the speakers, or if it's been
    // playing music in the last half hour, or she's just put some on.
    const music = p ? !browser || this.beat() || !this.el.hidden || now < this.expected || now - (this.trusted.get(app) ?? -Infinity) < 30 * 60_000 : false;
    if (!p || !music || (!p.playing && now - this.heard > PAUSED_FOR) || key === this.shut) {
      this.hide(false, !p ? "nothing playing" : !music ? "a browser with no beat" : key === this.shut ? "closed for this song" : "paused a while");
      return p;
    }
    if (browser && p.playing) this.trusted.set(app, now);
    if (key !== this.last) {
      this.last = key;
      this.shut = "";
      this.title.textContent = p.title;
      this.artist.textContent = p.artist || " ";
      this.app.textContent = p.app.toUpperCase();
      if (p.playing) this.onSong(p);
    }
    this.toggle.textContent = p.playing ? "⏸" : "▶";
    this.el.classList.toggle("paused", !p.playing);
    if (this.el.hidden) {
      this.el.hidden = false;
      log("player", { op: "shown", app: p.app, title: p.title, playing: p.playing });
      this.onChange();
    }
    return p;
  }
}
