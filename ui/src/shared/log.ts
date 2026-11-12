// Everything that happens in her windows, into her journal, when she's being tested (started with
// CAPRI_LOG_EVERYTHING=1): every click and setting touched, what the page asked her core and what came back, what she
// acted out and showed, and every error. Off for anyone else: nothing is listened to, nothing is sent.
// Read it back with `python tools/journal.py --session --all`.

import { invoke as raw } from "@tauri-apps/api/core";

type Entry = Record<string, unknown>;

let page = "page";
let state: "asking" | "on" | "off" = "asking";
let queue: Entry[] = [];
let timer = 0;

/** A key or a password never goes in a log. */
const SECRET = /key|token|secret|pass/i;
/** Asked many times a second or minute: only counted, one line a minute. */
const QUIET_CALLS = new Set(["pointer_at", "set_hull", "alive", "idle_secs", "now_playing", "whereabouts", "perches", "beat_now", "screens_around", "taskbar_top"]);
const QUIET_EVENTS = new Set(["capri://beat", "capri://cursor", "capri://caret"]);
const QUIET_HEARD = new Set(["level", "voice", "mic"]);
const counts: Record<string, number> = {};

function flush() {
  timer = 0;
  if (!queue.length) return;
  const events = queue;
  queue = [];
  void raw("log_ui", { page, events }).catch(() => {});
}

/** One thing that happened: `ev` says what kind of thing. */
export function log(ev: string, data: Entry = {}) {
  if (state === "off") return;
  if (queue.length > 3000) queue.shift();
  queue.push({ ev, ...data });
  if (state === "on" && !timer) timer = window.setTimeout(flush, 200);
}

function clip(v: unknown, n = 400): string {
  let s: string;
  try {
    s = typeof v === "string" ? v : v instanceof Error ? `${v.name}: ${v.message}` : (JSON.stringify(v) ?? String(v));
  } catch {
    s = String(v);
  }
  return s.length > n ? `${s.slice(0, n)}…` : s;
}

function redact(args?: Record<string, unknown>) {
  if (!args) return undefined;
  return Object.fromEntries(Object.entries(args).map(([k, v]) => [k, SECRET.test(k) ? "(hidden)" : v]));
}

/** A call the page made to her core, how long it took and what came back. */
export function noteCall(cmd: string, args: Record<string, unknown> | undefined, ms: number, result: unknown, error?: unknown) {
  if (state === "off" || cmd === "log_ui" || cmd === "log_everything") return;
  if (QUIET_CALLS.has(cmd) && error === undefined) {
    counts[cmd] = (counts[cmd] ?? 0) + 1;
    return;
  }
  log("ipc", { cmd, args: clip(redact(args), 600), ms: Math.round(ms), ...(error === undefined ? { result: clip(result, cmd === "get_settings" ? 120 : 500) } : { failed: clip(error, 800) }) });
}

/** Something her core told the page. Sent to every window, it's written once (by the first screen's page). */
export function noteEvent(event: string, payload: unknown, toAll: boolean) {
  if (state === "off" || QUIET_EVENTS.has(event)) return;
  if (toAll && page.startsWith("overlay") && page !== "overlay0") return;
  if (event === "capri://heard" && QUIET_HEARD.has((payload as { kind?: string })?.kind ?? "")) return;
  log("event", { event, payload: clip(payload, 500) });
}

