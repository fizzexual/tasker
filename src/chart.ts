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
  for (const s of series) {
    const { n, from, step, x0, lead } = place(s.values.length, opts.capacity, w);
    if (n === 0) continue;
    const y = (v: number) => h - 1 - (Math.min(Math.max(v, 0), max) / max) * (h - 2);
    const fade = (alpha: number) => {
      // Fades out towards the floor, so the line itself stays the thing you read.
      if (/^#[0-9a-f]{6}$/i.test(s.color)) {
        const g = ctx.createLinearGradient(0, 0, 0, h);
        g.addColorStop(0, `${s.color}47`);
        g.addColorStop(1, `${s.color}05`);
        ctx.globalAlpha = alpha;
        ctx.fillStyle = g;
      } else {
        ctx.globalAlpha = 0.16 * alpha;
        ctx.fillStyle = s.color;
      }
      ctx.fill();
      ctx.globalAlpha = 1;
    };

    // Before the first reading: the typical level so far carried back, fainter and dashed, so a
    // graph that has just started is not half empty and still does not pass the carried part off
    // as measured. (Not the first reading: that one is taken while this app itself is starting.)
    if (lead > 0) {
      const py = y(typical(s.values.slice(from)));
      ctx.beginPath();
      ctx.moveTo(0, py);
      ctx.lineTo(lead, py);
      ctx.strokeStyle = s.color;
      ctx.lineWidth = 1.5;
      ctx.setLineDash([3, 4]);
      ctx.globalAlpha = 0.45;
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.globalAlpha = 1;
      if (s.fill) {
        ctx.lineTo(lead, h);
        ctx.lineTo(0, h);
        ctx.closePath();
        fade(0.5);
      }
    }

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
      fade(1);
    }
  }
}

/** The middle reading: a level one early spike cannot pull up. */
export function typical(values: readonly number[]): number {
  const sorted = values.filter((v) => Number.isFinite(v)).sort((a, b) => a - b);
  if (sorted.length === 0) return 0;
  const mid = sorted.length >> 1;
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
}

/**
 * Where a line of `count` readings sits on a graph `w` wide that stands for `capacity` readings,
 * the newest at the right edge. `lead` is the width before the first reading (0 once the history
 * is full): the part the graph fills by carrying the first value back.
 */
export function place(count: number, capacity: number, w: number): { n: number; from: number; step: number; x0: number; lead: number } {
  const n = Math.min(count, capacity);
  const step = w / Math.max(1, capacity - 1);
  const x0 = n >= capacity && capacity > 1 ? 0 : Math.max(0, w - Math.max(0, n - 1) * step);
  return { n, from: count - n, step, x0, lead: n > 0 && x0 > 0.5 ? x0 : 0 };
}
