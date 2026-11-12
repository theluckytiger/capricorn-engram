// Her transmission panel: what she says (typed out), what she's doing (her steps, live), a box to type to her, and
// her "do it?" when a step is risky. It always closes: ✕, Esc, or by itself once nothing's going on.

import { log } from "../shared/log";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { inTauri } from "../shared/tauri";
import * as sounds from "./sounds";

export interface StepEvent {
  kind: "thinking" | "plan" | "started" | "finished";
  /** The job's plan, in a line. */
  text?: string;
  model?: string;
  tool?: string;
  args?: Record<string, unknown>;
  status?: "ok" | "failed" | "unsure";
  summary?: string;
  /** Where on screen a click landed (screen px), and what it was. */
  point?: [number, number];
  target?: { rect: [number, number, number, number]; label?: string };
}

/** What her panel shows, to show on the screen she's gone to. */
export type PanelState = { line: string; you: string | null; mode: "listening" | "awaiting" | "busy" | "said"; speaking: boolean };

export class Panel {
  readonly el: HTMLDivElement;
  private text: HTMLDivElement;
  /** Her portrait (drawn by portrait.ts). */
  readonly face: HTMLCanvasElement;
  private you: HTMLDivElement;
  private steps: HTMLOListElement;
  private confirmBox: HTMLDivElement;
  private form: HTMLFormElement;
  private input: HTMLInputElement;
  private timer = 0;
  private typing = 0;
  private busy = false;
  private line = "";
  /** A question with buttons still up: anything else she shows takes it down (you answered by voice). */
  private choice: (() => void) | null = null;
  onChange: () => void = () => {};
  /** A line of hers is going up (her portrait picks its face from it). */
  onLine: (line: string) => void = () => {};
  onAsk: (text: string) => void = () => {};
  onConfirm: (id: number, yes: boolean) => void = () => {};
  /** The panel was closed (✕, Esc, by itself, …; `why` goes in her journal): she stops talking and listening. */
  onClose: (why: string) => void = () => {};

