// Her settings window: every page reads her settings, and each change is saved as you make it. Voice on/off and
// listening for her name apply at once; models, her voice itself and conversation mode on her next start (a bar
// offers to restart her).

import "./settings.css";
import { call as tauriInvoke, invoke, listen } from "../shared/tauri";
import { startLog } from "../shared/log";

interface Settings {
  name: string | null;
  blackwall: boolean;
  yours: { style: string; memories: string[]; lines: string[] };
  home: string | null;
  brain: { cloud: boolean; fast: string; smart: string | null; eyes: string | null; daily_cloud_requests: number; auto: boolean };
  ears: { wake_word: boolean; follow_up_secs: number };
  voice: { enabled: boolean; engine: string; speaker: number; speed: number; sounds: boolean };
  screens: { follow_pointer: boolean; locked_to: number | null; size: number };
  privacy: { screenshots_to_cloud: boolean; speak_in_calls: boolean; watch_coders: boolean };
  care: { on: boolean; water: number; stretch: number; eyes: number; food: boolean; bed_hour: number; game_break: number };
  games: Record<string, "quiet" | "reachable">;
  keys: Record<string, string>;
  phone: { on: boolean; discord_user: string };
}

/** What a plain browser shows (no core to ask): her defaults. */
const DEFAULTS: Settings = {
  name: null,
  blackwall: false,
  yours: { style: "", memories: [], lines: [] },
  home: null,
  brain: { cloud: false, fast: "ollama/qwen3-vl:8b-instruct", smart: null, eyes: "ollama/qwen3-vl:8b-instruct", daily_cloud_requests: 200, auto: true },
  ears: { wake_word: true, follow_up_secs: 8 },
  voice: { enabled: true, engine: "kokoro", speaker: 37, speed: 1, sounds: true },
  screens: { follow_pointer: false, locked_to: null, size: 1 },
  privacy: { screenshots_to_cloud: false, speak_in_calls: false, watch_coders: true },
  care: { on: true, water: 75, stretch: 100, eyes: 0, food: true, bed_hour: 2, game_break: 120 },
  games: {},
  keys: {},
  phone: { on: false, discord_user: "" },
};

/** Her keys: what each does, and its usual letter. */
const KEYS: [string, string, string][] = [
  ["talk", "Talk (hold), or have her listen (tap)", "V"],
  ["wake_word", "Listening for her name on / off", "W"],
  ["type", "Type to her", "T"],
  ["away", "Put her away / bring her back", "C"],
  ["mute", "Mute your mic for her", "N"],
  ["ride", "She rides your pointer", "R"],
];
const LETTERS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ".split("");

/** Kokoro's voices she can use (hers first). */
const KOKORO: [number, string][] = [
  [37, "H: Japanese accent (hers)"],
  [40, "Japanese, softer"],
  [10, "American, light"],
  [7, "American, bright"],
  [2, "American, warm"],
  [3, "American, clear"],
  [6, "American, breathy"],
  [5, "American, low"],
  [4, "American, crisp"],
  [23, "British"],
];

type Field =
  | { kind: "text"; label: string; hint?: string; get: (s: Settings) => string; set: (s: Settings, v: string) => void; redraw?: boolean }
  | { kind: "toggle"; label: string; hint?: string; get: (s: Settings) => boolean; set: (s: Settings, v: boolean) => void; redraw?: boolean }
  | { kind: "number"; label: string; hint?: string; min: number; max: number; step?: number; get: (s: Settings) => number; set: (s: Settings, v: number) => void }
  | { kind: "select"; label: string; hint?: string; options: () => [string, string][]; get: (s: Settings) => string; set: (s: Settings, v: string) => void; redraw?: boolean }
  | { kind: "lines"; label: string; hint?: string; placeholder?: string; get: (s: Settings) => string[]; set: (s: Settings, v: string[]) => void };

interface Page {
  name: string;
  intro: string;
  fields?: Field[];
  render?: (el: HTMLElement, s: Settings) => void;
}

let current: Settings = structuredClone(DEFAULTS);
let currentPage = 0;
startLog("settings");
let models: string[] = [];

/** The models on this PC for a role (Ollama's, and LM Studio's, llama.cpp's or vLLM's when one is running), with the
 *  one set now even if nothing lists it. */
