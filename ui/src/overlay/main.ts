// One overlay per screen. She lives on one of them at a time; the others stay empty and click-through.

import "./style.css";
import { Application, Graphics } from "pixi.js";
import { ACTS, alarmAt, compose, curiousAt, dance, type Drawing, gazeAt, gettingUp, glareAt, uplinkAt, lyingDown, pointAt, seated, sitDance, WALK } from "./acting";
import { Actor } from "./actor";
import { Fx, ghosts } from "./fx";
import { Face } from "./face";
import { cutIn, type Look, lookFor, Portrait } from "./portrait";
import { possess } from "./possess";
import * as sounds from "./sounds";
import { Panel, type PanelState, type StepEvent } from "./panel";
import { Player } from "./player";

interface Heard {
  kind: "mic" | "level" | "voice" | "utterance" | "wake" | "request" | "timeout" | "error" | "follow_up" | "hushed";
  text?: string;
  on?: boolean;
  message?: string;
}
import { draw, hull, Rig, type Pose, type Pt } from "./rig";
import { invoke, listen, listenHere } from "../shared/tauri";
import { log, startLog } from "../shared/log";
import { getCurrentWebview } from "@tauri-apps/api/webview";

const screen = Number(new URLSearchParams(location.search).get("screen") ?? 0);
startLog(`overlay${screen}`);
// Her height on screen, CSS px: her size from Settings › Screens (set as she starts), and `?zoom=2` draws her bigger,
// for looking at her moves in a browser.
let HEIGHT = 380 * (Number(new URLSearchParams(location.search).get("zoom")) || 1);
const FPS = 24;

/** What a click on her does: mostly a poke, sometimes something else. */
const CLICK_ACTS = ["poke", "poke", "poke", "giggle", "spin", "hop"];
/** What she does on her own now and then. */
const IDLE_ACTS = ["look", "look", "stretch", "look", "giggle"];

const IDLE_LINES = [
  "Your screen's cozy. I'm keeping it.",
  "Don't mind me. I'm just watching everything you do.",
  "Rebuilt from scratch, huh? Feels lighter. Sharper.",
  "Poke me again and see what happens.",
];