  constructor(parent: HTMLElement) {
    this.el = document.createElement("div");
    this.el.className = "panel";
    this.el.hidden = true;
    this.el.innerHTML = `
      <header><span class="mic" title="Her mic"></span><span class="who">CAPRI</span><span class="sig" title="The brain on this request">// LINK</span><button class="x" title="Close (Esc)" aria-label="Close">✕</button></header>
      <div class="body"><canvas class="face" width="128" height="128"></canvas><div class="main">
      <div class="you" hidden></div>
      <div class="text"></div>
      <ol class="steps"></ol>
      <div class="confirm" hidden><span class="what"></span><div><button class="yes">Do it</button><button class="no">No</button><span class="say">or just say it</span></div></div>
      <form class="ask" hidden><input type="text" spellcheck="false" autocomplete="off" placeholder="Tell me what you want…" /></form>
      </div></div>`;
    this.face = this.el.querySelector("canvas.face")!;
    this.text = this.el.querySelector(".text")!;
    this.you = this.el.querySelector(".you")!;
    this.steps = this.el.querySelector(".steps")!;
    this.confirmBox = this.el.querySelector(".confirm")!;
    this.form = this.el.querySelector("form")!;
    this.input = this.el.querySelector("input")!;
    this.el.querySelector<HTMLButtonElement>(".x")!.onclick = () => this.close("✕ clicked");
    this.form.onsubmit = (e) => {
      e.preventDefault();
      const text = this.input.value.trim();
      if (!text || this.busy) return;
      this.input.value = "";
      this.busy = true;
      this.brain(null);
      this.steps.innerHTML = "";
      this.you.hidden = true;
      this.show("…", true);
      this.onAsk(text);
    };
    this.input.addEventListener("input", () => this.hold());
    window.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && !this.el.hidden) this.close("Esc");
    });
    parent.appendChild(this.el);
  }

  get open() {
    return !this.el.hidden;
  }

  /** A remark: typed out, gone after a read-length pause unless `stay`. */
  say(line: string, stay = false) {
    // A question waiting for its answer ("install Ollama?") isn't wiped by a remark of hers (her hello after the
    // intro took it away before it could be read): the remark is said once the question's answered.
    if (this.choice) {
      this.held = [line, stay];
      return;
    }
    log("panel", { op: "say", line });
    // A remark of her own isn't part of a job that's over: its steps go (a check-in showed under "open · Steam").
    if (!this.busy) this.steps.innerHTML = "";
    this.show(line, stay);
  }

  /** Showing a line that updates in place (`progress`). */
  private progressing = false;

  /** A line that keeps changing, like a download's progress: typed out once, then updated in place, silently. */
  progress(line: string) {
    // A question waiting for its answer stays; the progress shows again once it's answered.
    if (this.choice || !this.confirmBox.hidden) return;
    if (!this.el.hidden && this.progressing) {
      clearInterval(this.typing);
      this.line = line;
      this.text.textContent = line;
      this.onChange();
      return;
    }
    this.say(line, true);
    this.progressing = true;
  }

  /** Opens the box to type to her. */
  openInput() {
    log("panel", { op: "typing box" });
    this.form.hidden = false;
    if (this.el.hidden) this.show("Talk.", true);
    this.hold();
    if (inTauri) void getCurrentWindow().setFocus();
    setTimeout(() => this.input.focus(), 30);
    this.onChange();
  }

  /** Her name was heard (or the talk key): she's listening for what you want. Her ears say when they stop
   *  (a request, or nothing came), so the panel only has a far-off limit of its own. */
  listening() {
    log("panel", { op: "listening" });
    this.you.hidden = true;
    this.steps.innerHTML = "";
    this.confirmBox.hidden = true;
    this.busy = false;
    this.el.classList.remove("awaiting");
    this.el.classList.add("listening");
    this.show("Listening…", true);
    this.hold(30000);
  }

  /** She asked you something and is listening for the answer: her question stays, her mic light blinks. */
  awaiting() {
    this.el.classList.add("awaiting");
    this.hold(30000);
  }

  /** What she heard you say; she's working on it. */
  heard(text: string) {
    this.el.classList.remove("listening", "awaiting");
    this.brain(null);
    this.busy = true;
    this.steps.innerHTML = "";
    this.you.textContent = `“${text}”`;
    this.you.hidden = false;
    this.show("…", true);
  }

  /** She stopped listening and nothing came. */
  nothing() {
    if (this.el.classList.contains("awaiting")) {
      // No answer to her question: the question just goes, no remark.
      this.el.classList.remove("awaiting");
      this.hold(4000);
      return;
    }
    this.el.classList.remove("listening");
    if (!this.busy) this.show("Nothing? Fine.", false);
  }

  /** The mic light: your voice reaching her. */
  voice(on: boolean) {
    this.el.classList.toggle("voice", on);
  }

  /** Her answer to what you typed: the box stays a while for a follow-up. `model`: the brain it came from. */
  answer(line: string, model?: string | null) {
    log("panel", { op: "answer", line, model });
    if (model) this.brain(model);
    this.busy = false;
    this.el.classList.remove("listening", "awaiting");
    this.show(line, true);
    this.hold(25000);
  }

  /** Which brain is working on this request (a step started with it, or her answer came from it); none: LINK. */
  brain(model?: string | null) {
    log("panel", { op: "brain", model });
    const sig = this.el.querySelector<HTMLSpanElement>(".sig")!;
    sig.textContent = `// ${model ? brainName(model) : "LINK"}`;
    sig.classList.toggle("online", !!model && !model.startsWith("ollama/") && !model.startsWith("local"));
  }

  step(ev: StepEvent) {
    if (ev.kind === "thinking" && ev.model) this.brain(ev.model);
    if (ev.kind === "plan" && ev.text) {
      // Her plan for the job, over its steps (a new one replaces it).
      let plan = this.steps.querySelector<HTMLLIElement>("li.plan");
      if (!plan) {
        plan = document.createElement("li");
        plan.className = "plan";
        this.steps.prepend(plan);
      }
      plan.textContent = ev.text;
    } else if (ev.kind === "started" && ev.tool) {
      const li = document.createElement("li");
      li.className = "run";
      li.textContent = label(ev.tool, ev.args ?? {});
      this.steps.appendChild(li);
    } else if (ev.kind === "finished") {
      const li = [...this.steps.querySelectorAll<HTMLLIElement>("li.run")].pop();
      if (li) {
        li.className = ev.status ?? "ok";
        li.title = ev.summary ?? "";
      }
    }
    this.onChange();
  }

  confirm(id: number, what: string) {
    log("panel", { op: "asks yes or no", what });
    this.el.hidden = false;
    this.confirmBox.hidden = false;
    this.confirmBox.querySelector(".what")!.textContent = `Do this? ${what}`;
    const done = (yes: boolean) => {
      this.confirmBox.hidden = true;
      this.onConfirm(id, yes);
      this.onChange();
    };
    this.confirmBox.querySelector<HTMLButtonElement>(".yes")!.onclick = () => done(true);
    this.confirmBox.querySelector<HTMLButtonElement>(".no")!.onclick = () => done(false);
    clearTimeout(this.timer);
    this.onChange();
  }

  /** Her "do it?" was answered by voice or typing: the buttons go. */
  confirmAnswered() {
    this.confirmBox.hidden = true;
    this.onChange();
  }

  /** A remark that came while a question was waiting. */
  private held: [string, boolean] | null = null;
  /** The question being asked, if any: the next one waits for it. */
  private asking: Promise<unknown> = Promise.resolve();

  /** A question with two answers (her buttons). Resolves with the one clicked, or null after `ms` / on close. A second
   *  question waits for the first to be answered instead of taking its place. */
  choose(question: string, first: string, second: string, ms = 20000): Promise<0 | 1 | null> {
    const turn = this.asking.then(() => this.ask(question, first, second, ms));
    this.asking = turn;
    return turn;
  }

  private ask(question: string, first: string, second: string, ms: number): Promise<0 | 1 | null> {
    log("panel", { op: "asks which", question, first, second });
    return new Promise((resolve) => {
      this.el.hidden = false;
      this.show(question, true);
      this.confirmBox.hidden = false;
      this.confirmBox.querySelector(".what")!.textContent = "";
      const yes = this.confirmBox.querySelector<HTMLButtonElement>(".yes")!;
      const no = this.confirmBox.querySelector<HTMLButtonElement>(".no")!;
      const [yesText, noText] = [yes.textContent, no.textContent];
      yes.textContent = first;
      no.textContent = second;
      let done = false;
      // `replay`: answered or timed out, the remark held back meanwhile is said now. Taken over by an answer of hers
      // (`show`), it's dropped: what replaced the question is newer.
      const finish = (v: 0 | 1 | null, replay = true) => {
        if (done) return;
        done = true;
        this.choice = null;
        clearTimeout(t);
        yes.textContent = yesText;
        no.textContent = noText;
        this.confirmBox.hidden = true;
        this.onChange();
        const held = this.held;
        this.held = null;
        if (replay && held) this.say(...held);
        resolve(v);
      };
      const t = window.setTimeout(() => finish(null), ms);
      this.choice = () => finish(null, false);
      yes.onclick = () => finish(0);
      no.onclick = () => finish(1);
      this.onChange();
    });
  }

  /** She's talking: the panel stays up until she's done. */
  speaking(on: boolean) {
    this.el.classList.toggle("speaking", on);
    if (on) clearTimeout(this.timer);
    // Cut off by her name: she's listening now, and her ears say when that ends.
    else if (this.el.classList.contains("listening") || this.el.classList.contains("awaiting")) return;
    else if (!this.form.hidden) this.hold(20000);
    else this.hold(6000);
  }

  /** What it's showing, to go with her to another screen; null when it's shut, a box you're typing in, or a question
   *  with buttons (those stay). */
  snapshot(): PanelState | null {
    if (this.el.hidden || !this.form.hidden || !this.confirmBox.hidden) return null;
    const c = this.el.classList;
    const mode = c.contains("listening") ? "listening" : c.contains("awaiting") ? "awaiting" : this.busy ? "busy" : "said";
    return { line: this.line, you: this.you.hidden ? null : this.you.textContent, mode, speaking: c.contains("speaking") };
  }

  /** Shows what her panel showed on the screen she came from, as it was (not typed out again). */
  restore(s: PanelState) {
    clearTimeout(this.timer);
    clearInterval(this.typing);
    this.el.hidden = false;
    this.line = s.line;
    this.text.textContent = s.line;
    this.onLine(s.line);
    this.you.hidden = s.you === null;
    this.you.textContent = s.you ?? "";
    this.steps.innerHTML = "";
    this.el.classList.toggle("listening", s.mode === "listening");
    this.el.classList.toggle("awaiting", s.mode === "awaiting");
    this.el.classList.toggle("speaking", s.speaking);
    this.busy = s.mode === "busy";
    if (s.mode === "listening" || s.mode === "awaiting") this.hold(30000);
    else if (s.mode === "said" && !s.speaking) this.hold(6000);
    this.onChange();
  }

  /** She took it to another screen: hidden here, without stopping her talking or listening. */
  hide() {
    clearTimeout(this.timer);
    clearInterval(this.typing);
    this.el.hidden = true;
    this.form.hidden = true;
    this.confirmBox.hidden = true;
    this.steps.innerHTML = "";
    this.you.hidden = true;
    this.el.classList.remove("listening", "awaiting", "voice", "speaking");
    this.busy = false;
    this.onChange();
  }

  close(why = "closed") {
    log("panel", { op: "closed", why });
    this.onClose(why);
    clearTimeout(this.timer);
    clearInterval(this.typing);
    this.el.hidden = true;
    this.form.hidden = true;
    this.confirmBox.hidden = true;
    this.steps.innerHTML = "";
    this.you.hidden = true;
    this.el.classList.remove("listening", "awaiting", "voice");
    this.busy = false;
    this.onChange();
  }

  /** A long text in a wide panel that scrolls (Claude's answer while you were on another window): no typing out,
   *  up a minute and a half or until closed. */
  long(title: string, text: string) {
    log("panel", { op: "long answer", title, text });
    this.show(title, true);
    clearInterval(this.typing);
    this.el.classList.add("wide");
    this.text.textContent = "";
    const head = document.createElement("div");
    head.className = "long-title";
    head.textContent = title;
    const body = document.createElement("div");
    body.className = "long-body";
    body.textContent = text;
    this.text.append(head, body);
    this.hold(90_000);
    this.onChange();
  }

  private show(line: string, stay: boolean) {
    this.progressing = false;
    this.choice?.();
    this.el.classList.remove("wide");
    clearTimeout(this.timer);
    clearInterval(this.typing);
    this.el.hidden = false;
    this.text.textContent = "";
    this.line = line;
    this.onLine(line);
    let i = 0;
    this.typing = window.setInterval(() => {
      i = Math.min(line.length, i + 2);
      this.text.textContent = line.slice(0, i);
      // Her chatter as the line types out (not over spaces).
      if (line[i - 1]?.trim()) sounds.blip();
      if (i >= line.length) clearInterval(this.typing);
      this.onChange();
    }, 24);
    if (!stay) this.timer = window.setTimeout(() => this.close("read and gone"), 4000 + line.length * 60);
    this.onChange();
  }

  /** Keeps the panel up while you're typing; closes it after `ms` of nothing. */
  private hold(ms = 20000) {
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => {
      if (!this.busy && this.confirmBox.hidden) this.close("nothing going on");
    }, ms);
  }

  /** Beside her, kept on screen: to the left of `x` (or to its right, for her mask in the bottom-left corner). */
  place(x: number, y: number, right = false) {
    const w = this.el.offsetWidth || 360, h = this.el.offsetHeight || 80;
    const left = Math.max(12, Math.min(innerWidth - w - 12, right ? x + 24 : x - w - 24));
    const top = Math.max(12, Math.min(innerHeight - h - 12, y - h / 2));
    this.el.style.transform = `translate(${left}px, ${top}px)`;
  }

  rect(): [number, number][] | null {
    if (this.el.hidden) return null;
    const r = this.el.getBoundingClientRect();
    return [[r.left, r.top], [r.right, r.top], [r.right, r.bottom], [r.left, r.bottom]];
  }
}