const SERVERS: Record<string, string> = { lmstudio: "LM Studio", llamacpp: "llama.cpp", vllm: "vLLM", gemini: "Gemini", groq: "Groq" };
/** Cloud providers she can use as her online brain, free key or paid: what each is, and what it's good for. */
const ONLINE: { id: string; name: string; tag: "Free" | "Paid" | "Free & paid" | "Your plan"; line: string; rec?: string }[] = [
  { id: "claudecode", name: "Claude", tag: "Your plan", line: "Your Claude Pro or Max plan, through Claude Code. No key.", rec: "Works best" },
  { id: "nvidia", name: "NVIDIA", tag: "Free", line: "Fast, less reliable on long jobs. A key from build.nvidia.com.", rec: "Second" },
  { id: "groq", name: "Groq", tag: "Free", line: "About 1,000 requests a day, and very fast. Open models that are good with tools." },
  { id: "gemini", name: "Google Gemini", tag: "Free", line: "Sees pictures. The free tier is small: about 20 requests a day per model, a few hard jobs." },
  { id: "anthropic", name: "Claude API", tag: "Paid", line: "Claude with an API key, paid per use; she only calls it for what you choose below. (A Claude plan is the first card.)" },
  { id: "openai", name: "OpenAI", tag: "Paid", line: "A strong all-rounder that sees pictures. Pay per use." },
  { id: "ollamacloud", name: "Ollama cloud", tag: "Free", line: "Big open models on Ollama's servers, nothing to download. Sign in to Ollama once (in a terminal: ollama signin)." },
  { id: "codex", name: "Codex", tag: "Your plan", line: "Your ChatGPT plan, through Codex on this PC. No key. Install it and run codex login first." },
  { id: "cursor", name: "Cursor", tag: "Your plan", line: "Your Cursor plan. No key. Slow with screenshots." },
  { id: "openrouter", name: "OpenRouter", tag: "Free & paid", line: "One key for hundreds of models from every lab, some of them free." },
  { id: "deepseek", name: "DeepSeek", tag: "Paid", line: "Very cheap and good at reasoning. Doesn't see pictures." },
  { id: "mistral", name: "Mistral", tag: "Free & paid", line: "European, with a free tier to try it out." },
];
const modelOptions = (now: string | null, none?: string): [string, string][] => {
  const name = (m: string) => {
    const [p, rest] = [m.split("/")[0], m.slice(m.indexOf("/") + 1)];
    return p === "ollama" ? rest : SERVERS[p] ? `${SERVERS[p]} · ${rest}` : m;
  };
  const list = [...new Set([...(now ? [now] : []), ...models])].map((m): [string, string] => [m, name(m)]);
  return none ? [["", none], ...list] : list;
};
const orNull = (v: string) => (v.trim() ? v.trim() : null);

