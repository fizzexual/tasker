// Turning numbers into the short text shown on screen. Pure functions, no DOM.

const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

/** 1536 → "1.5 KB". Always at most 4 significant characters so columns stay narrow. */
export function bytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0 B";
  let i = 0;
  let v = n;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  // Rounding can push 1023.97 to "1024"; move to the next unit instead.
  if (i > 0 && v >= 999.95 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  const digits = i === 0 || v >= 100 ? 0 : 1;
  return `${v.toFixed(digits)} ${UNITS[i]}`;
}

/** Bytes per second. Below 1 KB/s shows as "0" so idle rows are quiet. */
export function rate(n: number): string {
  if (!Number.isFinite(n) || n < 1024) return "0";
  return `${bytes(n)}/s`;
}

/** A disk/network speed for headings: always has a unit. */
export function rateFull(n: number): string {
  if (!Number.isFinite(n) || n < 1) return "0 B/s";
  return `${bytes(n)}/s`;
}

/** 12.345 → "12.3%"; tiny non-zero values are not shown as a misleading "0.0%". */
export function percent(n: number, digits = 1): string {
  if (!Number.isFinite(n) || n <= 0) return `${(0).toFixed(digits)}%`;
  const clamped = Math.min(n, 100);
  const min = 1 / 10 ** digits;
  if (clamped < min) return `<${min}%`;
  return `${clamped.toFixed(digits)}%`;
}

/** Whole percent, for places with little room. */
export function percent0(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0%";
  return `${Math.round(Math.min(n, 100))}%`;
}

/** Seconds → "3d 04:05:06" or "04:05:06". */
export function duration(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) return "–";
  const s = Math.floor(totalSeconds);
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const pad = (v: number) => String(v).padStart(2, "0");
  const clock = `${pad(h)}:${pad(m)}:${pad(s % 60)}`;
  return d > 0 ? `${d}d ${clock}` : clock;
}

/** 1234567 → "1,234,567". */
export function count(n: number): string {
  if (!Number.isFinite(n)) return "–";
  return Math.round(n).toLocaleString("en-US");
}

/** Picks a round chart ceiling at or above `max` (in bytes/s), never below 100 KB/s. */
export function niceCeil(max: number): number {
  const floor = 100 * 1024;
  if (!Number.isFinite(max) || max <= floor) return floor;
  const exp = Math.pow(1024, Math.floor(Math.log(max) / Math.log(1024)));
  const lead = max / exp;
  for (const step of [1, 2, 5, 10, 20, 50, 100, 200, 500, 1024]) {
    if (lead <= step) return step * exp;
  }
  return 1024 * exp;
}

/**
 * Windows stores a driver's date as month-day-year ("10-18-2026"), which reads as a wrong date
 * in most countries. Written here as year-month-day; anything else is shown as it came.
 */
export function driverDate(raw: string): string {
  const m = /^(\d{1,2})-(\d{1,2})-(\d{4})$/.exec(raw.trim());
  if (!m) return raw.trim();
  const [, month, day, year] = m;
  if (Number(month) < 1 || Number(month) > 12 || Number(day) < 1 || Number(day) > 31) return raw.trim();
  return `${year}-${month.padStart(2, "0")}-${day.padStart(2, "0")}`;
}

/** Processor speed: 4450 → "4.45 GHz", 800 → "800 MHz". */
export function speed(mhz: number): string {
  if (!mhz) return "–";
  return mhz >= 1000 ? `${(mhz / 1000).toFixed(2)} GHz` : `${mhz} MHz`;
}