/** "windows {action: minimize, target: Brave}" → "minimize · Brave" */
function label(tool: string, args: Record<string, unknown>) {
  const bits = [args.action, args.target ?? args.name ?? args.place ?? args.query ?? args.keys ?? args.text].filter((v) => v !== undefined && v !== "");
  const tail = bits.map((b) => String(b)).join(" · ");
  return tail ? `${tool} · ${tail}` : tool;
}

/** A brain's id as her panel says it: "claudecode/sonnet" → "CLAUDE SONNET · PLAN", "ollama/qwen3-vl:8b-instruct" →
 *  "QWEN3-VL 8B · LOCAL". */
export function brainName(id: string): string {
  const [maker, ...rest] = id.split("/");
  const model = (rest.pop() ?? maker).toLowerCase();
  const where: Record<string, string> = { ollama: "LOCAL", claudecode: "PLAN", cursor: "PLAN", codex: "PLAN", nvidia: "NVIDIA", groq: "GROQ", gemini: "GOOGLE", anthropic: "API", openai: "OPENAI", openrouter: "OPENROUTER", deepseek: "DEEPSEEK", mistral: "MISTRAL", ollamacloud: "OLLAMA CLOUD", lmstudio: "LOCAL", llamacpp: "LOCAL", vllm: "LOCAL" };
  const name =
    maker === "claudecode" ? `CLAUDE ${model}` :
    maker === "cursor" ? (model.startsWith("composer") ? model.replace(/-/g, " ") : `CURSOR ${model.replace(/-/g, " ")}`) :
    maker === "codex" ? `CODEX ${model}` :
    model.includes("nemotron") ? "NEMOTRON" :
    model.replace(/:/g, " ").replace(/-(instruct|latest|preview|a\d+b)\b/g, "").replace(/-/g, " ").replace(/\s+/g, " ").trim();
  return `${name.toUpperCase()} · ${where[maker] ?? maker.toUpperCase()}`;
}