const PAGES: Page[] = [
  {
    name: "General",
    intro: "Where home is (for the weather and \"at home\").",
    fields: [
      { kind: "text", label: "Home", hint: "A city, with its country if it's ambiguous. You can also just tell her.", get: (s) => s.home ?? "", set: (s, v) => (s.home = orNull(v)) },
      { kind: "toggle", label: "Check in on me", hint: "Now and then while you're at the PC: water, a stretch, food, bed when it's late, a break from a long game. Never in a quiet game.", get: (s) => s.care.on, set: (s, v) => (s.care.on = v) },
      { kind: "number", label: "Bed nudge from (hour, 24 = off)", min: 0, max: 24, get: (s) => s.care.bed_hour, set: (s, v) => (s.care.bed_hour = v) },
      { kind: "number", label: "Game break after (minutes, 0 = off)", min: 0, max: 600, step: 15, get: (s) => s.care.game_break, set: (s, v) => (s.care.game_break = v) },
      { kind: "toggle", label: "Blackwall mode", hint: "She stops asking before risky steps (sending, deleting, running programs). Money, wiping data, Windows security and code from the internet still always ask. Or say \"blackwall on\".", get: (s) => s.blackwall, set: (s, v) => (s.blackwall = v) },
    ],
  },
  {
    name: "Make her yours",
    intro: "Who you are to her. You can also just tell her: \"call me…\", \"remember that…\".",
    render: (el) => {
      el.insertAdjacentHTML("afterbegin", `<p class="hint" id="bond"></p>`);
      void invoke<{ stage: number; name: string; days: number }>("bond").then((b) => {
        if (b) el.querySelector("#bond")!.textContent = `Your bond: ${b.name} (${b.stage + 1} of 5), ${b.days} day${b.days === 1 ? "" : "s"} together. It grows with days, chats, jobs done, reminders kept and head pats.`;
      });
    },
    fields: [
      { kind: "text", label: "What she calls you", hint: "Empty: she picks a nickname now and then.", get: (s) => s.name ?? "", set: (s, v) => (s.name = orNull(v)) },
      { kind: "text", label: "How she is with you", hint: "In your words: \"meaner\", \"more Night City slang\", \"calls my PC 'the rig'\".", get: (s) => s.yours.style, set: (s, v) => (s.yours.style = v.trim()) },
      { kind: "lines", label: "Things she remembers", hint: "One per line, in your words. She brings them up when they fit.", placeholder: "I hate mornings\nMy sister is called Ana", get: (s) => s.yours.memories, set: (s, v) => (s.yours.memories = v) },
      { kind: "lines", label: "Lines of yours", hint: "Things she says now and then, one per line.", placeholder: "Wake up, samurai.", get: (s) => s.yours.lines, set: (s, v) => (s.yours.lines = v) },
    ],
  },
  {
    name: "Brain",
    intro: "The models she thinks and sees with. Local ones run on your graphics card through Ollama; online ones need an API key from their provider.",
    fields: [
      { kind: "toggle", label: "Pick for this PC", hint: "She picks the local model that fits your graphics card (or memory) at every start. Choosing one below turns this off.", get: (s) => s.brain.auto, set: (s, v) => (s.brain.auto = v) },
      { kind: "select", label: "Thinking (fast)", hint: "Every request goes here first.", redraw: true, options: () => modelOptions(current.brain.fast), get: (s) => s.brain.fast, set: (s, v) => (s.brain.fast = v) },
      {
        kind: "text",
        label: "Or any model",
        hint: "Its id: lmstudio/…, llamacpp/…, vllm/…, a cloud one (openai/…, groq/…: its key below), or any OpenAI-compatible server as http://host:port/v1#model.",
        redraw: true,
        get: (s) => s.brain.fast,
        set: (s, v) => {
          if (v.trim()) s.brain.fast = v.trim();
        },
      },
      { kind: "select", label: "Eyes", hint: "Looking at windows to find what to click: a vision model.", options: () => modelOptions(current.brain.eyes, "Off"), get: (s) => s.brain.eyes ?? "", set: (s, v) => (s.brain.eyes = orNull(v)) },
      { kind: "text", label: "Smart (hard jobs)", hint: "A stronger model that plans jobs inside windows or of several steps, and takes over when the fast one gets stuck: the Online brain above sets it, or any id (gemini/…, groq/…, anthropic/…). Empty: none.", get: (s) => s.brain.smart ?? "", set: (s, v) => (s.brain.smart = orNull(v)) },
      { kind: "toggle", label: "Use the cloud", hint: "Off: nothing about you leaves your PC.", get: (s) => s.brain.cloud, set: (s, v) => (s.brain.cloud = v) },
      { kind: "number", label: "Cloud requests a day", min: 0, max: 5000, step: 10, get: (s) => s.brain.daily_cloud_requests, set: (s, v) => (s.brain.daily_cloud_requests = v) },
    ],
    render: (el) => {
      el.insertAdjacentHTML("afterbegin", `<h2>On this PC</h2><div class="actions"><span class="hint" id="brainstate">Checking her brain…</span><button id="getbrain" hidden></button></div><p class="hint" id="brainpick"></p>`);
      onlineBrain(el);
      void invoke<{ model: string; why: string }>("recommended_brain").then((r) => {
        if (r) el.querySelector("#brainpick")!.textContent = `For this PC: ${r.model.replace("ollama/", "")} (${r.why}).`;
      });
      const state = el.querySelector<HTMLSpanElement>("#brainstate")!;
      const button = el.querySelector<HTMLButtonElement>("#getbrain")!;
      void invoke<{ local: boolean; installed: boolean; running: boolean; model: string; has_model: boolean }>("brain_status").then((b) => {
        if (!b) return (state.textContent = "");
        if (!b.local) return (state.textContent = "Her brain is a cloud model.");
        state.textContent = !b.installed ? "Ollama (her local brain) isn't installed." : !b.running ? "Ollama is installed but not running." : b.has_model ? `Ollama is running with ${b.model}. All set.` : `Ollama is running, but ${b.model} isn't downloaded yet (${b.model.includes(":2b") ? "about 2 GB" : b.model.includes(":4b") ? "about 3 GB" : "about 6 GB"}).`;
        const todo = !b.installed ? "Install Ollama" : !b.running ? "Start Ollama" : !b.has_model ? "Download her model" : "";
        button.hidden = !todo;
        button.textContent = todo;
        button.onclick = () => {
          button.disabled = true;
          state.textContent = "On it. She'll say how it's going.";
          void invoke("install_ollama");
        };
      });
      // Cloud keys: pasted, checked, saved encrypted by Windows; never shown again.
      el.insertAdjacentHTML(
        "beforeend",
        `<h2>Saved keys</h2><p class="hint">Every key she has: the Online brain above saves its own here. Checked with a real call before it's saved, encrypted in Windows' Credential Manager, never in her files or logs. A key here also lets you use any of that provider's models by id (Smart, Or any model).</p>
         <div class="actions"><select id="kprov"></select><input id="kval" type="password" placeholder="Paste a key" autocomplete="off" spellcheck="false" /><button id="ksave">Check and save</button></div>
         <p class="hint" id="kmsg"></p><ul class="keys" id="klist"></ul>`,
      );
      const prov = el.querySelector<HTMLSelectElement>("#kprov")!;
      const val = el.querySelector<HTMLInputElement>("#kval")!;
      const msg = el.querySelector<HTMLParagraphElement>("#kmsg")!;
      const list = el.querySelector<HTMLUListElement>("#klist")!;
      const refresh = async () => {
        const keys = (await invoke<{ id: string; name: string; saved: boolean; env: boolean; env_var: string }[]>("keys_status")) ?? [];
        if (!prov.options.length) for (const k of keys) prov.add(new Option(k.name, k.id));
        list.innerHTML = "";
        for (const k of keys.filter((k) => k.saved || k.env)) {
          const li = document.createElement("li");
          li.textContent = `${k.name}: ${k.saved ? "saved" : `from ${k.env_var}`} `;
          if (k.saved) {
            const rm = document.createElement("button");
            rm.className = "ghost";
            rm.textContent = "Remove";
            rm.onclick = () => void invoke("delete_key", { provider: k.id }).then(refresh);
            li.appendChild(rm);
          }
          list.appendChild(li);
        }
      };
      el.querySelector<HTMLButtonElement>("#ksave")!.onclick = async () => {
        if (!val.value.trim()) return;
        msg.textContent = "Checking…";
        try {
          // Her own invoke, not the quiet one: a key that doesn't work should say why.
          msg.textContent = await tauriInvoke<string>("save_key", { provider: prov.value, key: val.value });
          val.value = "";
          void refresh();
        } catch (e) {
          msg.textContent = String(e);
        }
      };
      void refresh();
    },
  },
  {
    name: "Voice",
    intro: "How she sounds.",
    fields: [
      { kind: "toggle", label: "Talk out loud", hint: "Off: she only writes.", get: (s) => s.voice.enabled, set: (s, v) => (s.voice.enabled = v) },
      { kind: "toggle", label: "Sounds", hint: "Her sound effects: the whoosh when you throw her, a hit, a landing.", get: (s) => s.voice.sounds, set: (s, v) => (s.voice.sounds = v) },
      { kind: "select", label: "Voice engine", redraw: true, options: () => [["kokoro", "Kokoro"], ["supertonic", "Supertonic"], ["kitten", "Kitten"]], get: (s) => s.voice.engine, set: (s, v) => (s.voice.engine = v) },
      {
        kind: "select",
        label: "Her voice",
        hint: "Kokoro's voices; with another engine, its speaker number.",
        options: () => (current.voice.engine === "kokoro" ? KOKORO.map(([n, l]): [string, string] => [String(n), l]) : Array.from({ length: 10 }, (_, i): [string, string] => [String(i), `Speaker ${i}`])),
        get: (s) => String(s.voice.speaker),
        set: (s, v) => (s.voice.speaker = Number(v)),
      },
      { kind: "number", label: "Speed", min: 0.7, max: 1.4, step: 0.05, get: (s) => s.voice.speed, set: (s, v) => (s.voice.speed = v) },
    ],
  },
  {
    name: "Keys",
    intro: "Her keys are Ctrl+Alt plus a letter. Pick another letter if one clashes with an app of yours; it changes at once.",
    fields: KEYS.map(([id, label, usual]) => ({
      kind: "select" as const,
      label,
      hint: `Usually Ctrl+Alt+${usual}.`,
      options: () => LETTERS.map((l): [string, string] => [l, `Ctrl+Alt+${l}`]),
      get: (s: Settings) => s.keys[id] ?? usual,
      set: (s: Settings, v: string) => {
        if (v === usual) delete s.keys[id];
        else s.keys[id] = v;
      },
    })),
  },
  {
    name: "Ears",
    intro: "How she hears you. Ctrl+Alt+V: hold to talk, tap to have her listen. Ctrl+Alt+W: her name on or off.",
    fields: [
      { kind: "toggle", label: "Listen for her name", hint: "\"Capri\" or \"Capricorn\". Nothing is recorded or kept.", get: (s) => s.ears.wake_word, set: (s, v) => (s.ears.wake_word = v) },
      { kind: "number", label: "Conversation mode (seconds)", hint: "After she answers, she keeps listening this long without her name. 0: off (she still listens after a question of hers).", min: 0, max: 20, get: (s) => s.ears.follow_up_secs, set: (s, v) => (s.ears.follow_up_secs = v) },
    ],
  },
  {
    name: "Screens",
    intro: "How big she is, and whether she follows you from screen to screen.",
    fields: [
      { kind: "toggle", label: "Follow my pointer", hint: "When your pointer's been on another screen for five seconds, she goes there too.", get: (s) => s.screens.follow_pointer, set: (s, v) => (s.screens.follow_pointer = v) },
      {
        kind: "select",
        label: "Size",
        hint: "Changes at once.",
        options: () => [["0.5", "Tiny"], ["0.75", "Small"], ["1", "Her usual"], ["1.35", "Big"], ["1.8", "Giant"]],
        get: (s) => String(s.screens.size ?? 1),
        set: (s, v) => (s.screens.size = Number(v)),
      },
    ],
  },
  {
    name: "Games",
    intro: "Games she has seen, and how she is while you play: quiet (ears off too) or reachable (\"Capri\" still works, only the cloud thinks).",
    render: (el, s) => {
      const names = Object.keys(s.games).sort();
      if (!names.length) {
        el.insertAdjacentHTML("beforeend", `<p class="hint">None yet: she asks the first time a game starts.</p>`);
        return;
      }
      for (const exe of names) {
        const row = document.createElement("div");
        row.className = "row";
        row.innerHTML = `<label></label><div class="control"><select><option value="quiet">Quiet</option><option value="reachable">Reachable</option></select><button class="ghost">Forget</button></div>`;
        row.querySelector("label")!.textContent = exe.replace(/^game:/, "");
        const sel = row.querySelector("select")!;
        sel.value = s.games[exe];
        sel.onchange = () => {
          s.games[exe] = sel.value as "quiet" | "reachable";
          save();
        };
        row.querySelector("button")!.onclick = () => {
          delete s.games[exe];
          save();
          show(currentPage);
        };
        el.appendChild(row);
      }
    },
  },
  {
    name: "Coding AIs",
    intro: "She keeps an eye on coding agents while they work on this PC: what Claude Code is doing floats over her head, she cheers when it's done, and she comes to tell you when it's waiting on you.",
    fields: [
      { kind: "toggle", label: "Watch coding AIs", hint: "She follows Claude Code's transcripts, read on this PC only.", get: (s) => s.privacy.watch_coders, set: (s, v) => (s.privacy.watch_coders = v) },
    ],
    render: (el) => {
      el.insertAdjacentHTML(
        "beforeend",
        `<p class="hint">Claude Code doesn't write it down when it's waiting for your permission. Connected, it calls her (on this PC only) and she comes to tell you. This adds one line to Claude Code's own settings; a copy of them is kept first.</p>
         <div class="actions"><button id="claudehook">Let Claude Code call me</button></div><p class="hint" id="claudehooked"></p>`,
      );
      el.querySelector<HTMLButtonElement>("#claudehook")!.onclick = () =>
        void invoke<string>("connect_claude").then((m) => (el.querySelector("#claudehooked")!.textContent = m ?? "Couldn't connect: see her logs."));
    },
  },
  {
    name: "Phone",
    intro: "Message her from Discord when you're away from the PC. She does what you ask on the PC and answers in the chat, \"screenshot\" gets you a picture of the screen, and \"stop\" stops her. Only you: she reads only your own messages to your own bot, and anything risky asks you for a one-time code first.",
    fields: [
      { kind: "toggle", label: "Talk to me on Discord", hint: "Needs your user ID and your bot's token below.", get: (s) => s.phone.on, set: (s, v) => (s.phone.on = v) },
      { kind: "text", label: "Your Discord user ID", hint: "In Discord: Settings › Advanced › Developer Mode on, then right-click your name › Copy User ID.", get: (s) => s.phone.discord_user, set: (s, v) => (s.phone.discord_user = v.replace(/\D/g, "")) },
    ],
    render: (el) => {
      el.insertAdjacentHTML(
        "beforeend",
        `<h2>Her bot</h2>
         <ol class="steps">
           <li>On <span class="mono">discord.com/developers/applications</span>: New Application, give it her name, then Bot › Reset Token and copy it.</li>
           <li>OAuth2 › URL Generator: tick <span class="mono">bot</span>, open the link it makes and add her to a server you're in (your own is fine). That's what lets her message you.</li>
           <li>Paste the token here. It's checked with Discord, then kept encrypted in Windows' Credential Manager, never in her files.</li>
         </ol>
         <p class="hint">Whoever can message as you on Discord can use her: keep two-factor on for your Discord account, and turn this off when you don't need it.</p>
         <div class="actions"><input id="ptoken" type="password" placeholder="Paste the bot's token" autocomplete="off" spellcheck="false" /><button id="psave">Check and save</button><button id="pdel" class="ghost" hidden>Remove token</button></div>
         <div class="actions"><button id="phello" class="ghost">Send me a hello</button><span class="hint" id="phellomsg"></span></div>
         <p class="hint" id="pmsg"></p><p class="hint" id="pstate"></p>`,
      );
      const token = el.querySelector<HTMLInputElement>("#ptoken")!;
      const msg = el.querySelector<HTMLParagraphElement>("#pmsg")!;
      const state = el.querySelector<HTMLParagraphElement>("#pstate")!;
      const del = el.querySelector<HTMLButtonElement>("#pdel")!;
      const refresh = async () => {
        const st = await invoke<{ token: boolean; status: string }>("phone_status");
        if (!st) return;
        del.hidden = !st.token;
        state.textContent = `Token: ${st.token ? "saved" : "none yet"}. Chat: ${st.status || "off"}.`;
      };
      el.querySelector<HTMLButtonElement>("#psave")!.onclick = async () => {
        if (!token.value.trim()) return;
        msg.textContent = "Checking with Discord…";
        try {
          msg.textContent = await tauriInvoke<string>("save_phone_token", { token: token.value });
          token.value = "";
        } catch (e) {
          msg.textContent = String(e);
        }
        void refresh();
      };
      del.onclick = () => void invoke("delete_phone_token").then(() => ((msg.textContent = "Token removed."), refresh()));
      const hello = el.querySelector<HTMLSpanElement>("#phellomsg")!;
      el.querySelector<HTMLButtonElement>("#phello")!.onclick = async () => {
        hello.textContent = "Sending…";
        try {
          hello.textContent = await tauriInvoke<string>("phone_hello");
        } catch (e) {
          hello.textContent = String(e);
        }
      };
      void refresh();
      const timer = window.setInterval(() => (el.isConnected ? void refresh() : clearInterval(timer)), 3000);
    },
  },
  {
    name: "Memory",
    intro: "Her notes on how things worked in your apps, written when a job inside one went right and read the next time it comes up. On this PC only. (What she remembers about you is under Make her yours.)",
    render: (el) => {
      const draw = async () => {
        const notes = (await invoke<{ app: string; how: string; at: string }[]>("notes_list")) ?? [];
        el.innerHTML = notes.length ? "" : `<p class="hint">No notes yet.</p>`;
        notes.forEach((n, i) => {
          const row = document.createElement("div");
          row.className = "row";
          row.innerHTML = `<label><span></span><small></small></label><div class="control"><button class="ghost">Remove</button></div>`;
          row.querySelector("span")!.textContent = `${n.app} · ${n.at}`;
          row.querySelector("small")!.textContent = n.how;
          row.querySelector("button")!.onclick = () => void invoke("notes_remove", { index: i }).then(draw);
          el.appendChild(row);
        });
        if (notes.length) {
          const all = document.createElement("div");
          all.className = "actions";
          all.innerHTML = `<button class="ghost">Forget all her notes</button>`;
          all.querySelector("button")!.onclick = () => {
            if (confirm("Forget every note on how things worked? She works them out again.")) void invoke("notes_clear").then(draw);
          };
          el.appendChild(all);
        }
      };
      void draw();
    },
  },
  {
    name: "Privacy",
    intro: "What may leave your PC.",
    fields: [
      { kind: "toggle", label: "Show screenshots to online brains without asking", hint: "Off: she asks you first, once for each window in a job. On: she doesn't ask. Her eyes on this PC never need asking.", get: (s) => s.privacy.screenshots_to_cloud, set: (s, v) => (s.privacy.screenshots_to_cloud = v) },
      { kind: "toggle", label: "Speak during calls", hint: "Off: she writes instead while you're in a call.", get: (s) => s.privacy.speak_in_calls, set: (s, v) => (s.privacy.speak_in_calls = v) },
    ],
    render: (el) => {
      el.insertAdjacentHTML(
        "beforeend",
        `<p class="hint">She keeps the last few minutes of conversation in memory (never on disk), so "it" and "that" mean something. Her journal and model log (on this PC only) keep everything, for finding out what went wrong.</p>
         <div class="actions"><button id="forget">Forget our conversation</button><button id="wipe" class="ghost">Delete her logs…</button></div><p class="hint" id="forgot"></p>`,
      );
      const said = el.querySelector<HTMLParagraphElement>("#forgot")!;
      el.querySelector<HTMLButtonElement>("#forget")!.onclick = () => void invoke("forget", { logs: false }).then(() => (said.textContent = "Forgotten."));
      el.querySelector<HTMLButtonElement>("#wipe")!.onclick = () => {
        if (!confirm("Delete her journal, model log and recordings? What she did can't be looked into after.")) return;
        void invoke<number>("forget", { logs: true }).then((n) => (said.textContent = `Forgotten, and ${n ?? 0} log files deleted (the ones in use stay until she restarts).`));
      };
    },
  },
  {
    name: "About",
    intro: "Capricorn: Capri on your desktop, an open-source personal agent. github.com/theluckytiger/capricorn-engram",
    render: (el) => {
      el.insertAdjacentHTML(
        "beforeend",
        `<p class="hint">Version <span class="mono" id="ver">dev</span></p><p class="hint">Her journal and model log (everything she heard, decided and did) stay on this PC.</p><button id="logs">Open her logs</button>`,
      );
      void invoke<string>("version").then((v) => v && (el.querySelector("#ver")!.textContent = v));
      el.querySelector<HTMLButtonElement>("#logs")!.onclick = () => void invoke("open_logs");
    },
  },
];

