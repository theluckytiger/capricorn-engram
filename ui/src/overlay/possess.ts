// Possessing a window: drawn over it (the window itself is untouched and stays clickable): glitch slices tear
// through it, then it looks at you with her eyes and her zigzag grin, more eyes open across it, it laughs, and it
// glitches back to itself. On its own clock, twelve drawings a second, like her cut-ins.

type Rect = [number, number, number, number];

const INK = "rgba(11,6,20,";
const IRIS = "#3e5bd8";
const IRIS_LIGHT = "#7fb0ff";
const GREEN = "#08f562";
const CRIMSON = "#f60655";
const WHITE = "#fbf8fc";

/** How long it takes, in drawings at 12 a second. */
const DRAWINGS = 54;

/** Possesses the window at `rect` (CSS px of this screen); resolves when it's itself again. `hold`: stays on that
 *  drawing (for checking one). */
export function possess(rect: Rect, hold?: number): Promise<void> {
  const [l, t, r, b] = rect;
  const [w, h] = [r - l, b - t];
  const canvas = document.createElement("canvas");
  canvas.className = "possessed";
  canvas.width = Math.round(w * devicePixelRatio);
  canvas.height = Math.round(h * devicePixelRatio);
  Object.assign(canvas.style, { left: `${l}px`, top: `${t}px`, width: `${w}px`, height: `${h}px` });
  document.body.appendChild(canvas);
  const ctx = canvas.getContext("2d")!;
  ctx.scale(devicePixelRatio, devicePixelRatio);

  // Her other eyes, scattered over it, each opening on its own drawing.
  const more = Array.from({ length: 9 }, (_, i) => ({ x: w * (0.08 + Math.random() * 0.84), y: h * (0.1 + Math.random() * 0.8), s: 0.35 + Math.random() * 0.4, at: 20 + i * 2, red: i % 3 === 0 }));
  const [cx, cy] = [w / 2, h * 0.42];
  const eyeW = Math.min(w * 0.16, 170);

  return new Promise((done) => {
    let n = hold ? hold - 1 : 0;
    const frame = () => {
      n = hold ?? n + 1;
      ctx.clearRect(0, 0, w, h);
      const laughing = n >= 36 && n < 48;
      const shake = laughing ? (n % 2 ? 4 : -4) : 0;
      // The window goes dark under her, more as she takes it.
      ctx.fillStyle = `${INK}${Math.min(0.55, n * 0.05) * (n > 48 ? (DRAWINGS - n) / 6 : 1)})`;
      ctx.fillRect(0, 0, w, h);
      if (n <= 6 || n > 48) slices(ctx, w, h);
      if (n > 6 && n <= 50) {
        ctx.save();
        ctx.translate(shake, 0);
        // Her two eyes, opening over three drawings; squeezed shut to arcs when she laughs.
        const open = Math.min(1, (n - 6) / 3);
        for (const side of [-1, 1]) {
          const ex = cx + side * eyeW * 0.85;
          if (laughing) arc(ctx, ex, cy, eyeW);
          else eye(ctx, ex, cy, eyeW, eyeW * 0.42 * open, IRIS, IRIS_LIGHT, n);
        }
        // The grin: thin at first, then wide, zigzag teeth; open when she laughs.
        if (n > 10) grin(ctx, cx, cy + eyeW * 0.9, eyeW * (n > 14 ? 2.6 : 1.4), laughing ? eyeW * 0.5 : eyeW * 0.22);
        for (const e of more) {
          if (n < e.at) continue;
          const o = Math.min(1, (n - e.at) / 2);
          eye(ctx, e.x, e.y, eyeW * e.s, eyeW * e.s * 0.42 * o, e.red ? CRIMSON : GREEN, WHITE, n);
        }
        ctx.restore();
      }
      // Her frame around it, crackling.
      ctx.strokeStyle = n % 2 ? CRIMSON : GREEN;
      ctx.lineWidth = 3;
      ctx.strokeRect(1.5, 1.5, w - 3, h - 3);
      if (n >= DRAWINGS) {
        clearInterval(timer);
        canvas.remove();
        done();
      }
    };
    const timer = setInterval(frame, 83);
    frame();
  });
}

/** Bands of the window torn sideways, tinted. */
function slices(ctx: CanvasRenderingContext2D, w: number, h: number) {
  for (let i = 0; i < 9; i++) {
    const y = Math.random() * h;
    const bh = 6 + Math.random() * 30;
    ctx.fillStyle = Math.random() < 0.5 ? "rgba(246,6,85,0.35)" : "rgba(8,245,98,0.3)";
    ctx.fillRect((Math.random() - 0.5) * 60, y, w, bh);
  }
}

/** An almond eye with an iris that tracks a little, a pupil and a glint. */
function eye(ctx: CanvasRenderingContext2D, x: number, y: number, ew: number, eh: number, iris: string, light: string, n: number) {
  const almond = (w: number, hh: number, ox = 0, oy = 0) => {
    ctx.beginPath();
    ctx.moveTo(x + ox - w / 2, y + oy);
    ctx.quadraticCurveTo(x + ox, y + oy - hh, x + ox + w / 2, y + oy);
    ctx.quadraticCurveTo(x + ox, y + oy + hh, x + ox - w / 2, y + oy);
    ctx.closePath();
  };
  almond(ew * 1.12, eh * 1.15 + 3);
  ctx.fillStyle = "#0b0614";
  ctx.fill();
  if (eh < 2) return;
  almond(ew, eh);
  ctx.fillStyle = iris;
  ctx.fill();
  const look = Math.sin(n / 5) * ew * 0.08;
  almond(ew * 0.5, eh * 0.62, look);
  ctx.fillStyle = light;
  ctx.fill();
  almond(ew * 0.24, eh * 0.5, look);
  ctx.fillStyle = "#0b0614";
  ctx.fill();
  almond(ew * 0.1, eh * 0.18, look - ew * 0.08, -eh * 0.22);
  ctx.fillStyle = WHITE;
  ctx.fill();
}

/** A laughing eye, squeezed shut: ^ */
function arc(ctx: CanvasRenderingContext2D, x: number, y: number, ew: number) {
  ctx.strokeStyle = WHITE;
  ctx.lineWidth = Math.max(3, ew * 0.06);
  ctx.beginPath();
  ctx.moveTo(x - ew / 2, y + ew * 0.12);
  ctx.lineTo(x, y - ew * 0.16);
  ctx.lineTo(x + ew / 2, y + ew * 0.12);
  ctx.stroke();
}

/** Her grin: a crescent, a zigzag of teeth across it, her fang. */
function grin(ctx: CanvasRenderingContext2D, x: number, y: number, gw: number, open: number) {
  ctx.fillStyle = "#a3053c";
  ctx.beginPath();
  ctx.moveTo(x - gw / 2, y - open * 0.3);
  ctx.quadraticCurveTo(x, y + open * 1.6, x + gw / 2, y - open * 0.5);
  ctx.quadraticCurveTo(x, y + open * 0.2, x - gw / 2, y - open * 0.3);
  ctx.fill();
  ctx.strokeStyle = WHITE;
  ctx.lineWidth = Math.max(2, gw * 0.018);
  ctx.beginPath();
  for (let i = 0; i <= 12; i++) {
    const u = i / 12;
    const base = y - open * 0.3 + Math.sin(u * Math.PI) * open * 0.7;
    ctx.lineTo(x - gw / 2 + u * gw, base + (i % 2 ? open * 0.22 : -open * 0.12));
  }
  ctx.stroke();
}
