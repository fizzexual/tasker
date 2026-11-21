// The columns the process table can show, and how each one is written out.

import { bytes, count, percent, rate } from "./format";
import { priorityName, SECTION_LABEL, statusText, type ColumnKey, type Row, type StaticInfo } from "./model";

export interface Column {
  key: ColumnKey;
  label: string;
  /** Pixels. The Name column takes whatever is left. */
  width: number;
  num: boolean;
  /** Tints the cell by how big the number is. */
  heat?: "cpu" | "mem" | "gpu";
  text(row: Row, info: StaticInfo | undefined): string;
}

/** Hours:minutes:seconds, as Windows Task Manager writes CPU time. */
export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${Math.floor(s / 3600)}:${pad(Math.floor((s % 3600) / 60))}:${pad(s % 60)}`;
}

const one = (row: Row) => (row.kind === "proc" ? row.rep : null);

export const COLUMNS: readonly Column[] = [
  { key: "status", label: "Status", width: 112, num: false, text: (r) => statusText(r.flags) },
  { key: "pid", label: "PID", width: 62, num: true, text: (r) => (one(r) ? String(r.pid) : "") },
  { key: "user", label: "User name", width: 110, num: false, text: (_, i) => i?.user ?? "" },
  { key: "cpu", label: "CPU", width: 62, num: true, heat: "cpu", text: (r) => percent(r.cpu) },
  { key: "mem", label: "Memory", width: 86, num: true, heat: "mem", text: (r) => bytes(r.mem) },
  { key: "disk", label: "Disk", width: 84, num: true, text: (r) => rate(r.disk) },
  { key: "gpu", label: "GPU", width: 58, num: true, heat: "gpu", text: (r) => percent(r.gpu) },
  { key: "threads", label: "Threads", width: 70, num: true, text: (r) => count(r.threads) },
  { key: "handles", label: "Handles", width: 74, num: true, text: (r) => count(r.handles) },
  { key: "cputime", label: "CPU time", width: 84, num: true, text: (r) => clock(r.cpuTimeMs) },
  { key: "priority", label: "Priority", width: 100, num: false, text: (r) => (one(r) ? priorityName(r.rep!.priority) : "") },
  { key: "session", label: "Session", width: 64, num: true, text: (r) => (r.rep ? String(r.rep.session) : "") },
  {
    key: "started",
    label: "Started",
    width: 150,
    num: false,
    text: (r) => (one(r) && r.rep!.startMs ? new Date(r.rep!.startMs).toLocaleString() : ""),
  },
  { key: "arch", label: "Architecture", width: 86, num: false, text: (_, i) => i?.architecture ?? "" },
  {
    key: "elevated",
    label: "Elevated",
    width: 72,
    num: false,
    text: (_, i) => (i?.elevated ? "Yes" : i?.elevated === false ? "No" : ""),
  },
  { key: "ws", label: "Working set", width: 96, num: true, text: (r) => bytes(r.ws) },
  { key: "commit", label: "Commit size", width: 96, num: true, text: (r) => bytes(r.commit) },
  { key: "type", label: "Type", width: 150, num: false, text: (r) => (r.section ? SECTION_LABEL[r.section] : "") },
  { key: "description", label: "Description", width: 200, num: false, text: (_, i) => i?.description ?? "" },
  { key: "publisher", label: "Publisher", width: 170, num: false, text: (_, i) => i?.company ?? "" },
  { key: "path", label: "Path", width: 320, num: false, text: (_, i) => i?.path ?? "" },
  { key: "command", label: "Command line", width: 360, num: false, text: (r, i) => (one(r) ? (i?.command ?? "") : "") },
];

export const DEFAULT_COLUMNS: readonly ColumnKey[] = ["status", "pid", "cpu", "mem", "disk", "gpu"];
const KNOWN = new Set<ColumnKey>(COLUMNS.map((c) => c.key));

/**
 * Cleans a saved column choice: unknown names and repeats are dropped, and the result is put
 * in the table's fixed order. Anything that is not a list gives the default columns.
 */
export function cleanColumns(saved: unknown): ColumnKey[] {
  if (!Array.isArray(saved)) return [...DEFAULT_COLUMNS];
  const wanted = new Set(saved.filter((k): k is ColumnKey => typeof k === "string" && KNOWN.has(k as ColumnKey)));
  return COLUMNS.filter((c) => wanted.has(c.key)).map((c) => c.key);
}

/** Columns that need the once-per-process facts (program file, user, publisher…). */
export const INFO_COLUMNS: ReadonlySet<ColumnKey> = new Set(["user", "arch", "elevated", "description", "publisher", "path", "command"]);

/** The grid tracks for a row: the Name column, then each chosen column at its width. */
export function gridTemplate(keys: readonly ColumnKey[]): { template: string; minWidth: number } {
  const cols = COLUMNS.filter((c) => keys.includes(c.key));
  const gap = 8;
  return {
    template: ["minmax(200px, 1fr)", ...cols.map((c) => `${c.width}px`)].join(" "),
    minWidth: 200 + 16 + cols.reduce((sum, c) => sum + c.width + gap, 0),
  };
}