/** Her whole brain on a free cloud tier: pick Gemini or Groq, get a free key (a button opens the page), paste it;
 *  she picks the best model that key can use. Off: back to thinking on this PC. */
function onlineBrain(el: HTMLElement) {
  // Which card a model id belongs to (Ollama's cloud models are ollama/…:cloud).
  const at = (m: string | null) => {
    if (!current.brain.cloud || !m) return "";
    if (m.startsWith("ollama/") && (m.endsWith(":cloud") || m.includes("-cloud"))) return "ollamacloud";
    return ONLINE.find(({ id }) => m.startsWith(`${id}/`))?.id ?? "";
  };
  // Hard jobs: the online model is `smart` (her local one keeps quick things and chat); everything: it's `fast`.
  const everything = at(current.brain.fast);
  const on = everything || at(current.brain.smart);
  const name = (id: string) => ONLINE.find((o) => o.id === id)?.name ?? id;
  const tagClass = (t: string) => (t === "Free" ? "free" : t === "Paid" ? "paid" : t === "Your plan" ? "plan" : "both");
  // No key to paste for these: your plan's app, or Ollama, is signed in on this PC.
  const isPlan = (id: string) => ONLINE.find((o) => o.id === id)?.tag === "Your plan" || id === "ollamacloud";
  el.insertAdjacentHTML(
    "afterbegin",
    `<h2>Online brain</h2>
     <p class="lead">Works best with Claude Code. Better support for the others is coming.</p>
     <ul class="points">
       <li><b>For hard jobs</b>: clicking around in apps, anything with several steps. Quick commands and chat stay on your PC.</li>
       <li><b>For everything</b>: all her thinking goes online, and your graphics card stays free for games.</li>
       <li><b>What leaves your PC</b>: only what that model handles, and only to the provider you pick. Screenshots: she asks first, unless you've turned that off in Privacy.</li>
     </ul>
     <div class="providers">
       <button class="card" data-id=""><span class="top"><strong>On this PC only</strong></span><small>Off. Nothing leaves your PC.</small></button>
       ${ONLINE.map((o) => `<button class="card" data-id="${o.id}"><span class="top"><strong>${o.name}</strong><span class="tag ${tagClass(o.tag)}">${o.tag}</span>${o.rec ? `<span class="rec">${o.rec}</span>` : ""}</span><small>${o.line}</small></button>`).join("")}
     </div>
     <div class="actions" id="omodelrow" hidden><span class="hint">Model</span><select id="omodel"></select></div>
     <p class="hint" id="omodelnote"></p>
     <div class="actions" id="oscope"><span class="hint">Use it</span><select id="ofor"><option value="hard">for hard jobs (recommended)</option><option value="all">for everything</option></select></div>
     <ol class="steps" id="osteps" hidden>
       <li><button id="okeypage" class="ghost">Get a key</button><span class="hint">Sign in there, create an API key and copy it.</span></li>
       <li><input id="okey" type="password" placeholder="Paste the key here" autocomplete="off" spellcheck="false" /><button id="ouse">Use it</button></li>
     </ol>
     <p class="status" id="omsg"></p>`,
  );
  const cards = [...el.querySelectorAll<HTMLButtonElement>(".providers .card")];
  const scope = el.querySelector<HTMLSelectElement>("#ofor")!;
  const steps = el.querySelector<HTMLOListElement>("#osteps")!;
  const keyPage = el.querySelector<HTMLButtonElement>("#okeypage")!;
  const key = el.querySelector<HTMLInputElement>("#okey")!;
  const msg = el.querySelector<HTMLParagraphElement>("#omsg")!;
  const local = () => models.find((m) => m.startsWith("ollama/")) ?? DEFAULTS.brain.fast;
  let chosen = on;
  // Pick the model: your plan's (Claude Code, Cursor, Codex) or what your key can use, the recommended ones first.
  const modelRow = el.querySelector<HTMLDivElement>("#omodelrow")!;
  const modelSel = el.querySelector<HTMLSelectElement>("#omodel")!;
  const modelNote = el.querySelector<HTMLParagraphElement>("#omodelnote")!;
  let planList: { id: string; label: string; note: string; rec: boolean }[] = [];
  const noteModel = () => (modelNote.textContent = planList.find((m) => m.id === modelSel.value)?.note ?? "");
  const showModels = async (id: string) => {
    modelRow.hidden = true;
    modelNote.textContent = "";
    if (!id) return;
    const list = (await tauriInvoke<typeof planList>("plan_models", { provider: id }).catch(() => [])) ?? [];
    if (chosen !== id || !list.length) return;
    planList = list;
    // Built as elements, not HTML: the names come from another app's list.
    const group = (label: string, models: typeof planList) => {
      const g = document.createElement("optgroup");
      g.label = label;
      for (const m of models) g.appendChild(new Option(m.label, m.id));
      return g;
    };
    const others = list.filter((m) => !m.rec);
    modelSel.replaceChildren(group("Recommended", list.filter((m) => m.rec)), ...(others.length ? [group("Everything else", others)] : []));
    const now = [current.brain.smart, current.brain.fast].find((m) => m && at(m) === id)?.slice(id.length + 1);
    modelSel.value = now && list.some((m) => m.id === now) ? now : list[0].id;
    modelSel.dataset.for = id;
    modelRow.hidden = false;
    noteModel();
  };
  const paint = () => {
    for (const c of cards) c.classList.toggle("on", c.dataset.id === chosen);
    el.querySelector<HTMLDivElement>("#oscope")!.hidden = !chosen;
  };
  const say = (text: string, off = false) => {
    msg.textContent = text;
    msg.classList.toggle("off", off);
  };
  scope.value = everything ? "all" : "hard";
  paint();
  if (on) say(everything ? `On: ${current.brain.fast.slice(on.length + 1)} at ${name(on)} does all her thinking.` : `On: ${current.brain.smart!.slice(on.length + 1)} at ${name(on)} takes her hard jobs; the rest stays on your PC.`);
  else say("Off: she thinks only on your PC.", true);
  const use = async (id: string) => {
    say(isPlan(id) ? `Checking your ${name(id)} plan on this PC… (a few seconds)` : `Checking ${name(id)}… (trying its models, a few seconds)`, true);
    // Not restarted halfway: the bar comes back once the new choice is saved (restarting during the check kept
    // the old brain).
    bar.hidden = true;
    try {
      if (key.value.trim()) {
        await tauriInvoke<string>("save_key", { provider: id, key: key.value });
        key.value = "";
      }
      const r = await tauriInvoke<{ fast: string; eyes: string | null }>("online_brain", { provider: id, model: modelSel.dataset.for === id ? modelSel.value : null });
      current.brain.cloud = true;
      if (scope.value === "all") {
        current.brain.fast = r.fast;
        // An online brain that can't see, or screenshots that may not go online: her eyes stay on this PC.
        current.brain.eyes = (current.privacy.screenshots_to_cloud && r.eyes) || local();
        current.brain.auto = false;
        if (at(current.brain.smart)) current.brain.smart = null;
      } else {
        current.brain.smart = r.fast;
        if (at(current.brain.fast)) {
          current.brain.fast = local();
          current.brain.eyes = current.brain.fast;
          current.brain.auto = true;
        }
      }
      save();
      show(currentPage);
      const what = scope.value === "all" ? `${r.fast.slice(id.length + 1)} at ${name(id)} does all her thinking` : `${r.fast.slice(id.length + 1)} at ${name(id)} takes her hard jobs`;
      el.querySelector<HTMLParagraphElement>("#omsg")!.textContent = `Done: ${what}. Restart her to switch.`;
    } catch (e) {
      const said = String(e);
      // A plan has no key to paste: what went wrong says what to do (sign in to its app).
      if (isPlan(id)) return say(said, true);
      steps.hidden = false;
      keyPage.textContent = `Get a key from ${name(id)}`;
      say(said.includes("no key yet") ? `Two steps and ${name(id)} is hers:` : said, true);
    }
  };
  const off = () => {
    if (at(current.brain.fast)) {
      current.brain.fast = local();
      current.brain.eyes = current.brain.fast;
      current.brain.auto = true;
    }
    if (at(current.brain.smart)) current.brain.smart = null;
    save();
    show(currentPage);
  };
  for (const c of cards)
    c.onclick = () => {
      chosen = c.dataset.id ?? "";
      steps.hidden = true;
      paint();
      if (!chosen) return off();
      void showModels(chosen);
      void use(chosen);
    };
  modelSel.onchange = () => {
    noteModel();
    if (chosen) void use(chosen);
  };
  if (on) void showModels(on);
  scope.onchange = () => {
    if (chosen) void use(chosen);
  };
  keyPage.onclick = () => void invoke("key_page", { provider: chosen });
  el.querySelector<HTMLButtonElement>("#ouse")!.onclick = () => {
    if (key.value.trim() && chosen) void use(chosen);
  };
}