/** What an element is, in words someone reading the log can find on screen. */
function describe(target: EventTarget | null): Entry {
  if (!(target instanceof Element)) return { on: "window" };
  const el = target.closest<HTMLElement>("button, a, label, select, input, textarea, canvas, [role=button], [data-id], [data-page]") ?? (target as HTMLElement);
  const tag = el.tagName.toLowerCase();
  const out: Entry = { on: `${tag}${el.id ? `#${el.id}` : ""}${typeof el.className === "string" && el.className ? `.${el.className.trim().split(/\s+/).slice(0, 3).join(".")}` : ""}` };
  const text = (el.textContent ?? "").trim().replace(/\s+/g, " ");
  if (text && !["input", "select", "textarea"].includes(tag)) out.text = text.slice(0, 90);
  if (el.dataset.id !== undefined) out.card = el.dataset.id;
  const title = el.getAttribute("title") ?? el.getAttribute("aria-label");
  if (title) out.title = title;
  return out;
}

/** The words next to a field: its label, or what's written before it in its row. */
function fieldName(el: HTMLElement): string {
  const label = el.closest("label")?.textContent ?? el.previousElementSibling?.textContent ?? el.parentElement?.querySelector("label, .hint, span, strong")?.textContent ?? (el as HTMLInputElement).placeholder ?? "";
  return label.trim().replace(/\s+/g, " ").slice(0, 80);
}

function install() {
  const hear = (type: string, make: (e: Event) => Entry | null) =>
    window.addEventListener(
      type,
      (e) => {
        const entry = make(e);
        if (entry) log(type, entry);
      },
      true,
    );
  const at = (e: Event) => ({ x: Math.round((e as MouseEvent).clientX), y: Math.round((e as MouseEvent).clientY), ...describe(e.target) });
  hear("click", at);
  hear("dblclick", at);
  hear("contextmenu", at);
  // On her body only: a press there starts a drag or a pat (a press anywhere else is its click).
  hear("pointerdown", (e) => (e.target instanceof HTMLCanvasElement ? at(e) : null));
  hear("change", (e) => {
    const el = e.target as HTMLInputElement;
    if (!el?.tagName) return null;
    const hidden = el.type === "password" || (el.type !== "checkbox" && SECRET.test(`${el.id} ${el.name} ${el.placeholder}`));
    const value = el.type === "checkbox" || el.type === "radio" ? el.checked : hidden ? "(hidden)" : el.value;
    return { ...describe(el), field: fieldName(el), value };
  });
  hear("keydown", (e) => {
    const k = e as KeyboardEvent;
    // Keys that do something; what's typed into a field shows up when it changes.
    if (k.key.length === 1 && !k.ctrlKey && !k.altKey && !k.metaKey) return null;
    if (["Shift", "Control", "Alt", "Meta", "Backspace", "Delete"].includes(k.key)) return null;
    return { key: `${k.ctrlKey ? "Ctrl+" : ""}${k.altKey ? "Alt+" : ""}${k.shiftKey ? "Shift+" : ""}${k.metaKey ? "Win+" : ""}${k.key}`, ...describe(e.target) };
  });
  window.addEventListener("focus", () => log("page", { state: "focused" }));
  window.addEventListener("blur", () => log("page", { state: "lost focus" }));
  window.addEventListener("pagehide", () => {
    log("page", { state: "closing" });
    flush();
  });
  // Errors: the page's own, a promise nobody caught, a file that didn't load.
  window.addEventListener(
    "error",
    (e) => {
      if (e.target && e.target !== window) log("error", { source: "load", what: clip((e.target as HTMLImageElement).src ?? String(e.target)) });
      else log("error", { source: "script", message: e.message, where: `${e.filename}:${e.lineno}:${e.colno}`, stack: clip((e.error as Error)?.stack, 1500) });
    },
    true,
  );
  window.addEventListener("unhandledrejection", (e) => log("error", { source: "promise", message: clip(e.reason, 800), stack: clip((e.reason as Error)?.stack, 1500) }));
  for (const level of ["error", "warn", "info", "log", "debug"] as const) {
    const real = console[level].bind(console);
    console[level] = (...args: unknown[]) => {
      real(...args);
      log("console", { level, text: args.map((a) => clip(a, 600)).join(" ").slice(0, 1200) });
    };
  }
  window.setInterval(() => {
    if (!Object.keys(counts).length) return;
    log("ipc_counts", { ...counts });
    for (const k of Object.keys(counts)) delete counts[k];
  }, 60000);
  log("page", { state: "opened", url: location.pathname + location.search, size: `${innerWidth}x${innerHeight}` });
}

/** Call once as a page starts, with the page's name. What happens before her core has answered waits in the queue. */
export function startLog(name: string) {
  page = name;
  if (!("__TAURI_INTERNALS__" in window)) {
    state = "off";
    return;
  }
  raw<boolean>("log_everything")
    .then((on) => {
      state = on ? "on" : "off";
      if (on) {
        install();
        timer = window.setTimeout(flush, 200);
      } else queue = [];
    })
    .catch(() => {
      state = "off";
      queue = [];
    });
}