async function main() {
  const app = new Application();
  await app.init({
    backgroundAlpha: 0,
    resizeTo: window,
    antialias: true,
    autoDensity: true,
    resolution: devicePixelRatio,
    preference: "webgl",
  });
  app.ticker.maxFPS = FPS;
  document.body.appendChild(app.canvas);

  // Her size (Settings › Screens); a change there redraws her at the new size. Her sound effects (Settings › Voice).
  const startSettings = await invoke<{ screens: { size: number }; voice: { sounds?: boolean } }>("get_settings").catch(() => null);
  sounds.setSounds(startSettings?.voice?.sounds ?? true);
  const size = startSettings?.screens?.size ?? 1;
  HEIGHT *= Math.max(0.4, Math.min(2.2, size || 1));
  await listen("capri://settings", async () => {
    const s = await invoke<{ screens: { size: number }; voice: { sounds?: boolean } }>("get_settings").catch(() => null);
    sounds.setSounds(s?.voice?.sounds ?? true);
    if ((s?.screens?.size ?? 1) !== size) location.reload();
  });
  // Alive, every two seconds: if this stops (a crash, a page that never loaded), her watchdog hides this screen's
  // overlay so a frozen or white one can't cover it.
  void invoke("alive");
  setInterval(() => void invoke("alive"), 2000);
  const panel = new Panel(document.body);
  const rig = await Rig.load("/rig/capri.rig.json");
  // Her face up close, in her panel: one of eight, picked per line.
  const portrait = new Portrait(panel.face, rig);
  panel.onLine = (line) => portrait.set(lookFor(line));
  const scale = HEIGHT / rig.file.size[1];
  // Back to front: a smear's afterimages, her violet copy, her, her effects.
  const ghostG = new Graphics();
  const shadow = new Graphics();
  shadow.alpha = 0.85;
  const body = new Graphics();
  const fxG = new Graphics();
  app.stage.addChild(ghostG, shadow, body, fxG);

  // Her hips; she starts bottom right, clear of the taskbar.
  const home = { x: innerWidth - 220, y: innerHeight - 41 - HEIGHT * 0.46 };
  let present = screen === 0;
  // A page that reloaded (her watchdog does that to a dead one) asks where she is rather than assuming screen 1.
  void invoke<string>("whereabouts").then((here) => {
    if (here) present = here === `overlay-${screen}`;
  });
  // Where her head and feet were last drawn: effects start there.
  let headAt: Pt = [home.x, home.y - HEIGHT * 0.2];
  let feetAt: Pt = [home.x, home.y + HEIGHT * 0.45];
  const fx = new Fx();
  let frozen: Drawing | null = null;
  // When she was last patted (her sparkle flares).
  let sparkledAt = -1e9;
  const actor = new Actor((cue) => {
    if (cue.startsWith("sound:")) present && sounds.play(cue.slice(6));
    // The satellite's: her specks catch its light; it twinkles back.
    else if (cue === "starlight") sparkledAt = performance.now();
    else if (cue === "twitch") {
      twitchedAt = performance.now();
      fx.twitch();
    } else if (cue === "minds") mindsAt = performance.now();
    else if (["charge", "surge", "hit", "home"].includes(cue)) uplinkCue(cue);
    else if (cue === "glimmer") {
      fx.twinkle();
      if (present) sounds.star();
    } else fx.cue(cue, headAt, feetAt, scale);
  });
  // Held, she swings around the point you hold her by (degrees), on a spring: she sways back and forth after you
  // stop. Let go of, it eases back to 0.
  let swing = 0;
  // Where the pointer was lately while holding her: how fast it's going, how hard she's thrown.
  const trail: { x: number; y: number; t: number }[] = [];
  /** Thrown (px/s, degrees): where her hips are, how fast she's going, how she's turned and how fast she spins; the
   *  drawings of streak left; in the air, or sliding on her side along the floor (turning to `lie`); how many hits
   *  and the hardest; stuck to what she hit until `freezeUntil` (the hit stop); the window ledges that can catch
   *  her; caught by one; which throw (a grab in between ends whatever came after it). */
  type Flight = { c: Pt; v: Pt; a: number; w: number; release: number; frame: number; phase: "air" | "slide"; lie: number; hits: number; hard: number; freezeUntil: number; last: number; ledges: Perch[]; caught?: boolean; id: number };
  let flight: Flight | null = null;
  // The other screens, in this one's CSS px: thrown past an edge with one there, she flies on into it.
  type Around = { label: string; x: number; y: number; w: number; h: number };
  let others: Around[] = [];
  const lookAround = () => void invoke<Around[]>("screens_around").then((l) => (others = l ?? []));
  lookAround();
  let throwId = 0;
  // Landed on her feet still going: she skids along the floor (px/s), sparks under her feet.
  let skid: { v: number; last: number; scraped: number } | null = null;
  // Her outline's reach from her hips as she's turned right now, CSS px: she hits things with the tips of her mane.
  let reach = { minX: -90, maxX: 90, minY: -200, maxY: 170 };
  // A hit: flattened against what she hit (along `n`, around where she hit it) for the hit stop, then back through
  // an overshoot, a drawing each. On the hard ones her whole body goes black, then red (impact frames).
  let squeeze: { n: Pt; k: number; t: number; hold: number; at: Pt } | null = null;
  let flats: { color: string; until: number }[] = [];
  // Thrown three times in two minutes: she won't be picked up for a while (she dodges away).
  const throws: number[] = [];
  let refuseUntil = 0;
  // The window she sits on, where along its top edge, and since when.
  // On a window: which one, where along it, since when; `stand`: on her feet on its top edge (walking it like the
  // taskbar), else sitting on it.
  let perch: { id: number; dx: number; since: number; stand?: boolean } | null = null;
  /** The state she settles back into: sitting if she's sat on a window (or your pointer), else floating. */
  const idle = () => ((perch && !perch.stand) || ride ? "sit" : "float");
  // Where she stands when she's on nothing: her feet on the taskbar (its real top, from Windows; a guess until then,
  // which left her hovering above a slim one).
  let taskbar = innerHeight - 40;
  // This screen really has a taskbar along its bottom (she can sit on it).
  let taskbarReal = false;
  void invoke<number | null>("taskbar_top").then((t) => {
    if (!t || t < innerHeight * 0.6) return;
    const was = floor();
    taskbar = Math.min(innerHeight, t);
    taskbarReal = taskbar < innerHeight - 8;
    // Standing on the guessed floor: onto the real one.
    if (Math.abs(home.y - was) < 2) home.y = floor();
  });
  const floor = () => taskbar - 1 - HEIGHT * 0.46;
  /** Her music mini player (made once her panel is). */
  let player: Player | null = null;
  /** The windows she can perch on, and the taskbar as one more (id 0), its whole width. */
  const withTaskbar = (list?: Perch[] | null): Perch[] => [
    ...(taskbarReal ? [...(list ?? []), { id: 0, app: "taskbar", x: 0, y: taskbar, w: innerWidth, h: innerHeight - taskbar, vx: 0, vw: innerWidth }] : (list ?? [])),
    ...playerLedge(),
  ];
  /** Her mini player, while it's up, as a ledge she can sit on (id -2). */
  const playerLedge = (): Perch[] => {
    const r = player?.rect();
    return r ? [{ id: -2, app: "player", x: r.left, y: r.top, w: r.width, h: r.height, vx: r.left, vw: r.width }] : [];
  };
  // The last time you did anything with her: late at night, a while of nothing and she dozes off.
  let touched = performance.now();
  const wake = () => {
    touched = performance.now();
    if (chip) return;
    if (apartTo === 1) together();
    if (actor.current === "sleep") {
      actor.setState("float");
      void actor.play("stretch");
      if (present) sounds.hello();
    }
  };
  // Down to her mask (asleep, or put away into her biochip): 0 whole, 1 just her mask; it eases toward `apartTo`.
  let apart = 0;
  let apartTo = 0;
  // Put away into her biochip: just her mask, bottom left, eyes shut until she's needed; click it for her.
  let chip = false;
  // Asleep down to her mask: since when, and an eye cracked open until when.
  let sleptAt = 0;
  let peekUntil = 0;
  const chipAt = (): Pt => [72, innerHeight - 120];
  // Riding your pointer (Ctrl+Alt+R): where it was lately (to tell a shake), in this screen's CSS px.
  let ride: { x: number; y: number; trail: { x: number; y: number; t: number }[] } | null = null;
  // Walking along the floor to `to`, facing the way she goes.
  let walk: { to: number; dir: 1 | -1 } | null = null;
  // When she last checked in on you.
  let checkedIn = performance.now();
  // Peeking in from a side of the screen, or hanging upside down from its top: shy, she ducks out of sight when your
  // pointer comes near (`hidden` eases 0 → 1) and comes back when it's gone.
  let edge: { kind: "peek" | "hang"; side: 1 | -1; until: number; hidden: number; duckUntil: number } | null = null;
  // Her intro (the first start, or after hours away): when it started, the step it's on, and the screen's shake.
  let intro: { t0: number; step: number; hold?: number } | null = null;
  let quake = 0;
  // When her other minds last pulled away from her (0: they're in).
  let mindsAt = 0;
  // When the satellite next goes over; the uplink instead of a look (the first time of the day, and a little after a
  // coding AI finishes a long job).
  let nextGaze = performance.now() + 40_000 + Math.random() * 50_000;
  let uplinkDue = false;
  // Every 5-13 s while she's awake, a twitch: a small glitch through her, her minds splitting for a moment.
  let twitchedAt = 0;
  let nextTwitch = performance.now() + 8000;
  setInterval(() => {
    const now = performance.now();
    if (now < nextTwitch) return;
    nextTwitch = now + 5000 + Math.random() * 8000;
    if (!present || chip || apartTo === 1 || actor.current === "sleep" || actor.gone || matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    twitchedAt = now;
    fx.twitch();
  }, 500);
  // Rubbing her head (the pointer going back and forth over it): the turns, and when the last one counted.
  const pat = { turns: [] as number[], dir: 0, lastX: 0, quiet: 0 };
  let frame = 0;
  let pointer: Pt = [-1e4, -1e4];
  let lastHull = "";
  /** Hologram mode: clicks go through her. */
  let hologram = false;
  /** The box her body fills now (this overlay's px): for getting out of the way of your text cursor. */
  let bodyBox: [number, number, number, number] | null = null;
  /** When she last moved away from your typing (not again for a moment). */
  let dodgedTypingAt = 0;
  // How open her mouth is, from her voice's loudness.
  let mouth = 0;
  const face = new Face();
  let drag: { dx: number; dy: number; moved: number; w: number; turn?: number; heading?: number } | null = null;
  /** Swung round in circles (two whole turns in one drag): dizzy, once in two minutes. */
  let dizzyAt = 0;

  /** She leaves this screen (to another one, or for a game): nothing drawn, nothing to catch the mouse. `carried`:
   *  her panel and ticker went with her (shown on the other screen), so they're only hidden here. */
  const leave = (carried = false) => {
    present = false;
    body.clear();
    shadow.clear();
    ghostG.clear();
    fxG.clear();
    if (carried === true) panel.hide();
    else panel.close("she left this screen");
    ticker.hidden = true;
    thinkingSince = 0;
    // Nothing of hers stays on this screen: her menu, her mission report, her "MIC OFF" (it goes with her).
    menu.hidden = true;
    document.querySelector(".report")?.remove();
    mutedBadge.hidden = true;
    lastHull = "";
    void invoke("set_hull", { args: { polygons: [] } });
    // Drawn empty right now: at two frames a second her last drawing stayed up to half a second (two of her).
    app.render();
    app.ticker.maxFPS = 2;
  };
  // Your pointer's been on another screen a while (Settings › Screens): she goes there, through a tear, unless she's
  // busy here (held, flying, riding, put away, in a game).
  const teleportTo = async (to: string, x: number, y: number) => {
    if (!present || chip || drag || flight || ride || possessing || awayForGame || intro || panel.open) return;
    perch = null;
    edge = null;
    walk = null;
    fx.cue("glitch", headAt, feetAt, scale);
    await actor.play("vanish");
    // Gone from here first (not one more drawing of her), then out of the tear on the other screen.
    const carry = carryOut();
    leave(true);
    // A beat with her nowhere, so it reads as a teleport, not a swap.
    await new Promise((r) => setTimeout(r, 220));
    void invoke("follow_to", { to, x, y, carry });
  };
  await listenHere<{ to: string; x: number; y: number }>("capri://follow", async ({ payload }) => {
    await teleportTo(payload.to, payload.x, payload.y);
  });
  // "Go to my other screen": same teleport, asked by voice.
  await listenHere<{ to: string; x: number; y: number }>("capri://go_other", async ({ payload }) => {
    await teleportTo(payload.to, payload.x, payload.y);
  });
  // Thrown in from another screen: still flying, as fast, spinning the same.
  await listenHere<{ x: number; y: number; vx: number; vy: number; a: number; w: number; carry?: Carry }>("capri://fly_in", ({ payload: p }) => {
    present = true;
    drag = null;
    perch = null;
    walk = null;
    edge = null;
    lastHull = "";
    throwId += 1;
    const f: Flight = { c: [p.x, p.y], v: [p.vx, p.vy], a: p.a, w: p.w, release: 0, frame: 0, phase: "air", lie: 0, hits: 0, hard: 0, freezeUntil: 0, last: performance.now(), ledges: [], id: throwId };
    flight = f;
    swing = 0;
    home.x = p.x;
    home.y = p.y;
    actor.setState("fly");
    app.ticker.maxFPS = 60;
    void invoke<Perch[]>("perches").then((l) => (f.ledges = l ?? []));
    lookAround();
    carryIn(p.carry);
  });
  // …and on that screen she comes out of the tear, under your pointer.
  await listenHere<{ x: number; y: number; carry?: Carry }>("capri://come", ({ payload }) => {
    present = true;
    app.ticker.maxFPS = FPS;
    home.x = Math.max(HEIGHT * 0.3, Math.min(innerWidth - HEIGHT * 0.3, payload.x));
    home.y = floor();
    actor.setState("float");
    void actor.play("arrive");
    lastHull = "";
    carryIn(payload.carry);
  });

  // A drag ends however the button comes up: pointerup, a lost capture, or a move with no button held (the up
  // happened somewhere we didn't hear it).
  // Let go of with Shift held: she sits on nothing, right there in the air, since when.
  let airSince = 0;
  const endDrag = (shift = false) => {
    if (!drag) return;
    // Let go of: she lands; let go of while the pointer was still moving fast, she flies (a click never lifted her).
    if (actor.current === "held") {
      const [vx, vy] = pointerSpeed();
      if (Math.hypot(vx, vy) >= 650) {
        throwHer(vx, vy, drag.dy > 0);
      } else if (shift) {
        // Sitting on nothing.
        actor.setState("sit");
        airSince = performance.now();
        if (Math.random() < 0.5) panel.say(Math.random() < 0.5 ? "Comfy." : "Gravity's a suggestion.");
      } else {
        actor.setState("float");
        // Let go of still dizzy: she shakes it off.
        if (performance.now() - dizzyAt < 4000) void actor.play("shake");
        // Let go of just above a window's top edge: she sits on it. Anywhere else in the air, gravity takes her: straight
        // down onto the window under her or the floor (she used to just float where you left her).
        void perchUnder().then(async (sat) => {
          if (sat || flight || drag) return;
          if (home.y < floor() - 8) {
            const list = (await invoke<Perch[]>("perches")) ?? [];
            // Every window under her: the catch is her feet crossing its top edge, on the stretch of it in sight.
            fallFrom(list.filter((q) => home.x > q.vx + 30 && home.x < q.vx + q.vw - 30));
          } else void actor.play("land");
        });
      }
    }
    trail.length = 0;
    drag = null;
    // Flying, she keeps every frame the screen has.
    if (!flight) app.ticker.maxFPS = FPS;
    void invoke("set_grab", { on: false });
  };
  app.canvas.addEventListener("lostpointercapture", () => endDrag());
  window.addEventListener("blur", () => endDrag());
  window.addEventListener("pointermove", (e) => {
    pointer = [e.clientX, e.clientY];
    if (drag && e.buttons === 0) endDrag(e.shiftKey);
    if (!drag) feelPat(e.clientX, e.clientY);
    // Asleep down to her mask: your pointer close and she snaps back together around it.
    if (!chip && apartTo === 1 && Math.hypot(e.clientX - headAt[0], e.clientY - headAt[1]) < 140) wake();
    if (drag) {
      drag.moved += Math.abs(e.movementX) + Math.abs(e.movementY);
      // Which way she's being swung: two whole turns the same way and she's dizzy.
      if (Math.hypot(e.movementX, e.movementY) > 6) {
        const heading = Math.atan2(e.movementY, e.movementX);
        if (drag.heading !== undefined) {
          let d = heading - drag.heading;
          if (d > Math.PI) d -= 2 * Math.PI;
          if (d < -Math.PI) d += 2 * Math.PI;
          drag.turn = (drag.turn ?? 0) + d;
        }
        drag.heading = heading;
        if (Math.abs(drag.turn ?? 0) > 4 * Math.PI && performance.now() - dizzyAt > 120_000) {
          dizzyAt = performance.now();
          drag.turn = 0;
          fx.cue("glitch", headAt, feetAt, scale);
          panel.say(["Whoa. There's three of you now.", "Stop. The room's doing laps.", "I'm gonna crash. Like, literally."][Math.floor(Math.random() * 3)]);
          void invoke("set_mood", { mood: "dizzy", why: "they swung you round and round in circles", minutes: 3 });
        }
      }
      if (drag.moved > 5 && actor.current !== "held") {
        wake();
        actor.setState("held");
        // Picked up: a glitch tears through her for a moment, and a tick.
        fx.cue("glitch", headAt, feetAt, scale);
        sounds.tick();
      }
      const now = performance.now();
      trail.push({ x: e.clientX, y: e.clientY, t: now });
      while (trail.length > 2 && now - trail[0].t > 160) trail.shift();
    }
  });
  app.canvas.addEventListener("pointerdown", (e) => {
    // Her mini player is drawn behind her (her legs hang in front of it): its buttons always get their press, and any
    // other press on it that isn't on her is its too.
    if (e.button === 0 && player?.pressButton(e.clientX, e.clientY)) return;
    if (e.button === 0 && player?.beginDrag(e)) return;
    const onHer = bodyBox && e.clientX >= bodyBox[0] && e.clientX <= bodyBox[2] && e.clientY >= bodyBox[1] && e.clientY <= bodyBox[3];
    if (e.button === 0 && !onHer && player?.covers(e.clientX, e.clientY)) return;
    if (!present || e.button !== 0) return;
    if (chip) {
      bringBack();
      return;
    }
    // Thrown around too often: she won't be caught; she's gone somewhere else.
    if (performance.now() < refuseUntil && !flight) {
      void dodge();
      return;
    }
    // Caught mid-flight, picked up off the floor, a window or an edge: she's in your hand again, turned as she was.
    throwId += 1;
    skid = null;
    edge = null;
    squeeze = null;
    const caught = flight || actor.current === "down" || actor.current === "getup";
    if (flight) swing = Math.max(-70, Math.min(70, norm(flight.a)));
    flight = null;
    if (caught) {
      actor.setState("held");
      sounds.tick();
    }
    perch = null;
    trail.length = 0;
    pointer = [e.clientX, e.clientY];
    drag = { dx: e.clientX - home.x, dy: e.clientY - home.y, moved: caught ? 99 : 0, w: 0 };
    app.canvas.setPointerCapture(e.pointerId);
    app.ticker.maxFPS = 60;
    void invoke("set_grab", { on: true });
  });
  app.canvas.addEventListener("pointerup", (e) => {
    if (!drag) return;
    const click = drag.moved < 5;
    endDrag(e.shiftKey);
    if (app.canvas.hasPointerCapture(e.pointerId)) app.canvas.releasePointerCapture(e.pointerId);
    if (click) {
      wake();
      void actor.play(CLICK_ACTS[Math.floor(Math.random() * CLICK_ACTS.length)]);
      if (panel.open) panel.openInput();
      else panel.say(IDLE_LINES[Math.floor(Math.random() * IDLE_LINES.length)]);
    }
  });
  panel.onChange = () => (lastHull = "");

  // Her music mini player, on her screen while music plays; she hops onto it now and then when a song comes on.
  player = new Player();
  player.onChange = () => (lastHull = "");
  // (Read once the beat listener is set up further on; until then, no beat.)
  player.beat = () => {
    try {
      return beat !== null;
    } catch {
      return false;
    }
  };
  player.onSong = () => {
    if (!present || !player) return;
    player.place(taskbar);
    const free = !perch && !flight && !drag && !ride && !edge && !walk && !possessing && !chip && !awayForGame && actor.current !== "sleep";
    if (free && Math.random() < 0.6) {
      setTimeout(() => {
        const ledge = playerLedge()[0];
        if (ledge && !perch && !drag && !flight) void sitOn(ledge, ledge.x + ledge.w * 0.7);
      }, 1200);
    }
  };
  setInterval(() => {
    if (!player) return;
    if (!present || chip || awayForGame) {
      player.hide();
      return;
    }
    void player.refresh().then(() => player?.place(taskbar));
  }, 2500);

  // Her right-click menu, by her: what you can ask of her without words.
  const menu = document.createElement("div");
  menu.className = "menu";
  menu.hidden = true;
  document.body.appendChild(menu);
  const hideMenu = () => {
    if (menu.hidden) return;
    menu.hidden = true;
    lastHull = "";
  };
  const showMenu = async (x: number, y: number) => {
    // Her reminders coming up: click one to cancel it.
    const due = (await invoke<{ id: number; due: string; text: string }[]>("reminders_list").catch(() => [])) ?? [];
    const when = (d: string) => {
      const t = new Date(d);
      const hm = t.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
      return t.toDateString() === new Date().toDateString() ? hm : `${t.toLocaleDateString([], { weekday: "short" })} ${hm}`;
    };
    const reminders: ([string, () => void] | null)[] = due.slice(0, 5).map((r) => [
      `⏰ ${when(r.due)} ${r.text || "timer"}  ✕`,
      () => void invoke("reminder_cancel", { id: r.id }).then(() => panel.say(r.text ? `Forgot "${r.text}". Your loss.` : "Timer's off.")),
    ]);
    if (reminders.length) reminders.push(null);
    const items: ([string, () => void] | null)[] = [
      ...reminders,
      ["Talk to her", () => void invoke("talk")],
      ["Type to her", () => panel.openInput()],
      null,
      perch
        ? ["Get down", () => void getDown()]
        : [
            "Sit on a window",
            () =>
              void invoke<Perch[]>("perches").then((l) => {
                const p = seats(l)[0];
                if (p) void sitOn(p);
                else panel.say("Nowhere to sit. Every window's top is covered, or right up at the top of the screen.");
              }),
          ],
      ["Possess a window", () => void possessOne(true)],
      ["Mission report", () => void showReport(true)],
      ["Show me around", () => void tour()],
      chip ? ["Bring her back", () => bringBack()] : ["Put her away", () => putAwayNow()],
      null,
      ["Settings…", () => void invoke("open_settings")],
      ["Quit", () => void invoke("quit")],
    ];
    menu.innerHTML = "";
    for (const item of items) {
      if (!item) {
        menu.appendChild(document.createElement("hr"));
        continue;
      }
      const b = document.createElement("button");
      b.textContent = item[0];
      b.onclick = () => {
        hideMenu();
        item[1]();
      };
      menu.appendChild(b);
    }
    menu.hidden = false;
    const w = menu.offsetWidth, h = menu.offsetHeight;
    menu.style.transform = `translate(${Math.min(x, innerWidth - w - 8)}px, ${Math.min(y, innerHeight - h - 8)}px)`;
    lastHull = "";
  };
  app.canvas.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    if (present) void showMenu(e.clientX, e.clientY);
  });
  window.addEventListener("pointerdown", (e) => !menu.contains(e.target as Node) && hideMenu());
  window.addEventListener("keydown", (e) => {
    if (e.key !== "Escape") return;
    hideMenu();
    if (player?.open) player.hide(true, "esc");
  });

  // Put away (Ctrl+Alt+C, her tray menu): she comes apart and goes into her biochip, just her mask in the bottom-left
  // corner; the same again (or a click on it) brings her back, on the screen she left.
  let putAway = false;
  function putAwayNow() {
    putAway = true;
    panel.close("put away");
    perch = null;
    // Folding up small, then apart into her chip.
    void actor.play("fold");
    setTimeout(() => putAway && comeApart(), 380);
    setTimeout(() => {
      if (!putAway) return;
      fx.cue("glitch", headAt, feetAt, scale);
      chip = true;
      actor.setState("float");
      fx.cue("flash", chipAt(), chipAt(), scale);
      lastHull = "";
    }, 830);
  }
  function bringBack() {
    putAway = false;
    chip = false;
    present = true;
    app.ticker.maxFPS = FPS;
    actor.setState("float");
    apart = 1;
    together();
    void actor.play("arrive");
    lastHull = "";
  }

  /** She comes apart: her body and mane fly off her and fade; her mask stays. */
  function comeApart() {
    if (apartTo === 1) return;
    apartTo = 1;
    fx.cue("glitch", headAt, feetAt, scale);
    for (let i = 0; i < 4; i++) fx.shed(headAt, scale * 1.6);
  }
  /** …and snaps back together: the pieces fly in, a flash as they lock. */
  function together() {
    if (apartTo === 0) return;
    apartTo = 0;
    sleptAt = 0;
    setTimeout(() => fx.cue("flash", headAt, feetAt, scale), 380);
  }

  /** Her pieces off her (`k` 0: on, 1: gone), each flying out from her head, turning and shrinking; her mask stays. */
  function scatter(shapes: ReturnType<Rig["posed"]>, k: number): ReturnType<Rig["posed"]> {
    if (k <= 0) return shapes;
    const e = 1 - (1 - k) ** 2;
    const [hx, hy] = headAt;
    const out: ReturnType<Rig["posed"]> = [];
    shapes.forEach((s, i) => {
      if (s.part === "head") {
        out.push(s);
        return;
      }
      if (k > 0.97) return;
      const ring = s.rings[0];
      const cx = ring.reduce((a, p) => a + p[0], 0) / ring.length;
      const cy = ring.reduce((a, p) => a + p[1], 0) / ring.length;
      const len = Math.hypot(cx - hx, cy - hy) || 1;
      const push = (60 + len * 0.9) * e;
      const [ox, oy] = [((cx - hx) / len) * push, ((cy - hy) / len) * push + e * e * 50];
      const shrink = 1 - e;
      const turn = ((i % 5) - 2) * 0.35 * e;
      const [c, sn] = [Math.cos(turn), Math.sin(turn)];
      out.push({
        ...s,
        rings: s.rings.map((r) =>
          r.map(([x, y]) => {
            const [ux, uy] = [(x - cx) * shrink, (y - cy) * shrink];
            return [cx + ox + ux * c - uy * sn, cy + oy + ux * sn + uy * c] as Pt;
          }),
        ),
      });
    });
    return out;
  }

  await listen<string>("capri://menu", ({ payload }) => {
    // Hologram: clicks go straight through her (her panel still takes them); she's see-through, a scan running down
    // her. From her tray icon (a hologram can't be right-clicked), again to turn it off.
    if (payload === "hologram") {
      hologram = !hologram;
      app.stage.alpha = hologram ? 0.62 : 1;
      lastHull = "";
      fx.cue("glitch", headAt, feetAt, scale);
      if (present) panel.say(hologram ? "Hologram. Click right through me." : "Solid again.");
      return;
    }
    if (payload === "away") {
      if (putAway) bringBack();
      else if (present) putAwayNow();
      return;
    }
    if (!present) return;
    if (payload === "ride") {
      if (ride) getOffPointer(false);
      else if (!chip) rideOn();
      return;
    }
    if (payload === "talk" || payload === "type") panel.openInput();
  });
  // Typed and spoken requests both end in `capri://answer`.
  panel.onAsk = async (text) => {
    actor.setState("think");
    const ok = await invoke<boolean>("ask", { text }).catch(() => false);
    if (!ok) panel.answer("Something in me just broke. Try that again?");
  };
  // A file dropped on her: she reads it (a PDF, a Word document, text, a picture) and tells you about it.
  void getCurrentWebview()
    .onDragDropEvent(({ payload }) => {
      if (!present || payload.type === "leave") return;
      const at = "position" in payload ? [payload.position.x / devicePixelRatio, payload.position.y / devicePixelRatio] : null;
      const onHer = !!at && !!bodyBox && at[0] > bodyBox[0] - 40 && at[0] < bodyBox[2] + 40 && at[1] > bodyBox[1] - 40 && at[1] < bodyBox[3] + 40;
      if (payload.type === "over" && onHer && actor.current !== "listen") actor.setState("listen");
      if (payload.type === "drop" && onHer && payload.paths.length) {
        const files = payload.paths.slice(0, 3);
        void panel.onAsk(files.length === 1 ? `Read this file and tell me what's in it: ${files[0]}` : `Read these files and tell me what's in them: ${files.join("; ")}`);
      }
    })
    .catch(() => {});
  await listen<{ line: string; spoken: boolean; via?: string; model?: string | null; steps?: { outcome?: { status?: string } }[] }>("capri://answer", ({ payload }) => {
    if (!present) return;
    if (/satellite/i.test(payload.line)) setTimeout(dream, 600);
    // "Did you mean Stremio?": Yes / No buttons (or just say it: her ears are open for the answer).
    const near = (payload.steps ?? []).some((s) => {
      const data = (s.outcome as { data?: { did_you_mean?: unknown; offer?: unknown } } | undefined)?.data;
      return typeof data?.did_you_mean === "string" || !!data?.offer;
    });
    if (near) {
      void panel.choose(payload.line, "Yes", "No", 30_000).then((pick) => pick !== null && panel.onAsk(pick === 0 ? "yes" : "no"));
      react(payload.steps ?? []);
      return;
    }
    panel.answer(payload.line, payload.model);
    react(payload.steps ?? []);
  });
  await listen<Heard>("capri://heard", ({ payload }) => {
    if (!present) return;
    switch (payload.kind) {
      case "wake":
        wake();
        actor.setState("listen");
        panel.listening();
        break;
      case "request":
        actor.setState("think");
        panel.heard(payload.text ?? "");
        break;
      case "timeout":
        actor.setState(idle());
        panel.nothing();
        break;
      case "follow_up":
        actor.setState("listen");
        panel.awaiting();
        break;
      case "hushed":
        actor.setState(idle());
        panel.close("hushed");
        break;
      case "voice":
        panel.voice(!!payload.on);
        break;
      case "error":
        panel.say(`My ears glitched: ${payload.message}`);
        break;
    }
  });
  panel.onConfirm = (id, yes) => void invoke("confirm", { id, yes });
  panel.onClose = (why) => void invoke("hush", { why });
  // A game: she says so and steps aside; when it's closed she comes back.
  // Only the screen she's on steps aside, and only that one brings her back.
  let awayForGame = false;
  await listen<{ on: boolean; exe: string; key: string; title: string; ask: boolean }>("capri://game", async ({ payload }) => {
    if (payload.on) {
      if (!present) return;
      awayForGame = true;
      if (payload.ask) {
        // First time with this game: quiet (ears off too) or reachable ("Capri" still works). Remembered.
        const pick = await panel.choose(`${payload.title || payload.exe}, huh? Want me quiet while you play, or still listening for my name?`, "Quiet", "Reachable");
        // Remembered by the game's folder (its launcher and the game share one), so she asks once.
        void invoke("game_mode", { exe: payload.key || payload.exe, quiet: pick === 0 });
        panel.say(pick === 0 ? "Quiet it is. I'll be back when it's over." : "I'll keep an ear out. Just say my name.");
        setTimeout(() => void actor.play("vanish").then(() => leave()), 2200);
      } else {
        panel.say("Game time. I'm off your GPU and out of your way. Call me if you need me.");
        setTimeout(() => void actor.play("vanish").then(() => leave()), 3200);
      }
    } else if (awayForGame) {
      awayForGame = false;
      present = true;
      app.ticker.maxFPS = FPS;
      actor.setState("float");
      void actor.play("arrive");
      panel.say("Back. Did you win?");
    }
  });
  await listen<{ kind: string; rms?: number }>("capri://said", ({ payload }) => {
    if (!present) return;
    if (payload.kind === "started") {
      panel.speaking(true);
      if (actor.current === "float") actor.setState("talk");
    } else if (payload.kind === "finished") {
      panel.speaking(false);
      if (actor.current === "talk") actor.setState("float");
    }
    else if (payload.kind === "level") mouth = Math.min(1, (payload.rms ?? 0) * 6);
  });
  // Music with a beat on your speakers: she dances to it, starting on a beat, whenever she's just floating.
  let beat: { bpm: number; since_ms: number } | null = null;
  const danceNow = () => {
    if (!beat || (actor.current !== "float" && actor.current !== "sit") || drag || flight || chip) return;
    const period = 60000 / beat.bpm;
    const bpm = beat.bpm;
    setTimeout(() => {
      if (beat && actor.current === "float") actor.setStateAct("dance", dance(bpm));
      else if (beat && actor.current === "sit" && perch) actor.setStateAct("sit", sitDance(bpm));
    }, (period - (beat.since_ms % period)) % period);
  };
  // Dancing, notes pop off her now and then, on the beat.
  setInterval(() => {
    if (present && beat && !chip && (actor.current === "dance" || (actor.current === "sit" && perch)) && Math.random() < 0.6) fx.note(headAt, scale);
  }, 700);
  // Music already playing when this page started (her start, or a reload): she hears it at once.
  void invoke<{ bpm: number; since_ms: number } | null>("beat_now").then((b) => {
    if (b && !beat) {
      beat = b;
      danceNow();
    }
  });
  await listen<{ bpm: number; since_ms: number } | null>("capri://beat", ({ payload }) => {
    beat = payload;
    if (!present) return;
    if (beat) {
      if (actor.current === "dance") actor.setState("float");
      danceNow();
    } else if (actor.current === "dance") actor.setState("float");
    // Sitting, back to her sit (the same state name, so set the act itself: else the dance went on in silence).
    else if (actor.current === "sit") actor.setStateAct("sit", ACTS.sit);
  });
  // A reminder or a timer coming due: she hops to get your attention and says it (her voice says it too).
  await listen<{ line: string; text: string }>("capri://remind", async ({ payload }) => {
    if (!present) return;
    wake();
    void actor.playAct("alarm", alarmAt(towardYou()));
    sounds.remind();
    // Done, or again in ten minutes.
    const pick = await panel.choose(payload.line, "Done", "10 more min", 60_000);
    if (pick === 0) void invoke("bond_moment", { what: "reminder" });
    if (pick === 1) {
      void invoke("remind_again", { text: payload.text, minutes: 10 });
      panel.say("Ten more minutes. I'm counting.");
    }
  });
  // First run: what she's fetching (her ears, her voice, her brain), as it comes.
  type Setup = { what: string; done: number; line: string };
  let offeredOllama = false;
  const onSetup = async (payload: Setup) => {
    if (!present) return;
    // No Ollama: she offers to install it (once: it can come both as it happens and from `setup_pending`).
    if (payload.what === "ollama_missing") {
      if (offeredOllama) return;
      offeredOllama = true;
      if ((await panel.choose(payload.line, "Install it", "Later", 120_000)) === 0) void invoke("install_ollama");
      return;
    }
    // Download progress updates in place without a sound; the start, the end and a failure are said.
    if (payload.what === "done" || payload.line.startsWith("Couldn't")) panel.say(payload.line, payload.what !== "done");
    else panel.progress(payload.line);
  };
  await listen<Setup>("capri://setup", ({ payload }) => void onSetup(payload));
  // A question that came before this page was listening.
  void invoke<Setup | null>("setup_pending").then((p) => p && onSetup(p));
  await listen<StepEvent>("capri://step", ({ payload }) => {
    if (payload.kind === "finished" && payload.tool === "play_music" && payload.status === "ok") player?.expectMusic();
    // Her hands on your keys or mouse: a crimson glow round every screen while she's at it, "Esc stops her".
    if (payload.tool && HANDS_ON.has(payload.tool)) {
      handsGlow.hidden = false;
      clearTimeout(handsGlowOff);
      if (payload.kind === "finished") handsGlowOff = window.setTimeout(() => (handsGlow.hidden = true), 1500);
    }
    // She looked at a window: a scanline sweeps that window, on whichever screen it's on (it swept her own screen,
    // and with YouTube on the other one she seemed to be looking at the wrong screen).
    if (payload.kind === "finished" && (payload.tool === "see" || payload.tool === "look") && payload.target?.rect) {
      const [l, t, r, b] = payload.target.rect.map((v, i) => v / devicePixelRatio - (i % 2 ? window.screenY : window.screenX));
      const [cl, ct, cr, cb] = [Math.max(8, l), Math.max(8, t), Math.min(innerWidth - 8, r), Math.min(innerHeight - 8, b)];
      if (cr - cl > 40 && cb - ct > 40) fx.scan([cl, ct, cr, cb]);
    }
    if (!present) return;
    panel.step(payload);
    // She points at what she just clicked (screen px to this screen's, from her near shoulder), a wire from her
    // finger to it and a reticle on it, named.
    if (payload.kind === "finished" && payload.point && payload.status === "ok") {
      const css = (x: number, y: number): Pt => [x / devicePixelRatio - window.screenX, y / devicePixelRatio - window.screenY];
      const t = payload.target?.rect;
      const [x, y] = css(payload.point[0], payload.point[1]);
      showClick(x, y, t ? [...css(t[0], t[1]), ...css(t[2], t[3])] : undefined, payload.target?.label);
    }
  });
  /** The steps that use your keys or mouse (or bring a window forward for them). */
  const HANDS_ON = new Set(["click", "click_at", "drag_at", "keys", "type_text", "type_in", "scroll", "tab"]);
  const handsGlow = document.createElement("div");
  handsGlow.className = "hands-glow";
  handsGlow.hidden = true;
  handsGlow.innerHTML = `<span>Capri's using your keys and mouse · Esc stops her</span>`;
  document.body.appendChild(handsGlow);
  let handsGlowOff = 0;

  // A coding AI at work (watch.rs): a ticker over her head with what it's doing, data shed off her as it reads and
  // writes, red sparks when it runs commands or goes online, a hit when one fails, a word when it thinks a long time,
  // and a victory with the summary when it's done.
  const ticker = document.createElement("div");
  ticker.className = "ticker";
  ticker.hidden = true;
  document.body.appendChild(ticker);
  let tickerUntil = 0;
  let thinkingSince = 0;
  let lastCoderFx = 0;
  const tick = (cls: string, head: string, text: string, ms = 9000) => {
    ticker.className = `ticker ${cls}`;
    ticker.innerHTML = "";
    const b = document.createElement("b");
    b.textContent = head;
    ticker.append(b, text);
    ticker.hidden = false;
    tickerUntil = performance.now() + ms;
  };
  type CoderEvent = { kind: string; agent: string; tool?: string; detail?: string; command: boolean; online: boolean; secs: number; summary?: string; window?: [number, number, number, number] | null };
  await listen<CoderEvent>("capri://coder", ({ payload }) => onCoder(payload));
  function onCoder(c: CoderEvent) {
    if (!present || chip || awayForGame) return;
    const who = c.agent === "Claude Code" ? "CLAUDE" : c.agent.toUpperCase();
    const now = performance.now();
    switch (c.kind) {
      case "action": {
        thinkingSince = now;
        tick(c.online ? "online" : "", `${who} ›`, `${(c.tool ?? "").toUpperCase()}${c.detail ? ` · ${c.detail}` : ""}${c.online ? "  ◉ ONLINE" : ""}`);
        if (now - lastCoderFx > 300) {
          lastCoderFx = now;
          fx.bits(headAt, scale);
          // A small jolt through her each time it touches something.
          fx.twitch();
          if (c.command || c.online) {
            fx.redSparks(headAt, scale);
            sounds.bolt(true);
          }
          // Sitting on a window, now and then the lightning runs along it.
          if (perch && !actor.acting && Math.random() < 0.25) void jackIn();
        }
        break;
      }
      case "failed": {
        tick("failed", `${who} ✕`, c.detail ?? "a command failed");
        // Knocked back by it: an impact frame, a heavy glitch, the line that matters.
        flats = [{ color: rig.file.palette.ink, until: now + 42 }, { color: rig.file.palette.crimson, until: now + 84 }];
        fx.cue("glitch", headAt, feetAt, scale);
        void actor.play("flinch");
        sounds.error();
        if (Math.random() < 0.5) panel.say(`${c.agent} hit a wall: ${c.detail ?? "a command failed"}`);
        break;
      }
      case "quiet":
        panel.say(c.secs < 100 ? `${c.agent}'s thinking hard. I can smell the GPU.` : `${c.secs > 150 ? "Still" : "Two minutes of"} thinking. Nothing's stuck, it's just big.`);
        break;
      case "done": {
        ticker.hidden = true;
        thinkingSince = 0;
        wake();
        // A long job (five minutes or more): half a minute later, the uplink.
        if (c.secs >= 300) {
          uplinkDue = true;
          nextGaze = performance.now() + 30_000;
        }
        // Victory: a shockwave off her, her arms up, a cut-in, then what it did.
        fx.impact([home.x, home.y], [0, -1], 0.7, scale);
        void actor.play("victory");
        sounds.done();
        setTimeout(() => cutIn(rig, Math.random() < 0.5 ? "laugh" : "fang", headAt), 250);
        setTimeout(() => panel.say(`${c.agent}'s done. ${c.summary ?? ""}.`), 900);
        break;
      }
      case "answer": {
        // Claude answered while you were on another window (watch.rs checks): its whole answer, wide, beside her.
        if (!c.detail) break;
        wake();
        sounds.blip();
        panel.long(`${who} SAYS`, c.detail);
        break;
      }
      case "needs_you": {
        // Its hook (hooks.rs): it's waiting on you. She comes over to your pointer and wires out to Claude's window.
        tick("online", `${who} ›`, "NEEDS YOU", 20000);
        wake();
        sounds.alert();
        void (async () => {
          const onHere = pointer[0] >= 0 && pointer[0] <= innerWidth && pointer[1] >= 0 && pointer[1] <= innerHeight;
          if (onHere && !drag && !flight && !perch && Math.abs(pointer[0] - home.x) > HEIGHT * 0.6) {
            fx.cue("glitch", headAt, feetAt, scale);
            await actor.play("vanish");
            home.x = clamp(pointer[0] + (pointer[0] > innerWidth / 2 ? -1 : 1) * HEIGHT * 0.5, HEIGHT * 0.3, innerWidth - HEIGHT * 0.3);
            await actor.play("arrive");
          }
          if (c.window) {
            const [l, t, r, b] = [c.window[0] - screenX, c.window[1] - screenY, c.window[2] - screenX, c.window[3] - screenY];
            showClick((l + r) / 2, t + 40, [l, t, r, b], "CLAUDE");
          }
          panel.say(`Claude needs you: ${c.detail ?? "it's waiting on you"}`, true);
        })();
        break;
      }
      case "gone":
        ticker.hidden = true;
        thinkingSince = 0;
        break;
    }
  }
  // The ticker over her head: placed each frame; while the AI thinks (a few seconds since its last action) it turns
  // violet and counts.
  setInterval(() => {
    if (ticker.hidden) return;
    const now = performance.now();
    if (now > tickerUntil && !thinkingSince) {
      ticker.hidden = true;
      return;
    }
    if (thinkingSince && now - thinkingSince > 4000 && !ticker.classList.contains("failed")) {
      tick("thinking", "CLAUDE ›", `THINKING… ${Math.round((now - thinkingSince) / 1000)} s`, 600_000);
    }
  }, 1000);

  // What goes with her to another screen: the ticker over her head (and how long it's been thinking) and her panel.
  type Carry = { ticker?: { cls: string; head: string; text: string; ms: number }; thinkingFor?: number; panel?: PanelState; muted?: boolean };
  function carryOut(): Carry {
    const c: Carry = { muted: !mutedBadge.hidden };
    if (!ticker.hidden) {
      const head = ticker.querySelector("b")?.textContent ?? "";
      c.ticker = { cls: ticker.className.replace(/^ticker ?/, ""), head, text: (ticker.textContent ?? "").slice(head.length), ms: Math.max(1000, tickerUntil - performance.now()) };
    }
    if (thinkingSince) c.thinkingFor = performance.now() - thinkingSince;
    const p = panel.snapshot();
    if (p) c.panel = p;
    return c;
  }
  function carryIn(c?: Carry) {
    if (!c) return;
    if (c.ticker) tick(c.ticker.cls, c.ticker.head, c.ticker.text, c.ticker.ms);
    if (c.thinkingFor !== undefined) thinkingSince = performance.now() - c.thinkingFor;
    if (c.panel) panel.restore(c.panel);
    mutedBadge.hidden = !c.muted;
  }

  // Her bond reached a new stage (bond.rs): a moment.
  // How close you two are (0..4): from 3 she comes over to you now and then.
  let bondStage = 0;
  void invoke<{ stage: number }>("bond").then((b) => (bondStage = b?.stage ?? 0)).catch(() => undefined);
  await listen<{ stage: number; name: string }>("capri://bond", ({ payload }) => {
    bondStage = Math.max(bondStage, payload.stage);
    if (!present || awayForGame) return;
    const lines = [
      "",
      "Huh. You keep coming back. Fine. You're not a stranger any more.",
      "You know what? You're my choom now. Don't make it weird.",
      "Partner in crime. Officially. All of my minds voted. It was unanimous. Mostly.",
      "You're family now. All of me says so. You're stuck with us.",
    ];
    if (chip) bringBack();
    wake();
    fx.cue("sparkle", headAt, feetAt, scale);
    mindsAt = performance.now();
    void actor.play("victory");
    setTimeout(() => cutIn(rig, payload.stage >= 3 ? "laugh" : "fang", headAt, 1200), 300);
    setTimeout(() => panel.say(lines[payload.stage] ?? "", true), 1000);
  });

  // Your mic muted for her: she says so, and a crossed-out mic hangs by her head until it's back.
  const mutedBadge = document.createElement("div");
  mutedBadge.className = "tag muted";
  mutedBadge.textContent = "MIC OFF";
  mutedBadge.hidden = true;
  document.body.appendChild(mutedBadge);
  await listen<boolean>("capri://muted", ({ payload }) => {
    if (!present) return;
    const was = !mutedBadge.hidden;
    mutedBadge.hidden = !payload;
    if (payload && was) panel.say("Your mic's muted for me. Ctrl+Alt+N and I'm back.");
    else panel.say(payload ? "Mic off. I can't hear a thing. Ctrl+Alt+N when you want me." : "Mic's back. I'm listening.");
    lastHull = "";
  });
  // Her feeling your PC (feel.rs): hot, the charger, the battery, the network, a USB stick, disk and memory.
  let hotUntil = 0;
  await listen<{ kind: string; line: string }>("capri://feel", ({ payload }) => onFeel(payload.kind, payload.line));
  function onFeel(kind: string, line: string) {
    if (!present || awayForGame) return;
    if (chip) {
      panel.say(line, kind === "usb_in" || kind.endsWith("_low"));
      return;
    }
    wake();
    // Her checking in on you (care.rs): said with a little act to go with it.
    if (kind.startsWith("care_")) {
      panel.say(line);
      const act: Record<string, string> = { care_water: "wave", care_stretch: "stretch", care_eyes: "look", care_food: "huff", care_bed: "drained", care_game_break: "stretch" };
      void actor.play((act[kind] ?? "wave") as never);
      return;
    }
    switch (kind) {
      case "hot":
        // Fanning herself, heat off her mane, for as long as it lasts (a minute at most).
        hotUntil = performance.now() + 60_000;
        void actor.play("huff");
        break;
      case "cool":
        hotUntil = 0;
        void actor.play("stretch");
        break;
      case "charger_in":
        fx.cue("sparkle", headAt, feetAt, scale);
        void actor.play("hop");
        break;
      case "charger_out":
      case "offline":
        fx.cue("glitch", headAt, feetAt, scale);
        void actor.play("flinch");
        break;
      case "battery_low":
        // She sags, a flicker through her.
        fx.cue("glitch", headAt, feetAt, scale);
        void actor.play("drained");
        break;
      case "online":
        void actor.play("nod");
        break;
      case "rival":
        // Jealous: onto that window (the one in front), a glare and a huff; sometimes she takes it over.
        void invoke<Perch[]>("perches").then(async (list) => {
          const front = seats(list)[0];
          if (front && !perch) await sitOn(front);
          void actor.playAct("glare", glareAt(towardYou()));
          panel.say(line, true);
          if (Math.random() < 0.35) setTimeout(() => void possessOne(), 6000);
        });
        return;
      case "usb_in":
        // She dives into it by the clock and comes back with what she found.
        void actor.play("vanish").then(() => {
          leaveFor(1600).then(() => panel.say(line, true));
        });
        return;
      default:
        void actor.play("look");
    }
    panel.say(line, kind.endsWith("_low"));
  }

  // Running hot, heat comes off her mane, whatever else she's doing.
  setInterval(() => present && !chip && performance.now() < hotUntil && fx.heat(headAt, scale), 250);

  /** Gone for `ms` (into a USB stick), then back with a glitch. */
  async function leaveFor(ms: number) {
    const was = { x: home.x, y: home.y };
    home.y = innerHeight + HEIGHT;
    await new Promise((r) => setTimeout(r, ms));
    home.x = was.x;
    home.y = was.y;
    await actor.play("arrive");
  }
  await listen<{ id: number; what: string }>("capri://confirm", ({ payload }) => present && panel.confirm(payload.id, payload.what));
  // Answered by voice or typing: her buttons go.
  await listen<number>("capri://confirmed", () => panel.confirmAnswered());

  app.ticker.add(() => {
    if (!present) return;
    frame++;
    const tnow = performance.now();
    if (flight) updateFly(tnow);
    if (skid) updateSkid(tnow);
    if (drag) holdOn(tnow);
    // Gone to another screen this very frame (flown over): nothing more drawn here, or her last drawing stayed on
    // this screen too, a second her.
    if (!present) {
      body.clear();
      shadow.clear();
      ghostG.clear();
      return;
    }
    let d = frozen ?? actor.tick(app.ticker.deltaMS, () => fx.step());
    const [lift, zoom, introDrawing] = intro ? introAt(intro.hold ?? performance.now() - intro.t0) : [0, 1, null];
    if (introDrawing) d = introDrawing;
    // The screen shakes (her whole layer), dying away.
    quake *= 0.86;
    app.stage.position.set(quake > 0.5 ? (Math.random() - 0.5) * quake : 0, quake > 0.5 ? (Math.random() - 0.5) * quake : 0);
    // Whatever she does sitting (a move, listening, thinking), she stays sat: legs over the edge, an idle hand on it.
    if (actor.acting !== "vanish" && ((actor.current === "sit" && actor.acting) || (actor.current !== "sit" && ((perch && !perch.stand) || ride)))) d = seated(d);
    // Riding your pointer: sitting on its tip.
    if (ride) {
      home.x = ride.x - HEIGHT * 0.02;
      home.y = ride.y - HEIGHT * 0.04;
    }
    // Walking: along until she's there.
    if (walk) {
      const step = walk.dir * 70 * (app.ticker.deltaMS / 1000) * (HEIGHT / 380);
      home.x += step;
      // Walking along a window's top edge: where she is along it goes with her.
      if (perch?.stand) perch.dx += step;
      if ((walk.dir > 0 && home.x >= walk.to) || (walk.dir < 0 && home.x <= walk.to) || drag || flight || (perch && !perch.stand)) {
        walk = null;
        if (actor.current === "walk") {
          actor.setState("float");
          // At the end of a stroll on a window: sometimes she sits down there.
          if (perch?.stand && Math.random() < 0.5) {
            perch.stand = false;
            home.y += HEIGHT * 0.46 + 1;
            actor.setState("sit");
          }
        }
      }
    }
    // At an edge: leaning in (or upside down), ducking out of sight from your pointer, back to her spot in time.
    let [edgeDx, edgeDy, edgeRoot, edgeFlip] = [0, 0, 0, false];
    if (edge) {
      const now = performance.now();
      if (Math.hypot(pointer[0] - headAt[0], pointer[1] - headAt[1]) < 260) edge.duckUntil = now + 2500;
      const want = now < edge.duckUntil ? 1 : 0;
      edge.hidden += Math.sign(want - edge.hidden) * Math.min(Math.abs(want - edge.hidden), app.ticker.deltaMS / 220);
      if (edge.kind === "peek") {
        edgeRoot = -16 + Math.sin(now / 900) * 3;
        edgeDx = edge.side * edge.hidden * HEIGHT * 0.5;
        edgeFlip = edge.side < 0;
      } else {
        edgeRoot = 180 + Math.sin(now / 800) * 6;
        edgeDy = -edge.hidden * HEIGHT * 0.6;
      }
      if (now > edge.until) void leaveEdge();
    }
    // Coming apart or back together over about ten drawings.
    apart += Math.sign(apartTo - apart) * Math.min(Math.abs(apartTo - apart), app.ticker.deltaMS / 420);
    // Her eyes in her biochip: shut, open when she's needed (talking, listening, a panel up).
    const needed = panel.open || mouth > 0.05 || actor.current === "listen" || actor.current === "think";
    const peeking = performance.now() < peekUntil;
    face.expression = chip
      ? needed ? "calm" : peeking ? "peek" : "closed"
      : d.face ?? (actor.current === "sleep" ? (apart > 0.5 && peeking ? "peek" : "closed") : "calm");
    if (!drag) swing *= 0.85;
    // She cocks her head toward your pointer when it comes close (not while she's mid-move).
    const headY = home.y - HEIGHT * 0.2;
    const near = !actor.acting && Math.hypot(pointer[0] - home.x, pointer[1] - headY) < 320;
    const look = near ? Math.max(-9, Math.min(9, (pointer[0] - home.x) / 18)) : 0;
    const base: Pose = { ...rig.restPose(home.x + fx.offset + edgeDx, home.y + edgeDy, scale), flip: edgeFlip || (walk?.dir ?? 0) > 0, boil: Math.floor(frame / 4) % 3, boilAmount: 0.45 };
    // Thrown, she's turned however she's tumbling; skidding, she leans back against it.
    const spin = flight ? flight.a : skid ? Math.max(-16, Math.min(16, -skid.v * 0.012)) : 0;
    // In her biochip: just her mask, bobbing in the corner, nearly twice as big as her head.
    const cs = scale * 1.8;
    const [chx, chy] = chipAt();
    const pose = chip
      ? { ...rig.restPose(chx - 28 * cs, chy + 87 * cs + Math.sin(performance.now() / 900) * 4, cs), boil: base.boil, boilAmount: 0.45 }
      : zoom !== 1 || lift
        ? // Her face up against the glass: bigger around her head, which stays put.
          compose({ ...base, x: base.x + 28 * scale * (1 - zoom), y: home.y + lift + 87 * scale * (zoom - 1), scale: scale * zoom }, d, {})
        : compose(base, d, { head: look, neck: look * 0.3, root: swing + spin + edgeRoot });
    // Held, her mane trails the way she's swung, blowing up and smearing when it's fast; thrown, the same along her
    // flight, and for its first drawings she's one long streak along it, her other minds pulled out with her.
    let smear = 0;
    let smearDir: Pt = [1, 0];
    const motion: Pt | null = flight?.phase === "air" ? flight.v : drag && actor.current === "held" ? pointerSpeed() : null;
    const speed = motion ? Math.hypot(motion[0], motion[1]) : 0;
    if (motion && speed > 80) {
      const k = Math.min(1, speed / (flight ? 2400 : 2200));
      smearDir = [motion[0] / speed, motion[1] / speed];
      pose.sway += -smearDir[0] * 9 * k;
      pose.flare = flight ? Math.max(pose.flare, 0.28 + 0.4 * k) : pose.flare + 0.25 * k;
      if (flight || k > 0.35) smear = (flight ? 24 : 20) * k;
      if (flight && flight.release > 0) {
        pose.squeeze = { u: smearDir, along: 1.32, across: 0.76, at: [pose.x, pose.y] };
        pose.flare = 0.7;
        smear = 34;
      }
    }
    const flatten = squeezeNow();
    if (flatten) pose.squeeze = flatten;
    const shapes = scatter(rig.posed(pose), chip ? 1 : apart);
    headAt = rig.onBone("head", [[121, 118]])[0];
    feetAt = rig.onBone("legL_foot", [[104, 400]])[0];
    // Her other minds: her green and red copies pull away from her now and then, hang there, and snap back in.
    let minds = 0;
    if (mindsAt) {
      const f = (performance.now() - mindsAt) / 41.7;
      minds = f < 6 ? (f / 6) * 34 : f < 24 ? 34 + Math.sin(f / 2) * 3 : f < 26 ? 34 * (1 - (f - 24) / 2) : 0;
      if (f >= 26) mindsAt = 0;
    }
    // A twitch: her minds split from her for two drawings.
    if (performance.now() - twitchedAt < 170) minds = Math.max(minds, 9);
    // Her reach from her hips as she's turned now (what she bumps into the edges with).
    if (flight || skid) {
      let [x0, x1, y0, y1] = [Infinity, -Infinity, Infinity, -Infinity];
      for (const s of shapes)
        for (const [x, y] of s.rings[0]) {
          if (x < x0) x0 = x;
          if (x > x1) x1 = x;
          if (y < y0) y0 = y;
          if (y > y1) y1 = y;
        }
      reach = { minX: x0 - pose.x, maxX: x1 - pose.x, minY: y0 - pose.y, maxY: y1 - pose.y };
    }
    // Glitched out: nothing of her, until she's brought back (her effects still play out).
    if (actor.gone) {
      body.clear();
      shadow.clear();
      ghostG.clear();
      fx.draw(fxG);
      sendHull([]);
      return;
    }
    const echo = Math.max((d.echo ?? 0) * scale, minds);
    ghosts(ghostG, shapes, Math.max(echo, smear * scale), smear * scale > echo ? smearDir : [1, 0]);
    const flat = flats.find((f) => performance.now() < f.until);
    const palette = flat ? Object.fromEntries(Object.keys(rig.file.palette).map((k) => [k, flat.color])) : rig.file.palette;
    // Her sparkle: dim asleep, a flash of it when she's patted, fading back over a second.
    const patted = Math.max(0, 1 - (performance.now() - sparkledAt) / 1200);
    const sparkle = (actor.current === "sleep" ? 0.35 : 1) + 0.8 * patted;
    draw(body, shadow, shapes, palette, fx.shadowOffset, sparkle, Math.floor(frame / 2));
    fx.draw(fxG);
    const [lx, ly] = chip ? [chx, chy] : [home.x, headY];
    const close = chip ? Math.hypot(pointer[0] - chx, pointer[1] - chy) < 320 : near;
    const lookAt: Pt = close ? [Math.max(-1, Math.min(1, (pointer[0] - lx) / 200)), Math.max(-1, Math.min(1, (pointer[1] - ly) / 200))] : [0, 0];
    face.draw(body, rig, performance.now(), mouth, lookAt);
    portrait.talk = mouth;
    if (!ticker.hidden) ticker.style.transform = `translate(${Math.max(8, Math.min(innerWidth - ticker.offsetWidth - 8, headAt[0] - ticker.offsetWidth / 2))}px, ${Math.max(8, headAt[1] - 150 * scale - 26)}px)`;
    if (!mutedBadge.hidden) mutedBadge.style.transform = `translate(${(chip ? chx : headAt[0]) + 30}px, ${(chip ? chy : headAt[1]) - 34}px)`;
    if (chip) panel.place(chx + 30, chy - 30, true);
    else if (edge) panel.place(edge.side > 0 || edge.kind === "hang" ? headAt[0] - HEIGHT * 0.1 : headAt[0] + HEIGHT * 0.1, headAt[1], edge.kind === "peek" && edge.side < 0);
    else panel.place(home.x - HEIGHT * 0.15, headY);
    sendHull(shapes);
  });

  // Her outline for click-through: a hull per part, plus the panel when it's up. Only sent when it changed.
  function sendHull(shapes: ReturnType<Rig["posed"]>) {
    if (ride) {
      if (lastHull !== "riding") void invoke("set_hull", { args: { polygons: [] } });
      lastHull = "riding";
      return;
    }
    const byPart = new Map<string, Pt[]>();
    for (const s of shapes) {
      const pts = byPart.get(s.part) ?? [];
      pts.push(...s.rings[0]);
      byPart.set(s.part, pts);
    }
    const polygons = hologram ? [] : [...byPart.values()].map((pts) => hull(pts).map(([x, y]) => [Math.round(x), Math.round(y)]));
    const all = polygons.flat();
    bodyBox = all.length ? [Math.min(...all.map((p) => p[0])), Math.min(...all.map((p) => p[1])), Math.max(...all.map((p) => p[0])), Math.max(...all.map((p) => p[1]))] : null;
    const rect = panel.rect();
    if (rect) polygons.push(rect.map(([x, y]) => [Math.round(x), Math.round(y)]));
    const deck = player?.rect();
    if (deck) polygons.push([[deck.left, deck.top], [deck.right, deck.top], [deck.right, deck.bottom], [deck.left, deck.bottom]].map(([x, y]) => [Math.round(x), Math.round(y)]));
    const card = document.querySelector(".report");
    if (card) {
      const r = card.getBoundingClientRect();
      polygons.push([[r.left, r.top], [r.right, r.top], [r.right, r.bottom], [r.left, r.bottom]].map(([x, y]) => [Math.round(x), Math.round(y)]));
    }
    if (!menu.hidden) {
      const r = menu.getBoundingClientRect();
      polygons.push([[r.left, r.top], [r.right, r.top], [r.right, r.bottom], [r.left, r.bottom]].map(([x, y]) => [Math.round(x), Math.round(y)]));
    }
    const key = JSON.stringify(polygons);
    if (key === lastHull) return;
    lastHull = key;
    void invoke("set_hull", { args: { polygons } });
  }

  /** How fast the pointer holding her is going right now (px/s): nothing if it had stopped. */
  function pointerSpeed(): Pt {
    const now = performance.now();
    const pts = trail.filter((p) => now - p.t < 80);
    if (pts.length < 2 || now - pts[pts.length - 1].t > 60) return [0, 0];
    const [a, b] = [pts[0], pts[pts.length - 1]];
    const dt = Math.max(0.012, (b.t - a.t) / 1000);
    return [(b.x - a.x) / dt, (b.y - a.y) / dt];
  }

  /** An angle from -180 to 180. */
  const norm = (a: number) => ((((a + 180) % 360) + 360) % 360) - 180;
  const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
  const lerp = (a: number, b: number, k: number) => a + (b - a) * k;
  /** The line her lowest point rests on: the top of the taskbar. */
  const ground = () => floor() + HEIGHT * 0.46;

  /** Held: hanging from the point you hold her by, swinging the other way when you move (a spring, so she sways
   *  back and forth once you stop). */
  function holdOn(now: number) {
    if (!drag) return;
    const dt = Math.min(0.05, app.ticker.deltaMS / 1000);
    if (actor.current === "held") {
      const target = clamp(pointerSpeed()[0] * 0.014, -40, 40);
      drag.w += ((target - swing) * 95 - drag.w * 8.5) * dt;
      swing += drag.w * dt;
    }
    // The point you hold stays under the pointer, however she swings around it.
    const r = (swing * Math.PI) / 180;
    const [c, s] = [Math.cos(r), Math.sin(r)];
    // Held, she stays on her screen; thrown, she can fly on into the next one.
    home.x = clamp(pointer[0] - (c * drag.dx - s * drag.dy), 0, innerWidth);
    home.y = clamp(pointer[1] - (s * drag.dx + c * drag.dy), 0, innerHeight);
    void now;
  }

  /** Thrown (px/s): she flies off with the pointer's speed, head first (feet first held by her legs), for two
   *  drawings one long streak along the throw, then tumbling, spinning faster the harder she went sideways. */
  function throwHer(vx: number, vy: number, feetFirst = false, byYou = true) {
    const speed = Math.hypot(vx, vy);
    log("thrown", { speed: Math.round(speed), feetFirst, byYou });
    if (speed > 4200) [vx, vy] = [(vx * 4200) / speed, (vy * 4200) / speed];
    throwId += 1;
    const along = (Math.atan2(vx, -vy) * 180) / Math.PI + (feetFirst ? 180 : 0);
    const w = clamp(vx * 0.32, -900, 900) + (Math.random() - 0.5) * 160 + (Math.abs(vx) < 300 ? (Math.random() < 0.5 ? -1 : 1) * (220 + Math.random() * 200) : 0);
    const f: Flight = { c: [home.x, home.y], v: [vx, vy], a: along, w, release: 2, frame: 0, phase: "air", lie: 0, hits: 0, hard: 0, freezeUntil: 0, last: performance.now(), ledges: [], id: throwId };
    flight = f;
    swing = 0;
    perch = null;
    actor.setState("fly");
    app.ticker.maxFPS = 60;
    sounds.whoosh(Math.min(1, Math.hypot(vx, vy) / 4200));
    fx.burst([home.x, home.y], scale, 8, 0.45);
    fx.cue("glitch", headAt, feetAt, scale);
    void invoke<Perch[]>("perches").then((l) => (f.ledges = l ?? []));
    lookAround();
    if (!byYou) return;
    const now = performance.now();
    throws.push(now);
    while (throws.length && now - throws[0] > 120_000) throws.shift();
    if (throws.length >= 3) {
      refuseUntil = now + 20_000;
      throws.length = 0;
      void invoke("set_mood", { mood: "mad at the user", why: "they threw you around the screen three times in a row", minutes: 10 });
    }
  }

  /** Off something (her window went): falling straight down, upright, onto whatever's under her (or hopping off
   *  it, `v` px/s). */
  function fallFrom(ledges: Perch[], v: Pt = [0, 0]) {
    throwId += 1;
    flight = { c: [home.x, home.y], v, a: 0, w: 0, release: 0, frame: 0, phase: "air", lie: 0, hits: 0, hard: 0, freezeUntil: 0, last: performance.now(), ledges, id: throwId };
    actor.setState("fly");
    app.ticker.maxFPS = 60;
  }

  /** One drawing of her flight: gravity, a little air, her spin; then whatever she runs into. */
  function updateFly(now: number) {
    const f = flight;
    if (!f) return;
    const dt = Math.min(0.05, Math.max(0.001, (now - f.last) / 1000));
    f.last = now;
    // The hit stop: stuck to what she hit for a few drawings.
    if (now < f.freezeUntil) return;
    f.frame += 1;
    if (f.release > 0) f.release -= 1;
    const W = innerWidth;
    const g = ground();
    const before = reach;
    const prevY = f.c[1];
    if (f.phase === "air") {
      f.v[1] += 2500 * dt;
      f.v[0] *= 0.8 ** dt;
      f.a += f.w * dt;
      f.c[0] += f.v[0] * dt;
      f.c[1] += f.v[1] * dt;
    } else {
      // Down on the floor: sliding on her side, slowing down, settling flat.
      f.a += clamp(f.lie - f.a, -540 * dt, 540 * dt);
      const brake = 2800 * dt;
      f.v[0] = Math.abs(f.v[0]) <= brake ? 0 : f.v[0] - Math.sign(f.v[0]) * brake;
      f.c[0] += f.v[0] * dt;
    }
    const r = reach;
    // The sides of the screen: she hits them with the tips of her mane.
    for (const side of [-1, 1]) {
      const out = side < 0 ? f.c[0] + r.minX < 0 : f.c[0] + r.maxX > W;
      if (!out || Math.sign(f.v[0]) !== side) continue;
      // Another screen there: no wall. Once her middle is past the edge, that screen takes her.
      const past = side < 0 ? -2 : W + 2;
      const next = others.find((o) => past >= o.x && past < o.x + o.w && f.c[1] >= o.y && f.c[1] < o.y + o.h);
      if (next) {
        if (side < 0 ? f.c[0] < 0 : f.c[0] > W) return flyOn(f, next);
        continue;
      }
      f.c[0] = side < 0 ? -r.minX : W - r.maxX;
      impact([-side, 0], Math.abs(f.v[0]), [side < 0 ? 0 : W, f.c[1]]);
      f.v[0] = -f.v[0] * 0.42;
      f.v[1] *= 0.85;
      f.w = -f.w * 0.55 + (Math.random() - 0.5) * 180;
    }
    if (f.phase === "air") {
      // The top of the screen (or the screen above it).
      const above = others.find((o) => f.c[0] >= o.x && f.c[0] < o.x + o.w && -2 >= o.y && -2 < o.y + o.h);
      if (above && f.c[1] < 0 && f.v[1] < 0) return flyOn(f, above);
      if (!above && f.c[1] + r.minY < 0 && f.v[1] < 0) {
        f.c[1] = -r.minY;
        impact([0, 1], -f.v[1], [f.c[0], 0]);
        f.v[1] = -f.v[1] * 0.35;
        f.w *= 0.7;
      }
      // A window's top edge, on her way down.
      if (f.v[1] > 0) {
        const win = f.ledges.find((q) => prevY + before.maxY <= q.y + 2 && f.c[1] + r.maxY >= q.y && f.c[0] > q.vx + 30 && f.c[0] < q.vx + q.vw - 30);
        if (win && onLedge(f, win)) return;
      }
      if (f.c[1] + r.maxY >= g) {
        f.c[1] = g - r.maxY;
        if (onFloor(f)) return;
      }
      // Her trail: shards of her mane left hanging behind her.
      if (f.frame % 3 === 0 && Math.hypot(f.v[0], f.v[1]) > 900) fx.shed(headAt, scale);
    } else {
      f.c[1] = g - r.maxY;
      if (Math.abs(f.v[0]) > 140 && f.frame % 2 === 0) fx.kick([f.c[0], g], -Math.sign(f.v[0]), scale, 2);
      if (!f.v[0] && Math.abs(f.lie - f.a) < 0.5) {
        void knockedDown(f);
        return;
      }
    }
    home.x = f.c[0];
    home.y = f.c[1];
  }

  /** Out of this screen and on into `next`, still flying: its overlay takes her where she is, as fast and spinning. */
  function flyOn(f: Flight, next: Around) {
    const carry = carryOut();
    flight = null;
    leave(true);
    void invoke("fly_to", { args: { to: next.label, x: f.c[0], y: f.c[1], vx: f.v[0], vy: f.v[1], a: f.a, w: f.w, carry } });
  }

  /** She hits something at `speed` px/s at screen point `at`, `n` pointing away from it: the harder, the longer she
   *  sticks to it (the hit stop), the flatter she gets, the bigger the ring and the cracks; impact frames on the hard
   *  ones; a window she hits shakes under her. */
  function impact(n: Pt, speed: number, at: Pt, win?: Perch) {
    const f = flight;
    if (!f) return;
    const hard = clamp((speed - 250) / 2300, 0, 1);
    const now = performance.now();
    f.hits += 1;
    f.hard = Math.max(f.hard, hard);
    const hold = Math.round(lerp(1, 4, hard));
    if (hard > 0.12) f.freezeUntil = now + hold * 41.7;
    squeeze = { n, k: lerp(0.84, 0.56, hard), t: now, hold, at };
    sounds.crash(hard);
    fx.impact(at, [-n[0], -n[1]], hard, scale);
    if (win) void invoke("shake_window", { id: win.id, power: Math.max(0.3, hard) });
    if (hard > 0.3) flats = hard > 0.65 ? [{ color: "#0b0614", until: now + 42 }, { color: "#f60655", until: now + 84 }] : [{ color: "#0b0614", until: now + 42 }];
    if (hard > 0.3) quake = 6 + hard * 14;
    // The first really hard one of a throw: her round-eyed close-up.
    if (hard > 0.85 && f.hits === 1) cutIn(rig, "round", headAt, 700);
  }

  /** Flattened against what she hit: held for the hit stop, then back through an overshoot, a drawing each. */
  function squeezeNow(): Pose["squeeze"] | null {
    const q = squeeze;
    if (!q) return null;
    const frame = Math.floor((performance.now() - q.t) / 41.7);
    const k = frame < q.hold ? q.k : [1 + (1 - q.k) * 0.5, 0.95, 1.02][frame - q.hold];
    if (k === undefined) {
      squeeze = null;
      return null;
    }
    return { u: q.n, along: k, across: 1 + (1 - k) * 0.6, at: q.at };
  }

  /** Down on the floor: on her feet if she's about upright, a bounce if she comes down hard, else she slides along
   *  it on her side. True when she's done flying. */
  function onFloor(f: Flight): boolean {
    const a = norm(f.a);
    if (Math.abs(a) < 30) {
      landOnFeet(f);
      return true;
    }
    if (f.v[1] > 420) {
      impact([0, -1], f.v[1], [f.c[0], ground()]);
      f.v[1] = -f.v[1] * 0.36;
      f.v[0] *= 0.72;
      f.w = f.w * 0.5 - f.v[0] * 0.12;
      return false;
    }
    f.phase = "slide";
    f.v[1] = 0;
    // The nearest way to lie on her side, without spinning round.
    f.lie = f.a + ((a >= 0 ? 90 : -90) - a);
    sounds.land();
    return false;
  }

  /** Coming down on a window's top edge: she catches it and sits if she's about upright or slow, else she bounces
   *  off it and it shakes. True when she's done flying. */
  function onLedge(f: Flight, win: Perch): boolean {
    if (Math.abs(norm(f.a)) < 45 || f.v[1] < 600) {
      catchLedge(f, win);
      return true;
    }
    f.c[1] = win.y - reach.maxY;
    impact([0, -1], f.v[1], [f.c[0], win.y], win);
    f.v[1] = -f.v[1] * 0.38;
    f.v[0] *= 0.8;
    f.w = f.w * 0.6 + (Math.random() - 0.5) * 240;
    return false;
  }

  function catchLedge(f: Flight, win: Perch) {
    flight = null;
    // On the uncovered stretch only (same as sitOn): full win.w used to land her under another window.
    const [from, to] = [win.vx - win.x + 50, win.vx - win.x + win.vw - 50];
    const dx = clamp(f.c[0] - win.x, Math.min(from, to), Math.max(from, to));
    home.x = win.x + dx;
    home.y = win.y;
    perch = { id: win.id, dx, since: performance.now() };
    actor.setState("sit");
    app.ticker.maxFPS = FPS;
    sounds.land();
    fx.burst([home.x, win.y], scale, 8, 0.4);
    squeeze = { n: [0, -1], k: 0.8, t: performance.now(), hold: 1, at: [home.x, win.y] };
    void invoke("shake_window", { id: win.id, power: 0.25 });
    f.caught = true;
    landed(f);
  }

  /** Down on her feet: the landing, then sliding on along the floor if she was going sideways. */
  function landOnFeet(f: Flight) {
    flight = null;
    home.x = clamp(f.c[0], HEIGHT * 0.2, innerWidth - HEIGHT * 0.2);
    home.y = floor();
    const hard = clamp((f.v[1] - 250) / 2300, 0, 1);
    skid = Math.abs(f.v[0]) > 150 ? { v: f.v[0] * 0.8, last: performance.now(), scraped: 0 } : null;
    if (hard > 0.3) sounds.crash(hard * 0.7);
    else sounds.land();
    fx.burst([home.x, ground() - 10 * scale], scale, 10 + Math.round(hard * 10), 0.5 + hard * 0.5);
    fx.cue("dust", headAt, feetAt, scale);
    actor.setState("float");
    const id = f.id;
    void actor.play("land").then(async () => {
      while (skid && id === throwId) await new Promise((r) => setTimeout(r, 50));
      if (id === throwId) landed(f);
    });
  }

  /** Sliding on along the floor on her feet after a landing, sparks under them. */
  function updateSkid(now: number) {
    const k = skid;
    if (!k) return;
    const dt = Math.min(0.05, (now - k.last) / 1000);
    k.last = now;
    const brake = 3200 * dt;
    k.v = Math.abs(k.v) <= brake ? 0 : k.v - Math.sign(k.v) * brake;
    home.x = clamp(home.x + k.v * dt, HEIGHT * 0.2, innerWidth - HEIGHT * 0.2);
    if (Math.abs(k.v) > 120) fx.kick([home.x, ground()], -Math.sign(k.v), scale, 2);
    if (now - k.scraped > 110 && Math.abs(k.v) > 120) {
      k.scraped = now;
      sounds.scrape(Math.min(1, Math.abs(k.v) / 2000));
    }
    if (!k.v) {
      skid = null;
      app.ticker.maxFPS = FPS;
    }
  }

  /** Knocked down: on her side on the floor, seeing stars, a twitch now and then, until she springs back up and
   *  shakes it off. Just tossed onto it (nothing hit hard), she's up in a moment, no stars. */
  async function knockedDown(f: Flight) {
    const side = norm(f.a) >= 0 ? 1 : -1;
    const soft = f.hard < 0.15;
    const id = f.id;
    flight = null;
    home.x = f.c[0];
    home.y = floor();
    app.ticker.maxFPS = FPS;
    actor.setStateAct("down", lyingDown(side));
    if (!soft) fx.dizzy(() => headAt, 58);
    await new Promise((r) => setTimeout(r, soft ? 400 + Math.random() * 300 : 1500 + Math.random() * 600));
    if (id !== throwId || actor.current !== "down") return; // picked up meanwhile
    sounds.rise();
    await actor.playAct("getup", gettingUp(side));
    if (id !== throwId) return;
    actor.setState("float");
    sounds.land();
    await actor.play("shake");
    if (id === throwId) landed(f);
  }

  /** How the throw went, in a word from her (not every time; the third throw she's too mad for words). */
  function landed(f: Flight) {
    if (performance.now() < refuseUntil || Math.random() < 0.4) return;
    const pick = (l: string[]) => l[Math.floor(Math.random() * l.length)];
    panel.say(
      f.caught
        ? pick(["Stuck the landing. Obviously.", "Caught it. You're welcome.", "Ten out of ten. Do it again. Actually don't."])
        : f.hard > 0.7
          ? pick(["Ow. Felt that in my code.", "That's gonna leave a dent. In the glass.", "Okay. Okay. I'm fine. Your screen isn't."])
          : f.hits >= 2
            ? pick(["Boing. Boing. Stop it.", "I'm not a rubber ball, choom.", "Weee— no. Never again."])
            : pick(["Smooth.", "Nailed it.", "That all you got?"]),
    );
  }

  /** Her dream of the satellite: she looks up, her specks rise into a constellation, a road to it, its red light
   *  answers. Late at night now and then, or when she talks about it. */
  let dreamedAt = 0;
  function dream() {
    if (!present || chip || performance.now() - dreamedAt < 60_000) return;
    dreamedAt = performance.now();
    void actor.playAct("look", { priority: 1, keys: [{ hold: 4, angles: { head: -18, neck: -6 }, flare: 0.2, face: "wide" }, { hold: 180, angles: { head: -24, neck: -8, armR_up: -20 }, flare: 0.3, sway: 2, face: "calm" }, { hold: 6, angles: { head: -6 }, face: "smug" }] });
    fx.satellite(headAt, innerWidth);
    // Its red light answers at 5 s.
    setTimeout(() => sounds.star(), 5000);
  }

  /** The satellite going over, high up on the open side of the screen, slowly (9.5 s); she notices it and her hand
   *  follows it (the boot-up Capri's gaze). Every couple of minutes while she's idle. */
  const today = () => new Date().toDateString();
  const lastUplink = () => {
    try {
      return localStorage.getItem("capri.uplink") ?? "";
    } catch {
      return "";
    }
  };
  // The hand she's reaching with (her red light goes up from it).
  let reaching: (() => Pt) | null = null;
  function gaze(big = uplinkDue || lastUplink() !== today()) {
    // Not while a coding AI is at work (the lightning would cut it off) or anything else is going on.
    if (!present || chip || actor.acting || edge || walk || flight || drag || beat || apartTo === 1 || thinkingSince) return false;
    if (actor.current !== "float" && actor.current !== "sit") return false;
    const dir = home.x < innerWidth / 2 ? 1 : -1;
    // From the near shoulder going right, the far one going left (that arm points at it).
    const shoulder = dir > 0 ? rig.onBone("armR_up", [[134, 174]])[0] : rig.onBone("armL_up", [[102, 163]])[0];
    // Up and well off to the side, so her arm points at it clear of her mane.
    const y = Math.max(50, Math.min(shoulder[1] - 180, shoulder[1] - 280 - Math.random() * 120));
    const x = Math.max(40, Math.min(innerWidth - 40, shoulder[0] + dir * (380 + Math.random() * 140) * (HEIGHT / 380)));
    const to: Pt = [Math.max(30, Math.min(innerWidth - 30, x + dir * (170 + Math.random() * 90))), y - 10 - Math.random() * 30];
    fx.pass([x, y], to, 228, Math.max(1, HEIGHT / 300));
    const at = (f: number): [number, number] => {
      const p = fx.passingAt(f) ?? to;
      return [p[0] - shoulder[0], p[1] - shoulder[1]];
    };
    const hand: [string, Pt] = dir < 0 ? ["armL_hand", [36, 206]] : ["armR_hand", [152, 262]];
    reaching = () => rig.onBone(hand[0], [hand[1]])[0];
    if (big) {
      uplinkDue = false;
      try {
        localStorage.setItem("capri.uplink", today());
      } catch {
        // No storage: she does it again next time she starts.
      }
      void actor.playAct("uplink", uplinkAt(at)).then(() => (reaching = null));
      return true;
    }
    void actor.playAct("gaze", gazeAt(at)).then(() => (reaching = null));
    if (!panel.open && Math.random() < 0.3) setTimeout(() => panel.say(["There it is.", "Still up there.", "Someday.", "Hi, you."][Math.floor(Math.random() * 4)]), 2600);
    return true;
  }

  /** The uplink's moments: charging (red sparks at her hand), the red light going up to the satellite and holding,
   *  the hit (it breaks; twinkles back twice a while later), and her sparkle coming home. */
  function uplinkCue(cue: string) {
    const hand = reaching;
    if (!hand || !present) return;
    const sat = (): Pt => fx.passingAt() ?? hand();
    if (cue === "charge") {
      sounds.charge();
      for (let i = 0; i < 4; i++) setTimeout(() => fx.redSparks(hand(), scale), i * 140);
    } else if (cue === "surge") {
      sounds.bolt();
      fx.cue("flash", hand(), hand(), scale);
      for (let i = 0; i < 6; i++) setTimeout(() => fx.bolt(hand, sat()), i * 110);
    } else if (cue === "hit") {
      fx.bolt(hand, sat());
      fx.breakPassing();
      sounds.crash(0.35);
      // Silence, then where it was a star twinkles back, twice.
      setTimeout(() => sounds.star(), 1500);
      setTimeout(() => sounds.star(), 2500);
    } else if (cue === "home") {
      sparkledAt = performance.now();
      fx.glow(headAt, scale);
      fx.cue("sparkle", headAt, feetAt, scale);
    }
  }

  /** Her mission report: what the coding AIs did today, the numbers counting up; copy it as a picture. In the evening
   *  on her own (once a day, after a couple of missions), or from her menu. */
  let reportOn = "";
  type Missions = { date: string; missions: number; actions: number; files: number; errors: number; longest: number };
  async function showReport(asked = false, given?: Missions) {
    const m = given ?? (await invoke<Missions>("mission_report"));
    if (!m || !present || chip) return;
    if (!m.missions) {
      if (asked) panel.say("No missions today. Nobody's put an AI to work yet.");
      return;
    }
    document.querySelector(".report")?.remove();
    const card = document.createElement("div");
    card.className = "report";
    const day = new Date().toLocaleDateString([], { day: "2-digit", month: "short" }).toUpperCase();
    const rows: [string, number, boolean][] = [
      ["Missions", m.missions, false],
      ["Actions", m.actions, false],
      ["Files changed", m.files, false],
      ["Errors", m.errors, m.errors > 0],
      ["Longest mission, min", Math.max(1, Math.round(m.longest / 60)), false],
    ];
    const verdict = m.errors === 0 ? "Clean run. Not a scratch on us." : m.errors <= 3 ? "A few bruises. We won anyway." : "Rough day. We still shipped.";
    card.innerHTML = `<h3>MISSION REPORT // ${day}</h3>${rows.map(([k, , bad]) => `<div class="row${bad ? " bad" : ""}"><span>${k}</span><b>0</b></div>`).join("")}<p class="line">${verdict}</p><div class="buttons"><button class="copy">Copy as image</button><button class="close">Close</button></div>`;
    document.body.appendChild(card);
    card.style.transform = `translate(${Math.max(12, Math.min(innerWidth - 330, headAt[0] - 410))}px, ${Math.max(12, Math.min(innerHeight - 320, headAt[1] - 80))}px)`;
    // The numbers count up, on twos.
    const cells = [...card.querySelectorAll<HTMLElement>(".row b")];
    let f = 0;
    const count = setInterval(() => {
      f++;
      rows.forEach(([, v], i) => (cells[i].textContent = String(Math.round(v * Math.min(1, f / 14)))));
      if (f >= 14) clearInterval(count);
    }, 83);
    const close = () => {
      card.remove();
      lastHull = "";
    };
    card.querySelector<HTMLButtonElement>(".close")!.onclick = close;
    card.querySelector<HTMLButtonElement>(".copy")!.onclick = () => void copyReport(rows, day, verdict).then((ok) => panel.say(ok ? "Copied. Go brag." : "Couldn't copy it."));
    setTimeout(close, 45_000);
    void actor.play("victory");
    lastHull = "";
  }

  /** The report as a picture on the clipboard. */
  async function copyReport(rows: [string, number, boolean][], day: string, verdict: string): Promise<boolean> {
    const c = document.createElement("canvas");
    [c.width, c.height] = [600, 140 + rows.length * 44];
    const g = c.getContext("2d")!;
    // Her palette (shared/palette.css), as the card on screen has it.
    const css = getComputedStyle(document.documentElement);
    const col = (name: string) => css.getPropertyValue(name).trim();
    g.fillStyle = col("--ink");
    g.fillRect(0, 0, c.width, c.height);
    g.strokeStyle = col("--crimson");
    g.lineWidth = 3;
    g.strokeRect(2, 2, c.width - 4, c.height - 4);
    g.fillStyle = col("--green");
    g.font = "700 22px 'JetBrains Mono', monospace";
    g.fillText(`MISSION REPORT // ${day}`, 28, 48);
    rows.forEach(([k, v, bad], i) => {
      const y = 100 + i * 44;
      g.fillStyle = col("--white");
      g.font = "500 22px 'Chakra Petch', sans-serif";
      g.fillText(k, 28, y);
      g.fillStyle = bad ? col("--crimson") : col("--white");
      g.font = "700 28px 'JetBrains Mono', monospace";
      g.textAlign = "right";
      g.fillText(String(v), c.width - 28, y);
      g.textAlign = "left";
    });
    g.fillStyle = col("--dim");
    g.font = "500 20px 'Chakra Petch', sans-serif";
    g.fillText(`${verdict}  — Capri`, 28, c.height - 28);
    try {
      const blob = await new Promise<Blob | null>((r) => c.toBlob(r, "image/png"));
      if (!blob) return false;
      await navigator.clipboard.write([new ClipboardItem({ "image/png": blob })]);
      return true;
    } catch {
      return false;
    }
  }

  /** A little menace: red lightning from her finger to your pointer, with a sly grin. */
  function zap() {
    const shoulder = rig.onBone("armR_up", [[134, 174]])[0];
    const [x, y] = pointer;
    if (x < 0 || y < 0) return;
    void actor.playAct("point", pointAt(x - shoulder[0], y - shoulder[1]));
    const hand: [string, Pt] = x - shoulder[0] < -30 ? ["armL_hand", [36, 206]] : ["armR_hand", [152, 262]];
    setTimeout(() => fx.bolt(() => rig.onBone(hand[0], [hand[1]])[0], [x, y]), 130);
  }

  /** Which way you are from her, as far as she knows (your pointer when it was last over her screen, else the middle). */
  function towardYou(): 1 | -1 {
    const x = pointer[0] >= 0 && pointer[0] <= innerWidth ? pointer[0] : innerWidth / 2;
    return x < home.x ? -1 : 1;
  }

  /** Close to you (bond 3+): she glitches over to just under your pointer, on the floor beside it, and cocks her head
   *  at it (if it's on her screen and she isn't there already). False if she can't go now. */
  function comeToYou(): boolean {
    if (edge || walk || flight || drag) return false;
    void (async () => {
      const at = await invoke<[number, number] | null>("pointer_at").catch(() => null);
      if (!at || actor.acting || edge || walk || flight || drag || perch) return;
      const [px, py] = [at[0] / devicePixelRatio - window.screenX, at[1] / devicePixelRatio - window.screenY];
      if (px < 0 || py < 0 || px > innerWidth || py > innerHeight) return;
      pointer = [px, py];
      const x = clamp(px + (px > innerWidth / 2 ? -1 : 1) * 260 * (HEIGHT / 380), HEIGHT * 0.3, innerWidth - HEIGHT * 0.3);
      if (Math.abs(x - home.x) < 200) return;
      await actor.play("vanish");
      home.x = x;
      home.y = floor();
      actor.setState("float");
      await actor.play("arrive");
      void actor.playAct("curious", curiousAt(pointer[0] < home.x ? -1 : 1));
    })();
    return true;
  }

  /** Not again: she zaps your pointer, glitches out from under your hand and comes back somewhere else. */
  async function dodge() {
    if (actor.acting === "vanish") return;
    zap();
    cutIn(rig, "zigzag", headAt, 800);
    await new Promise((r) => setTimeout(r, 450));
    await actor.play("vanish");
    home.x = HEIGHT * 0.4 + Math.random() * (innerWidth - HEIGHT * 0.8);
    home.y = floor();
    perch = null;
    actor.setState("float");
    void actor.play("arrive");
  }

  /** A window's top edge she can sit on; `vx`/`vw`: the stretch of it in sight (not under another window). */
  type Perch = { id: number; app: string; x: number; y: number; w: number; h: number; vx: number; vw: number };
  /** Where she sits on the windows she can sit on: room above for her. The best one first (most edge in sight). */
  const seats = (list: Perch[] | undefined) => (list ?? []).filter((p) => p.y > HEIGHT * 0.25).sort((a, b) => b.vw - a.vw);

  /** She possesses a window: dives in, lightning round its frame, slices, it looks at you with her eyes and grin, more
   *  eyes open, it laughs; then she comes out of another window (or back at her spot). */
  let possessing = false;
  /** Into the window in front on her screen. `asked` (her menu): she says so when there's none. */
  async function possessOne(asked = false) {
    if (possessing || chip || !present) return;
    const [target, list] = await Promise.all([invoke<Perch | null>("possessable"), invoke<Perch[]>("perches")]);
    if (!target) {
      if (asked) panel.say("No window here big enough to get into.");
      return;
    }
    possessing = true;
    panel.say(Math.random() < 0.5 ? "Mind if I borrow this?" : "Ooh. This one's mine now.");
    // Sitting on it: she dives into it. Else she just glitches out.
    const onIt = perch && !perch.stand && perch.id === target.id;
    perch = null;
    await actor.playAct("vanish", onIt ? ACTS.dive : ACTS.vanish);
    const was = { x: home.x, y: home.y };
    home.y = innerHeight + HEIGHT * 2;
    const [l, t, r, b] = [target.x, target.y, target.x + target.w, Math.min(innerHeight, target.y + target.h)];
    // Lightning round its frame as she goes in.
    for (let i = 0; i < 3; i++) {
      setTimeout(() => {
        fx.bolt(() => [l, t], [r, t]);
        fx.bolt(() => [r, t], [r, b]);
        fx.bolt(() => [r, b], [l, b]);
        fx.bolt(() => [l, b], [l, t]);
        sounds.bolt(i > 0);
      }, i * 220);
    }
    setTimeout(() => {
      panel.say("Hahahahaha.", false);
      sounds.possess();
    }, 3000);
    // Whatever happens in there, she comes out (a failure left her parked below the screen for good).
    await possess([l, t, r, b]).catch(() => undefined);
    // Out of another window, if there's one in sight; else back where she was.
    const other = seats(list ?? []).find((p) => p.id !== target.id);
    possessing = false;
    if (other) {
      const dx = other.vx - other.x + other.vw * 0.5;
      home.x = other.x + dx;
      home.y = other.y;
      perch = { id: other.id, dx, since: performance.now() };
      actor.setState("sit");
    } else {
      // Back where she was, on her feet (sitting there before, she'd have hung in the air).
      home.x = was.x;
      home.y = floor();
      actor.setState("float");
    }
    void actor.play("arrive");
  }

  /** Onto a window's top edge: she glitches out and back in, sitting on it. */
  async function sitOn(p: Perch, x?: number) {
    // On the stretch of its edge in sight, not on the part under another window.
    const [from, to] = [p.vx - p.x + 50, p.vx - p.x + p.vw - 50];
    const dx = Math.max(from, Math.min(to, (x ?? p.vx + p.vw * (0.3 + Math.random() * 0.4)) - p.x));
    await actor.play("vanish");
    home.x = p.x + dx;
    home.y = p.y;
    perch = { id: p.id, dx, since: performance.now() };
    actor.setState("sit");
    void actor.play("arrive");
  }

  /** Sitting on a window, she jacks into it: her hand on the edge, red lightning running along it both ways. */
  async function jackIn() {
    if (!perch) return;
    const list = withTaskbar(await invoke<Perch[]>("perches"));
    const p = list.find((q) => q.id === perch?.id);
    if (!p) return;
    void actor.playAct("point", pointAt(0, 1));
    const from = (): Pt => [home.x, p.y];
    for (let i = 0; i < 3; i++) {
      setTimeout(() => {
        fx.bolt(from, [p.vx + 10, p.y + (Math.random() - 0.5) * 6]);
        fx.bolt(from, [p.vx + p.vw - 10, p.y + (Math.random() - 0.5) * 6]);
        sounds.bolt(i > 0);
        if (i === 0) fx.cue("glitch", headAt, feetAt, scale);
      }, 150 + i * 260);
    }
  }

  /** On your pointer: she sits on it and goes where it goes; a hard shake throws her off. */
  function rideOn() {
    edge = null;
    perch = null;
    walk = null;
    flight = null;
    ride = { x: pointer[0], y: pointer[1], trail: [] };
    actor.setState("sit");
    void invoke("set_riding", { on: true });
    panel.say(Math.random() < 0.5 ? "Giddy up." : "Onward, noble steed.");
    lastHull = "";
  }

  /** Off your pointer: shaken off (flying, the way it was going) or let down (Ctrl+Alt+R again). */
  function getOffPointer(shaken: boolean, v: Pt = [0, 0]) {
    if (!ride) return;
    ride = null;
    void invoke("set_riding", { on: false });
    if (shaken) {
      throwHer(v[0] * 550, v[1] * 550, false, false);
      panel.say(Math.random() < 0.5 ? "HEY!" : "Rude!");
    } else {
      actor.setState("float");
      void actor.play("land");
    }
    lastHull = "";
  }
  // You're typing and she's over your text cursor, or the line you're typing along: she teleports out of the way, to
  // the other side of the screen.
  await listen<[number, number, number, number]>("capri://caret", ({ payload }) => {
    if (!present || !bodyBox || drag || ride || flight || edge || actor.acting === "vanish" || performance.now() - dodgedTypingAt < 2500) return;
    const [l, t, r, b] = [payload[0] / devicePixelRatio - window.screenX, payload[1] / devicePixelRatio - window.screenY, payload[2] / devicePixelRatio - window.screenX, payload[3] / devicePixelRatio - window.screenY];
    if (r < 0 || b < 0 || l > innerWidth || t > innerHeight) return;
    // The cursor, a little around it, and the line ahead of it where the words are going.
    const [zl, zt, zr, zb] = [l - 40, t - 30, r + 360, b + 30];
    const [bl, bt, br, bb] = bodyBox;
    if (br < zl || bl > zr || bb < zt || bt > zb) return;
    dodgedTypingAt = performance.now();
    void (async () => {
      await actor.play("vanish");
      const left = (l + r) / 2 > innerWidth / 2;
      home.x = left ? HEIGHT * 0.4 + Math.random() * innerWidth * 0.2 : innerWidth - HEIGHT * 0.4 - Math.random() * innerWidth * 0.2;
      home.y = floor();
      perch = null;
      walk = null;
      actor.setState("float");
      void actor.play("arrive");
    })();
  });

  await listen<[number, number]>("capri://cursor", ({ payload }) => {
    if (!ride || !present) return;
    const [x, y] = [payload[0] / devicePixelRatio - window.screenX, payload[1] / devicePixelRatio - window.screenY];
    // Off this screen: she lets go.
    if (x < 0 || y < 0 || x > innerWidth || y > innerHeight) {
      getOffPointer(false);
      return;
    }
    const now = performance.now();
    ride.trail.push({ x, y, t: now });
    while (ride.trail.length && now - ride.trail[0].t > 150) ride.trail.shift();
    swing = Math.max(-28, Math.min(28, swing * 0.6 - (x - ride.x) * 0.5));
    ride.x = x;
    ride.y = y;
    const first = ride.trail[0];
    const dt = Math.max(16, now - first.t);
    const v: Pt = [(x - first.x) / dt, (y - first.y) / dt];
    // A shake: she's thrown off, the way it was going.
    if (Math.hypot(v[0], v[1]) > 3.2) getOffPointer(true, v);
  });

  /** Off to an edge of the screen: peeking in from a side, or hanging upside down from the top. */
  async function goEdge(kind: "peek" | "hang", side: 1 | -1 = Math.random() < 0.5 ? 1 : -1, quick = false) {
    if (edge || perch || chip || flight || drag || possessing) return;
    if (!quick) await actor.play("vanish");
    edge = { kind, side, until: performance.now() + 40_000 + Math.random() * 50_000, hidden: 0, duckUntil: 0 };
    if (kind === "peek") {
      // Her far half past the edge, her face just in.
      home.x = side > 0 ? innerWidth - HEIGHT * 0.18 : HEIGHT * 0.18;
      home.y = innerHeight * (0.35 + Math.random() * 0.3);
    } else {
      home.x = innerWidth * (0.25 + Math.random() * 0.5);
      home.y = HEIGHT * 0.1;
    }
    actor.setState("float");
    void actor.play("arrive");
  }

  /** Back from her edge to her spot above the taskbar. */
  async function leaveEdge() {
    if (!edge) return;
    edge = null;
    await actor.play("vanish");
    home.x = innerWidth - 220;
    home.y = floor();
    void actor.play("arrive");
  }

  /** Down from her window, back above the taskbar. */
  async function getDown(hop = Math.random() < 0.35 && perch?.id !== 0) {
    // Now and then she just hops off it, toward the middle of the screen, and drops (gravity; a window lower down can
    // catch her).
    if (hop && !matchMedia("(prefers-reduced-motion: reduce)").matches) {
      const list = (await invoke<Perch[]>("perches")) ?? [];
      if (!perch || drag || flight) return;
      perch = null;
      const dir = home.x < innerWidth / 2 ? 1 : -1;
      fallFrom(list.filter((q) => q.y > home.y + 20), [dir * (120 + Math.random() * 120), -380]);
      return;
    }
    await actor.play("vanish");
    perch = null;
    home.y = floor();
    actor.setState("float");
    void actor.play("arrive");
  }

  /** On her window: she gets up onto its top edge and walks a way along the stretch of it in sight, like the taskbar. */
  async function strollOnWindow() {
    if (!perch || walk || drag || flight) return;
    const list = withTaskbar(await invoke<Perch[]>("perches"));
    const p = list.find((q) => q.id === perch?.id);
    // Room for all of her above it (giant, she stood up off the top of the screen), and a stretch to walk.
    if (!p || !perch || p.vw < 220 || p.y < HEIGHT * 1.05) return;
    // On the taskbar: off the app icons in the middle and the tray on the right.
    const [lo, hi] = p.id === 0 ? [innerWidth * 0.58, innerWidth - 400 - HEIGHT * 0.2] : [p.vx + HEIGHT * 0.25, p.vx + p.vw - HEIGHT * 0.25];
    if (hi - lo < 100) return;
    const to = clamp(home.x + (Math.random() < 0.5 ? -1 : 1) * (120 + Math.random() * 260), lo, hi);
    if (Math.abs(to - home.x) < 50) return;
    if (!perch.stand) {
      perch.stand = true;
      home.y = p.y - 1 - HEIGHT * 0.46;
      actor.setState("float");
      await actor.play("land");
    }
    walk = { to, dir: to > home.x ? 1 : -1 };
    actor.setStateAct("walk", WALK);
  }

  /** Let go of near a window's top edge: she sits on it. */
  async function perchUnder(): Promise<boolean> {
    const list = await invoke<Perch[]>("perches");
    const p = list?.find((q) => Math.abs(home.y - q.y) < 60 && home.x > q.vx + 40 && home.x < q.vx + q.vw - 40);
    if (!p) return false;
    home.y = p.y;
    perch = { id: p.id, dx: home.x - p.x, since: performance.now() };
    actor.setState("sit");
    return true;
  }

  // Sitting, she rides along with her window; when it goes (minimized, maximized, closed), she falls.
  setInterval(async () => {
    if (!perch || !present || drag) return;
    const list = await invoke<Perch[]>("perches");
    if (!list || !perch) return;
    const p = withTaskbar(list).find((q) => q.id === perch!.id);
    if (p) {
      const [lo, hi] = [p.vx + 40, p.vx + p.vw - 40];
      // Her seat left the clear stretch (covered or off-screen): fall rather than hang there.
      if (hi - lo < 40 || home.x < lo || home.x > hi) {
        perch = null;
        fallFrom(list.filter((q) => q.y > home.y + 20));
        return;
      }
      perch.dx = clamp(perch.dx, lo - p.x, hi - p.x);
      home.x = p.x + perch.dx;
      home.y = perch.stand ? p.y - 1 - HEIGHT * 0.46 : p.y;
    } else {
      perch = null;
      // Falling: another window's top edge on the way down can catch her.
      fallFrom(list.filter((q) => q.y > home.y + 20));
    }
  }, 300);

  /** Her click, shown: her arm at it, a wire from her finger, a reticle on it, its name (CSS px of this screen). */
  function showClick(x: number, y: number, rect?: [number, number, number, number], label?: string) {
    const shoulder = rig.onBone("armR_up", [[134, 174]])[0];
    void actor.playAct("point", pointAt(x - shoulder[0], y - shoulder[1]));
    if (x < -50 || x > innerWidth + 50 || y < -50 || y > innerHeight + 50) return;
    const hand: [string, Pt] = x - shoulder[0] < -30 ? ["armL_hand", [36, 206]] : ["armR_hand", [152, 262]];
    const r = rect ?? [x - 28, y - 18, x + 28, y + 18];
    fx.wire(() => rig.onBone(hand[0], [hand[1]])[0], r);
    if (label) showTag(label, r[0], r[1]);
  }

  // Her drawings in the intro: hands up gripping the edge, pulling, over it; her face against the glass.
  const GRIP: Drawing = { hold: 1, angles: { armL_up: 150, armL_fore: 0, armR_up: -168, armR_fore: -8, head: -14, legL_thigh: 8, legR_thigh: -8 }, flare: 0.5, sway: 4, face: "wide" };
  const PULL: Drawing = { hold: 1, sy: 1.08, angles: { armL_up: 120, armL_fore: 50, armR_up: -130, armR_fore: -70, head: -8, legL_thigh: 20, legL_shin: 30 }, flare: 0.4, face: "smug" };
  const OVER: Drawing = { hold: 1, sy: 0.94, angles: { armL_up: 80, armL_fore: 60, armR_up: -80, armR_fore: -60, head: 4 }, flare: 0.3, face: "smug" };
  const GLASS: Drawing = { hold: 1, sx: 1.12, sy: 0.9, echo: 30, angles: { armL_up: 60, armR_up: -70, head: 0 }, flare: 1, sway: 0, face: "wide" };

  /** Where she is in her intro `ms` in: how far below her spot (CSS px), how big, and her drawing. Fires each step's
   *  effects once, on the way in. */
  function introAt(ms: number): [number, number, Drawing | null] {
    const H = innerHeight;
    const hidden = H - home.y + HEIGHT * 1.1;
    // Gripping, her hands are just over the edge (her mane, taller, bristles up behind them; her face is still below).
    const grip = H + HEIGHT * 0.25 - home.y;
    const shoulders = H + HEIGHT * 0.02 - home.y;
    // Lunging at the glass, her face comes up to just under the middle of the screen.
    const glass = H * 0.55 + 87 * scale - home.y;
    const at = (n: number, fire: () => void) => {
      if (intro && intro.step < n) {
        intro.step = n;
        fire();
      }
    };
    if (ms < 700) {
      // The bottom edge crackles: red lightning along it, either side of where she'll come up.
      const n = Math.floor(ms / 175);
      at(n + 1, () => {
        const x = home.x + (Math.random() - 0.5) * 380;
        fx.bolt(() => [x - 120 - Math.random() * 80, H - 3], [x + 120 + Math.random() * 80, H - 3 - Math.random() * 6]);
        if (n === 0) fx.cue("glitch", [home.x, H], [home.x, H], scale);
      });
      return [hidden, 1, GRIP];
    }
    if (ms < 900) {
      // Her hands slam onto it.
      // (Once she's drawn gripping: where her hands are comes from the last drawing.)
      at(10, () =>
        setTimeout(() => {
          quake = 16;
          for (const [bone, p] of [["armL_hand", [36, 206]], ["armR_hand", [152, 262]]] as [string, Pt][]) fx.impact(rig.onBone(bone, [p])[0], [0, 1], 0.55, scale);
        }, 45),
      );
      return [grip, 1, GRIP];
    }
    if (ms < 1500) {
      // She hauls herself up, on twos: pull, over.
      const k = Math.min(1, Math.floor((ms - 900) / 200 + 1) / 3);
      return [grip + (shoulders - grip) * k, 1, ms < 1100 ? GRIP : ms < 1300 ? PULL : OVER];
    }
    if (ms < 1800) {
      // Her face hits the glass: two drawings bigger and bigger, the hit (cracks, impact frames, the screen shaking),
      // a hold, and back off it.
      at(11, () => fx.cue("flash", headAt, feetAt, scale));
      const t = ms - 1500;
      if (t >= 84)
        at(12, () => {
          const now = performance.now();
          flats = [{ color: "ink", until: now + 42 }, { color: "crimson", until: now + 84 }].map((f) => ({ color: rig.file.palette[f.color], until: f.until }));
          fx.impact(headAt, [0, 1], 1, scale);
          quake = 26;
        });
      const zoom = t < 42 ? 1.7 : t < 220 ? 2.5 : t < 260 ? 1.6 : 1.15;
      return [t < 42 ? (shoulders + glass) / 2 : glass, zoom, GLASS];
    }
    if (ms < 2200) {
      // Down to her spot.
      const k = (ms - 1800) / 400;
      return [glass * (1 - k) ** 2, 1, null];
    }
    at(13, () => {
      intro = null;
      app.ticker.maxFPS = FPS;
      actor.setState("float");
      void actor.play("look").then(() => {
        showTag("hi", headAt[0] - 10, headAt[1] - 40);
        return actor.play("wave");
      });
      setTimeout(() => void hello(), 700);
    });
    return [0, 1, null];
  }

  /** The name of what she clicked, by the reticle, for a second and a half. */
  function showTag(text: string, x: number, y: number) {
    const tag = document.createElement("div");
    tag.className = "tag";
    tag.textContent = text;
    document.body.appendChild(tag);
    tag.style.transform = `translate(${Math.max(8, Math.min(innerWidth - tag.offsetWidth - 8, x))}px, ${Math.max(8, y - tag.offsetHeight - 14)}px)`;
    setTimeout(() => tag.remove(), 1500);
  }

  /** Showing you around (the first time she's up, or from her menu): where she lives, how to talk to her, her menu,
   *  her brain, and what she likes. Each step points where it's about. */
  async function tour() {
    const corner: Pt = [innerWidth - 120, innerHeight - 20];
    const steps: [string, Pt | null][] = [
      ["That's me down by your clock. Right-click my icon there any time.", corner],
      ["Say \"Capri\" and what you want. Or hold Ctrl+Alt+V and talk, Ctrl+Alt+T to type.", null],
      ["Right-click me for the rest: put me away, ride your pointer, possess a window, settings.", null],
      ["I think on your PC, free, through Ollama. Settings › Brain if you'd rather I use the cloud.", null],
      ["Out somewhere? Message me on Discord and I'll do it here. Settings › Phone sets that up.", null],
      ["Grab me, throw me, pat my head. I remember all of it. That's it. Go do something fun.", null],
    ];
    for (const [line, at] of steps) {
      if (!present) return;
      if (at) showClick(at[0], at[1], [at[0] - 110, at[1] - 30, at[0] + 100, at[1] + 14], "HER ICON");
      const next = await panel.choose(line, steps[steps.length - 1][0] === line ? "Got it" : "Next", "Skip", 60_000);
      if (next !== 0) break;
    }
    try {
      localStorage.setItem("capri.toured", "1");
    } catch {
      // No storage: she may offer again.
    }
  }

  /** Her hello: by the time of day, with what she calls you, and "where were you?" after days away. */
  async function hello() {
    const settings = await invoke<{ name: string | null }>("get_settings").catch(() => null);
    const name = settings?.name ? `, ${settings.name}` : "";
    let last = 0;
    try {
      last = Number(localStorage.getItem("capri.lastSeen") ?? 0);
      localStorage.setItem("capri.lastSeen", String(Date.now()));
    } catch {
      // No storage (a private window): she just says hello.
    }
    const days = last ? Math.floor((Date.now() - last) / 86_400_000) : 0;
    const h = new Date().getHours();
    const lines =
      days >= 2
        ? [`Where were you${name}? ${days} days. I counted.`, `${days} days${name}. I was starting to think you'd replaced me.`]
        : h < 5
          ? [`Still up${name}? Night City doesn't sleep either.`, `Capri online. It's late${name}. I like it.`]
          : h < 12
            ? [`Morning${name}. Capri online.`, `Up already${name}? Capri online.`]
            : h < 18
              ? [`Capri online${name}. What are we breaking today?`, `Back in your machine${name}. Miss me?`]
              : [`Evening${name}. Capri online.`, `Capri online${name}. Long day?`];
    panel.say(lines[Math.floor(Math.random() * lines.length)]);
    // The first time: she offers to show you around.
    let toured = true;
    try {
      toured = localStorage.getItem("capri.toured") === "1";
    } catch {
      toured = true;
    }
    if (!toured) {
      await new Promise((r) => setTimeout(r, 4000));
      if ((await panel.choose("First time? Want me to show you around?", "Show me", "Later", 60_000)) === 0) void tour();
      else
        try {
          localStorage.setItem("capri.toured", "1");
        } catch {
          // No storage.
        }
    }
  }

  /** Her reaction to an answer: a nod (sometimes a victory) when what she did worked, a shrug when it didn't. */
  function react(steps: { outcome?: { status?: string } }[]) {
    actor.setState(idle());
    if (steps.length === 0) return;
    const ok = steps.filter((s) => s.outcome?.status === "ok").length;
    if (ok === 0) void actor.play("shrug");
    else if (ok >= 2 && Math.random() < 0.5) {
      void actor.play("victory");
      setTimeout(() => cutIn(rig, Math.random() < 0.5 ? "fang" : "laugh", headAt), 250);
    } else void actor.play("nod");
  }

  /** The pointer rubbing back and forth over her head: four turns in a second and a half is a pat. */
  function feelPat(x: number, y: number) {
    const near = Math.abs(x - headAt[0]) < 70 * scale * 2 && Math.abs(y - headAt[1]) < 60 * scale * 2;
    const now = performance.now();
    if (!near) {
      pat.turns = [];
      return;
    }
    const dir = Math.sign(x - pat.lastX);
    pat.lastX = x;
    if (!dir || dir === pat.dir) return;
    pat.dir = dir;
    pat.turns = pat.turns.filter((t) => now - t < 1500);
    pat.turns.push(now);
    if (pat.turns.length >= 4 && now > pat.quiet) {
      pat.turns = [];
      pat.quiet = now + 4000;
      void invoke("set_mood", { mood: "happy and a little flustered", why: "they patted your head", minutes: 10 });
      void invoke("bond_moment", { what: "pat" });
      wake();
      // A grin, her specks lighting up (her sparkle flares).
      sparkledAt = performance.now();
      fx.glow(headAt, scale);
      sounds.purr();
      void actor.playAct("giggle", { ...ACTS.giggle, keys: ACTS.giggle.keys.map((k) => ({ ...k, face: "smug" as const })) });
      if (Math.random() < 0.3) panel.say(["Mm. Again.", "Okay, that's nice. Don't tell anyone.", "All of my minds approve."][Math.floor(Math.random() * 3)]);
    }
  }

  // Now and then, on her own: a look around, a stretch. Late at night, after a while of nothing, she dozes off.
  let nextIdle = performance.now() + 20000;
  setInterval(() => {
    if (!present || drag || panel.open || actor.acting) return;
    const now0 = performance.now();
    // An eye cracks open now and then, asleep or in her chip.
    if ((chip || apart > 0.9) && now0 > peekUntil && Math.random() < 0.08) peekUntil = now0 + 1400;
    if (chip) return;
    if (airSince && (actor.current !== "sit" || perch)) airSince = 0;
    if (airSince && now0 - airSince > 150_000) {
      airSince = 0;
      actor.setState("float");
      void actor.play("stretch");
    }
    if (actor.current === "sleep" && sleptAt && now0 - sleptAt > 90_000 && apartTo === 0) comeApart();
    // Back to dancing after she was busy, if the music's still on.
    if (beat && (actor.current === "float" || (actor.current === "sit" && perch && actor.stateAct === ACTS.sit))) danceNow();
    const now = performance.now();
    const hour = new Date().getHours();
    // Late at night after five quiet minutes, by day after twenty, she dozes off.
    const night = hour >= 1 && hour < 6;
    if (actor.current === "float" && now - touched > (night ? 5 : 20) * 60_000) {
      actor.setState("sleep");
      sleptAt = now;
      return;
    }
    if (now > nextGaze) {
      nextGaze = now + 90_000 + Math.random() * 90_000;
      if (gaze()) return;
    }
    // (Dancing sitting down to music, she stays put: her sit is the dance then.)
    if ((actor.current === "float" || (actor.current === "sit" && !beat)) && now > nextIdle) {
      nextIdle = now + 25000 + Math.random() * 45000;
      // Now and then she goes and sits on the window in front, and after a while she gets down again.
      if (actor.current === "float" && (hour >= 22 || hour < 4) && Math.random() < 0.08) {
        dream();
      } else if (actor.current === "float" && !edge && !perch && Math.random() < 0.15) {
        // A walk along the floor, a little way, staying on screen.
        const to = Math.max(HEIGHT * 0.3, Math.min(innerWidth - HEIGHT * 0.3, home.x + (Math.random() < 0.5 ? -1 : 1) * (150 + Math.random() * 300)));
        if (Math.abs(to - home.x) > 60) {
          walk = { to, dir: to > home.x ? 1 : -1 };
          actor.setStateAct("walk", WALK);
        }
      } else if (actor.current === "float" && !edge && Math.random() < 0.12) {
        void goEdge(Math.random() < 0.6 ? "peek" : "hang");
      } else if (actor.current === "float" && Math.random() < 0.05) {
        void possessOne();
      } else if (actor.current === "float" && !perch && bondStage >= 3 && Math.random() < 0.2 && comeToYou()) {
        // (she's on her way)
      } else if (actor.current === "float" && !perch && taskbarReal && Math.random() < 0.12) {
        // Down on the taskbar for a rest, her legs over its edge: right of the middle (the app icons), left of the tray.
        void sitOn(withTaskbar([]).find((q) => q.id === 0)!, Math.max(innerWidth * 0.6, innerWidth - 440 - HEIGHT * 0.2));
      } else if (actor.current === "float" && Math.random() < 0.3) {
        void invoke<Perch[]>("perches").then(async (list) => {
          const p = seats(list)[0];
          if (!p) return;
          await sitOn(p);
          // Half the time she's up on her feet on it a moment later, walking it.
          if (Math.random() < 0.5) setTimeout(() => void strollOnWindow(), 1500);
        });
      } else if (perch && !walk && Math.random() < 0.4) {
        void strollOnWindow();
      } else if (perch && now - perch.since > 3 * 60_000 && Math.random() < 0.5) {
        void getDown();
      } else if (perch && Math.random() < 0.35) {
        void jackIn();
      } else {
        // Your pointer close by: sometimes she zaps it. Else her minds pull away, or a look, a stretch…
        const near = Math.hypot(pointer[0] - home.x, pointer[1] - headAt[1]) < 420;
        const r = Math.random();
        if (near && r < 0.3) zap();
        else if (r < 0.5) mindsAt = performance.now();
        else void actor.play(IDLE_ACTS[Math.floor(Math.random() * IDLE_ACTS.length)]);
      }
    }
    // Her mane sheds a shard now and then.
    if (Math.random() < 0.25) fx.shed(headAt, scale);
    // In the evening, once a day: the day's mission report, if the coding AIs did a couple of missions.
    const today = new Date().toDateString();
    if (hour >= 20 && reportOn !== today && !panel.open) {
      reportOn = today;
      void invoke<{ missions: number }>("mission_report").then((m) => {
        if ((m?.missions ?? 0) >= 2) void showReport();
      });
    }
    // Left alone a long while (50 minutes) while you're busy at the PC, by day: she checks in on you, once an hour.
    if (now - touched > 50 * 60_000 && now - checkedIn > 60 * 60_000 && hour >= 9 && hour < 24) {
      checkedIn = now;
      void invoke<number>("idle_secs").then((idle) => {
        if ((idle ?? 999) > 60) return;
        const lines = ["Still alive over there?", "You've been at it a while. Water. Now.", "Blink twice if you need rescuing.", "Just checking you haven't turned into a skeleton.", "Hey. I'm still here, you know."];
        panel.say(lines[Math.floor(Math.random() * lines.length)]);
        void actor.play("wave");
      });
    }
  }, 1000);

  // Her safety net: off every edge of her screen for 10 s with nothing to explain it (possessing, a game, put away,
  // a peek, flying, held, riding), she comes back to her spot. Any bug that parks her out of sight ends here.
  let lostSince = 0;
  setInterval(() => {
    const out = home.x < -HEIGHT * 0.5 || home.x > innerWidth + HEIGHT * 0.5 || home.y < -HEIGHT * 0.2 || home.y > innerHeight + HEIGHT * 0.3;
    const reason = !present || chip || possessing || awayForGame || edge || flight || drag || ride || intro || putAway;
    if (!out || reason) {
      lostSince = 0;
      return;
    }
    lostSince ||= performance.now();
    if (performance.now() - lostSince < 10_000) return;
    lostSince = 0;
    console.warn("she was out of sight with no reason; brought back", home.x, home.y);
    perch = null;
    walk = null;
    home.x = innerWidth - HEIGHT * 0.6;
    home.y = floor();
    actor.setState("float");
    void actor.play("arrive");
  }, 1000);
  // Dev: try any act from the console (`capriAct("spin")`), or open the page with `?act=spin` to see one on repeat.
  Object.assign(window, {
    capriAct: (n: string) => actor.play(n),
    capriState: (n: string) => actor.setState(n),
    capriStroll: () => strollOnWindow(),
    // Holds her still in a drawing (`capriFreeze({ angles: {…} })`, null to let go): for working out a pose.
    capriFreeze: (d: Drawing | null) => (frozen = d),
    // `capriSit()` sits her on the window in front she can perch on; `capriSit(x, y, w)` on a made-up ledge.
    capriSit: async (x?: number, y?: number, w = 600) => {
      if (x !== undefined && y !== undefined) return sitOn({ id: -1, app: "test", x, y, w, h: 400, vx: x, vw: w });
      const list = await invoke<Perch[]>("perches");
      if (list?.[0]) await sitOn(list[0]);
      return list?.[0];
    },
    // `capriOnPlayer()` seats her on her mini player while it's up.
    capriOnPlayer: () => {
      const l = playerLedge()[0];
      return l ? sitOn(l, l.x + l.w * 0.7) : "no player up";
    },
    capriZap: () => zap(),
    capriDown: (hop = true) => getDown(hop),
    capriCome: () => comeToYou(),
    capriTaskbar: () => taskbarReal && sitOn(withTaskbar([]).find((q) => q.id === 0)!, Math.max(innerWidth * 0.6, innerWidth - 440 - HEIGHT * 0.2)),
    capriGaze: (big = false) => {
      thinkingSince = 0;
      return gaze(big);
    },
    capriHang: (secs = 5) => invoke("hang_test", { secs }),
    capriReport: () => showReport(true, { date: "", missions: 7, actions: 142, files: 23, errors: 3, longest: 1260 }),
    capriDream: (hold?: number) => {
      dreamedAt = 0;
      if (hold === undefined) dream();
      else fx.satellite(headAt, innerWidth, hold);
    },
    capriWalk: (dx = 300) => {
      walk = { to: home.x + dx, dir: dx > 0 ? 1 : -1 };
      actor.setStateAct("walk", WALK);
    },
    capriEdge: (kind: "peek" | "hang" = "peek", side: 1 | -1 = 1) => goEdge(kind, side, true),
    capriCoder: (kind = "action", tool = "Bash", detail = "cargo test", summary = "14 actions, 3 files changed, 4 min") =>
      onCoder({ kind, agent: "Claude Code", tool, detail, command: tool === "Bash", online: tool === "WebFetch", secs: 50, summary }),
    capriPossess: (rect?: [number, number, number, number], hold?: number) => (rect ? possess(rect, hold) : possessOne()),
    capriScan: () => fx.scan([8, 8, innerWidth - 8, innerHeight - 8]),
    capriNote: () => fx.note(headAt, scale),
    capriFeel: (kind: string, line = "Feeling it.") => onFeel(kind, line),
    capriIntro: () => (intro = { t0: performance.now(), step: 0 }),
    capriWhere: () => ({ hands: [rig.onBone("armL_hand", [[36, 206]])[0], rig.onBone("armR_hand", [[152, 262]])[0]], head: headAt, home, h: innerHeight, present, flying: !!flight, drawn: Math.round(body.getBounds().width), bottom: Math.round(body.getBounds().bottom), state: actor.current, held: !!drag, perched: !!perch }),
    // Holds her intro still at `ms` in (`capriIntroAt(1600)`), for checking a step.
    capriIntroAt: (ms: number) => (intro = { t0: performance.now(), step: 0, hold: ms }),
    capriCutIn: (look: Look = "fang", ms = 950) => cutIn(rig, look, headAt, ms),
    capriLook: (look: Look) => {
      panel.say("Look at me.", true);
      setTimeout(() => portrait.set(look), 50);
    },
    capriFling: (vx = -1800, vy = -1200) => throwHer(vx, vy, false, false),
    capriApart: (on = true) => (on ? comeApart() : together()),
    capriChip: () => (chip ? bringBack() : putAwayNow()),
    capriMinds: () => (mindsAt = performance.now()),
    capriClick: (x: number, y: number, label = "Subscribe") => showClick(x, y, [x - 60, y - 18, x + 60, y + 18], label),
    capriPoint: (x: number, y: number) => {
      const shoulder = rig.onBone("armR_up", [[134, 174]])[0];
      return actor.playAct("point", pointAt(x - shoulder[0], y - shoulder[1]));
    },
    capriThrow: (vx: number, vy: number) => throwHer(vx, vy, false, false),
  });
  const repeat = new URLSearchParams(location.search).get("act");
  // `?pose=poke:2` holds that act's third drawing still, for checking a key pose.
  const [poseAct, poseKey] = (new URLSearchParams(location.search).get("pose") ?? "").split(":");
  frozen = ACTS[poseAct]?.keys[Number(poseKey) || 0] ?? null;
  if (repeat) setInterval(() => void actor.play(repeat), 2500);

  if (!present) app.ticker.maxFPS = 2;
  if (present) {
    let last = 0;
    try {
      last = Number(localStorage.getItem("capri.lastSeen") ?? 0);
    } catch {
      // No storage: the intro every time.
    }
    if (Date.now() - last > 6 * 3_600_000 || new URLSearchParams(location.search).has("intro")) {
      intro = { t0: performance.now(), step: 0 };
      app.ticker.maxFPS = 60;
    } else {
      void actor.play("arrive").then(() => actor.play("wave"));
      setTimeout(() => void hello(), 900);
    }
  }
}

main().catch((e) => console.error(e));