const root = document.getElementById("app")!;
root.innerHTML = `
  <nav>${PAGES.map((p, i) => `<button data-i="${i}">${p.name}</button>`).join("")}</nav>
  <main><h1></h1><p class="intro"></p><section></section></main>
  <div class="bar" hidden><span></span><button id="restart">Restart her</button></div>`;
const bar = root.querySelector<HTMLDivElement>(".bar")!;
root.querySelector<HTMLButtonElement>("#restart")!.onclick = () => void invoke("restart");

function show(i: number) {
  currentPage = i;
  root.querySelectorAll("nav button").forEach((b, j) => b.classList.toggle("on", i === j));
  const page = PAGES[i];
  root.querySelector("h1")!.textContent = page.name;
  root.querySelector(".intro")!.textContent = page.intro;
  const section = root.querySelector("section")!;
  section.innerHTML = "";
  for (const f of page.fields ?? []) section.appendChild(field(f));
  page.render?.(section, current);
}

function field(f: Field): HTMLElement {
  const row = document.createElement("div");
  row.className = "row";
  row.innerHTML = `<label><span></span><small></small></label><div class="control"></div>`;
  row.querySelector("span")!.textContent = f.label;
  row.querySelector("small")!.textContent = f.hint ?? "";
  const box = row.querySelector(".control")!;
  if (f.kind === "toggle") {
    const t = document.createElement("button");
    t.className = "toggle";
    t.setAttribute("aria-label", f.label);
    const word = document.createElement("span");
    word.className = "toggle-word";
    const paint = () => {
      t.classList.toggle("on", f.get(current));
      t.setAttribute("aria-pressed", String(f.get(current)));
      word.textContent = f.get(current) ? "On" : "Off";
      word.classList.toggle("on", f.get(current));
    };
    paint();
    t.onclick = () => {
      f.set(current, !f.get(current));
      paint();
      save();
      if (f.redraw) show(currentPage);
    };
    box.append(t, word);
  } else if (f.kind === "lines") {
    const area = document.createElement("textarea");
    area.rows = 4;
    area.placeholder = f.placeholder ?? "";
    area.value = f.get(current).join("\n");
    area.onchange = () => {
      f.set(current, area.value.split("\n").map((l) => l.trim()).filter(Boolean));
      save();
    };
    box.appendChild(area);
  } else if (f.kind === "select") {
    const sel = document.createElement("select");
    for (const [v, l] of f.options()) sel.add(new Option(l, v));
    sel.value = f.get(current);
    sel.onchange = () => {
      f.set(current, sel.value);
      save();
      if (f.redraw) show(currentPage);
    };
    box.appendChild(sel);
  } else {
    const input = document.createElement("input");
    input.type = f.kind === "number" ? "number" : "text";
    if (f.kind === "number") Object.assign(input, { min: f.min, max: f.max, step: f.step ?? 1 });
    input.value = String(f.get(current));
    input.onchange = () => {
      if (f.kind === "number") {
        const n = Math.min(f.max, Math.max(f.min, Number(input.value) || 0));
        f.set(current, n);
        input.value = String(n);
      } else f.set(current, input.value);
      save();
      if (f.kind === "text" && f.redraw) show(currentPage);
    };
    box.appendChild(input);
  }
  return row;
}

