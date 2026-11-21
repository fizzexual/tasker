// A small canvas line chart. No library: it draws ~120 points once per refresh.

export interface Series {
  values: readonly number[];
  color: string;
  fill?: boolean;
}

export interface ChartOptions {
  /** Top of the scale. */
  max: number;
  /** How many points the full width stands for (the newest is at the right edge). */
  capacity: number;
  grid?: boolean;
}

function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

export function seriesColor(kind: "cpu" | "mem" | "disk" | "net" | "gpu"): string {
  return cssVar(`--c-${kind}`);
}

/** Fits the canvas bitmap to its on-screen size so lines stay sharp on any display scale. */
function fit(canvas: HTMLCanvasElement): { w: number; h: number; ctx: CanvasRenderingContext2D } | null {
  const dpr = window.devicePixelRatio || 1;
  const w = canvas.clientWidth;
  const h = canvas.clientHeight;
  if (w === 0 || h === 0) return null;
  const bw = Math.round(w * dpr);
  const bh = Math.round(h * dpr);
  if (canvas.width !== bw || canvas.height !== bh) {
    canvas.width = bw;
    canvas.height = bh;
  }
  const ctx = canvas.getContext("2d");
  if (!ctx) return null;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return { w, h, ctx };
}

export function drawChart(canvas: HTMLCanvasElement, series: readonly Series[], opts: ChartOptions): void {
  const f = fit(canvas);
  if (!f) return;
  const { w, h, ctx } = f;
  ctx.clearRect(0, 0, w, h);

  if (opts.grid !== false) {
    ctx.strokeStyle = cssVar("--grid");
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let i = 1; i < 4; i++) {
      const y = Math.round((h * i) / 4) + 0.5;
      ctx.moveTo(0, y);
      ctx.lineTo(w, y);
    }
    for (let i = 1; i < 6; i++) {
      const x = Math.round((w * i) / 6) + 0.5;
      ctx.moveTo(x, 0);
      ctx.lineTo(x, h);
    }
    ctx.stroke();
  }

  const max = opts.max > 0 ? opts.max : 1;
  const step = w / Math.max(1, opts.capacity - 1);
  for (const s of series) {
    const n = Math.min(s.values.length, opts.capacity);
    if (n === 0) continue;
    const from = s.values.length - n;
    const x0 = w - (n - 1) * step;
    const y = (v: number) => h - 1 - (Math.min(Math.max(v, 0), max) / max) * (h - 2);

    ctx.beginPath();
    for (let i = 0; i < n; i++) {
      const px = x0 + i * step;
      const py = y(s.values[from + i]);
      if (i === 0) ctx.moveTo(px, py);
      else ctx.lineTo(px, py);
    }
    ctx.strokeStyle = s.color;
    ctx.lineWidth = 1.5;
    ctx.lineJoin = "round";
    ctx.stroke();

    if (s.fill) {
      ctx.lineTo(w, h);
      ctx.lineTo(x0, h);
      ctx.closePath();
      // Fades out towards the floor, so the line itself stays the thing you read.
      if (/^#[0-9a-f]{6}$/i.test(s.color)) {
        const fade = ctx.createLinearGradient(0, 0, 0, h);
        fade.addColorStop(0, `${s.color}47`);
        fade.addColorStop(1, `${s.color}05`);
        ctx.fillStyle = fade;
        ctx.fill();
      } else {
        ctx.globalAlpha = 0.16;
        ctx.fillStyle = s.color;
        ctx.fill();
        ctx.globalAlpha = 1;
      }
    }
  }
}