let timer = 0;
// When this window last saved (its own save comes back as a change; that one isn't news).
let savedAt = 0;
/** Saves a moment after the last change, and says whether a restart is needed for it. */
function save() {
  clearTimeout(timer);
  timer = window.setTimeout(async () => {
    savedAt = Date.now();
    const restart = await invoke<boolean>("save_settings", { settings: current });
    bar.hidden = false;
    bar.querySelector("span")!.textContent = restart ? "Saved. Some of this applies when she restarts." : "Saved.";
    bar.querySelector<HTMLButtonElement>("#restart")!.hidden = !restart;
    if (!restart) setTimeout(() => (bar.hidden = true), 1800);
  }, 300);
}

root.querySelectorAll<HTMLButtonElement>("nav button").forEach((b) => (b.onclick = () => show(Number(b.dataset.i))));
void (async () => {
  current = (await invoke<Settings>("get_settings")) ?? structuredClone(DEFAULTS);
  models = (await invoke<string[]>("local_models")) ?? [];
  // settings.html?page=Brain opens on that page (her menu, and pictures of a page).
  const asked = PAGES.findIndex((p) => p.name.toLowerCase() === new URLSearchParams(location.search).get("page")?.toLowerCase());
  show(Math.max(0, asked));
  // Drawn: the window can show now (it waits hidden, so it never flashes white).
  requestAnimationFrame(() => requestAnimationFrame(() => void invoke("settings_ready")));
  // Changed elsewhere ("call me…", "remember that…" said to her): shown here too.
  await listen("capri://settings", async () => {
    if (Date.now() - savedAt < 1500) return;
    current = (await invoke<Settings>("get_settings")) ?? current;
    show(currentPage);
  });
})();
