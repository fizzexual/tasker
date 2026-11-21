// What the process table shows, worked out from the raw list. Pure functions, no DOM.

/** Has an open window: shown under "Apps". */
export const F_APP = 1;
/** Its window has stopped answering. */
export const F_HUNG = 2;
/** Every thread is frozen. */
export const F_SUSPENDED = 4;
/** Part of Windows itself. */
export const F_WINDOWS = 8;
/** Running in efficiency mode. */
export const F_ECO = 16;

export interface Proc {
  pid: number;
  ppid: number;
  name: string;
  /** Share of the whole PC, 0–100. */
  cpu: number;
  /** Bytes only this process uses. */
  mem: number;
  /** Bytes per second. */
  diskRead: number;
  diskWrite: number;
  /** Share of the graphics card, 0–100. */
  gpu: number;
  threads: number;
  handles: number;
  session: number;
  /** Windows base priority number (8 = normal). */
  priority: number;
  cpuTimeMs: number;
  /** Unix milliseconds; with the PID it names one process for good. */
  startMs: number;
  /** Working set, bytes. */
  ws: number;
  /** Commit size, bytes. */
  commit: number;
  flags: number;
  /** Title of its top window, empty when it has none. */
  title: string;
}

/** A row as the backend sends it (see `ProcRow` in proclist.rs). */
export type RawProc = [
  pid: number,
  ppid: number,
  name: string,
  cpu: number,
  mem: number,
  diskRead: number,
  diskWrite: number,
  gpu: number,
  threads: number,
  handles: number,
  session: number,
  priority: number,
  cpuTimeMs: number,
  startMs: number,
  ws: number,
  commit: number,
  flags: number,
  title: string,
];

export function fromRaw(r: RawProc): Proc {
  return {
    pid: r[0],
    ppid: r[1],
    name: r[2],
    cpu: r[3],
    mem: r[4],
    diskRead: r[5],
    diskWrite: r[6],
    gpu: r[7],
    threads: r[8],
    handles: r[9],
    session: r[10],
    priority: r[11],
    cpuTimeMs: r[12],
    startMs: r[13],
    ws: r[14],
    commit: r[15],
    flags: r[16],
    title: r[17],
  };
}

/** Facts that stay the same for a process's whole life, fetched once per process. */
export interface StaticInfo {
  path: string;
  user: string;
  description: string;
  company: string;
  architecture: string;
  elevated: boolean | null;
  command: string;
}

/** Identity that survives PID reuse. */
export function procKey(p: Pick<Proc, "pid" | "startMs">): string {
  return `${p.pid}:${p.startMs}`;
}

export type InfoOf = (p: Proc) => StaticInfo | undefined;

/** "By type" is the Windows Task Manager Processes tab; "list" is its Details tab. */
export type ViewMode = "type" | "name" | "list";
export type Section = "apps" | "background" | "windows";
export const SECTIONS: readonly Section[] = ["apps", "background", "windows"];
export const SECTION_LABEL: Record<Section, string> = {
  apps: "Apps",
  background: "Background processes",
  windows: "Windows processes",
};

export type ColumnKey =
  | "name"
  | "status"
  | "pid"
  | "cpu"
  | "mem"
  | "disk"
  | "gpu"
  | "user"
  | "threads"
  | "handles"
  | "cputime"
  | "priority"
  | "session"
  | "started"
  | "arch"
  | "elevated"
  | "description"
  | "publisher"
  | "command"
  | "path"
  | "ws"
  | "commit"
  | "type";

export interface ViewOptions {
  view: ViewMode;
  sortKey: ColumnKey;
  desc: boolean;
  filter: string;
  /** Lower-case program names of the groups that are open. */
  expanded: ReadonlySet<string>;
  /** Sections that are folded shut ("By type" only). */
  collapsed: ReadonlySet<Section>;
  info: InfoOf;
}

export interface Row {
  /** Stable identity: `p:<pid>` for a process, `g:<name>` for a group, `s:<section>` for a heading. */
  key: string;
  kind: "proc" | "group" | "section";
  /** A process shown inside an open group. */
  child: boolean;
  name: string;
  /** The process, or for a group its first member (used for the icon). 0 for a heading. */
  pid: number;
  /** How many processes this row stands for (for a heading: how many rows are under it). */
  count: number;
  open: boolean;
  section: Section | null;
  /** The process itself, or a group's first member; null for a heading. */
  rep: Proc | null;
  cpu: number;
  mem: number;
  disk: number;
  gpu: number;
  threads: number;
  handles: number;
  ws: number;
  commit: number;
  cpuTimeMs: number;
  flags: number;
}

export function sectionOf(flags: number): Section {
  if (flags & F_APP) return "apps";
  if (flags & F_WINDOWS) return "windows";
  return "background";
}

/** The one thing worth saying about a process's state; empty when it simply runs. */
export function statusText(flags: number): string {
  if (flags & F_HUNG) return "Not responding";
  if (flags & F_SUSPENDED) return "Suspended";
  if (flags & F_ECO) return "Efficiency mode";
  return "";
}

const PRIORITY_NAMES: Record<number, string> = {
  4: "Low",
  6: "Below normal",
  8: "Normal",
  10: "Above normal",
  13: "High",
  24: "Realtime",
};

export function priorityName(base: number): string {
  return PRIORITY_NAMES[base] ?? (base > 0 ? String(base) : "");
}

/** The name people know a program by ("Google Chrome"), falling back to the file name. */
export function friendlyName(p: Proc, info: StaticInfo | undefined): string {
  return info?.description || p.name;
}

/**
 * Whether a process matches the search box. Plain text is looked for in the file name, the
 * friendly name, the window title and the user; a number also finds a PID; `session:2` shows
 * one person's processes.
 */
export function matches(p: Proc, filter: string, info?: StaticInfo): boolean {
  const f = filter.trim().toLowerCase();
  if (!f) return true;
  const session = /^session:(\d+)$/.exec(f);
  if (session) return p.session === Number(session[1]);
  if (p.name.toLowerCase().includes(f) || p.title.toLowerCase().includes(f)) return true;
  if (info && (info.description.toLowerCase().includes(f) || info.user.toLowerCase().includes(f))) return true;
  return /^\d+$/.test(f) && String(p.pid).startsWith(f);
}

/** What a row is sorted by in a column: a number, or text for the text columns. */
export function sortValue(row: Row, key: ColumnKey, info: InfoOf): number | string {
  const p = row.rep;
  const one = row.kind === "proc" ? p : null;
  const facts = p ? info(p) : undefined;
  switch (key) {
    case "name":
      return row.name;
    case "status":
      return statusText(row.flags);
    case "pid":
      return one ? one.pid : -1;
    case "cpu":
      return row.cpu;
    case "mem":
      return row.mem;
    case "disk":
      return row.disk;
    case "gpu":
      return row.gpu;
    case "threads":
      return row.threads;
    case "handles":
      return row.handles;
    case "ws":
      return row.ws;
    case "commit":
      return row.commit;
    case "cputime":
      return row.cpuTimeMs;
    case "priority":
      return one ? one.priority : -1;
    case "session":
      return p ? p.session : -1;
    case "started":
      return one ? one.startMs : 0;
    case "user":
      return facts?.user ?? "";
    case "arch":
      return facts?.architecture ?? "";
    case "elevated":
      return facts?.elevated ? "Yes" : facts?.elevated === false ? "No" : "";
    case "description":
      return facts?.description ?? "";
    case "publisher":
      return facts?.company ?? "";
    case "path":
      return facts?.path ?? "";
    case "command":
      return one ? (facts?.command ?? "") : "";
    case "type":
      return row.section ? SECTION_LABEL[row.section] : "";
  }
}

function compare(a: Row, b: Row, key: ColumnKey, desc: boolean, info: InfoOf): number {
  const va = sortValue(a, key, info);
  const vb = sortValue(b, key, info);
  let d: number;
  if (typeof va === "number" && typeof vb === "number") d = va - vb;
  else d = String(va).localeCompare(String(vb), undefined, { sensitivity: "base", numeric: true });
  if (d !== 0) return desc ? -d : d;
  // Equal values: keep a fixed order so idle rows do not shuffle on every refresh.
  return a.pid - b.pid;
}

function procRow(p: Proc, child: boolean, name: string): Row {
  return {
    key: `p:${p.pid}`,
    kind: "proc",
    child,
    name,
    pid: p.pid,
    count: 1,
    open: false,
    section: sectionOf(p.flags),
    rep: p,
    cpu: p.cpu,
    mem: p.mem,
    disk: p.diskRead + p.diskWrite,
    gpu: p.gpu,
    threads: p.threads,
    handles: p.handles,
    ws: p.ws,
    commit: p.commit,
    cpuTimeMs: p.cpuTimeMs,
    flags: p.flags,
  };
}

function groupRow(k: string, members: readonly Proc[], name: string, open: boolean): Row {
  let first = members[0];
  const row: Row = {
    key: `g:${k}`,
    kind: "group",
    child: false,
    name,
    pid: first.pid,
    count: members.length,
    open,
    section: null,
    rep: first,
    cpu: 0,
    mem: 0,
    disk: 0,
    gpu: 0,
    threads: 0,
    handles: 0,
    ws: 0,
    commit: 0,
    cpuTimeMs: 0,
    flags: 0,
  };
  let any = 0;
  let all = ~0;
  for (const m of members) {
    row.cpu += m.cpu;
    row.mem += m.mem;
    row.disk += m.diskRead + m.diskWrite;
    row.gpu = Math.max(row.gpu, m.gpu);
    row.threads += m.threads;
    row.handles += m.handles;
    row.ws += m.ws;
    row.commit += m.commit;
    row.cpuTimeMs += m.cpuTimeMs;
    any |= m.flags;
    all &= m.flags;
    if (m.pid < first.pid) first = m;
  }
  row.cpu = Math.min(row.cpu, 100);
  row.pid = first.pid;
  row.rep = first;
  // One window or one frozen window marks the program; "suspended", "efficiency" and
  // "part of Windows" only when it is true for every process of it.
  row.flags = (any & (F_APP | F_HUNG)) | (all & (F_SUSPENDED | F_WINDOWS | F_ECO));
  row.section = sectionOf(row.flags);
  return row;
}

/** Filters, groups and sorts the process list into the rows to draw. */
export function buildRows(procs: readonly Proc[], opts: ViewOptions): Row[] {
  const { info } = opts;
  const shown = procs.filter((p) => matches(p, opts.filter, info(p)));
  const sort = (rows: Row[]) => rows.sort((a, b) => compare(a, b, opts.sortKey, opts.desc, info));

  if (opts.view === "list") return sort(shown.map((p) => procRow(p, false, p.name)));

  // One row per program: its processes are gathered under the program's file name.
  const groups = new Map<string, Proc[]>();
  for (const p of shown) {
    const k = p.name.toLowerCase();
    const list = groups.get(k);
    if (list) list.push(p);
    else groups.set(k, [p]);
  }
  const friendly = opts.view === "type";
  const top: { row: Row; members: Proc[] }[] = [];
  for (const [k, members] of groups) {
    if (members.length === 1) {
      const p = members[0];
      top.push({ row: procRow(p, false, friendly ? friendlyName(p, info(p)) : p.name), members });
      continue;
    }
    const lead = members.reduce((a, b) => (b.pid < a.pid ? b : a));
    const name = friendly ? friendlyName(lead, info(lead)) : lead.name;
    top.push({ row: groupRow(k, members, name, opts.expanded.has(k)), members });
  }

  const withChildren = (list: { row: Row; members: Proc[] }[]): Row[] => {
    const rows = sort(list.map((t) => t.row));
    const members = new Map(list.map((t) => [t.row.key, t.members]));
    const out: Row[] = [];
    for (const row of rows) {
      out.push(row);
      if (row.kind === "group" && row.open) {
        // Inside an app, a process is best known by its window title.
        const kids = (members.get(row.key) ?? []).map((p) => procRow(p, true, (friendly && p.title) || p.name));
        out.push(...sort(kids));
      }
    }
    return out;
  };

  if (opts.view === "name") return withChildren(top);

  const out: Row[] = [];
  for (const section of SECTIONS) {
    const inSection = top.filter((t) => t.row.section === section);
    if (!inSection.length) continue;
    const open = !opts.collapsed.has(section);
    out.push({
      key: `s:${section}`,
      kind: "section",
      child: false,
      name: SECTION_LABEL[section],
      pid: 0,
      count: inSection.length,
      open,
      section,
      rep: null,
      cpu: 0,
      mem: 0,
      disk: 0,
      gpu: 0,
      threads: 0,
      handles: 0,
      ws: 0,
      commit: 0,
      cpuTimeMs: 0,
      flags: 0,
    });
    if (open) out.push(...withChildren(inSection));
  }
  return out;
}

/**
 * Which processes a row stands for: itself, or every member of the group. Used for
 * "end all" and for adding up what one person runs.
 */
export function membersOf(procs: readonly Proc[], row: Row): Proc[] {
  if (row.kind === "proc") return row.rep ? procs.filter((p) => p.pid === row.pid) : [];
  if (row.kind === "group") {
    const k = row.key.slice(2);
    return procs.filter((p) => p.name.toLowerCase() === k);
  }
  return [];
}

export interface Totals {
  count: number;
  cpu: number;
  mem: number;
  disk: number;
}

/** Adds up what one sign-in session runs (the Users tab). */
export function sessionTotals(procs: readonly Proc[], session: number): Totals {
  const t: Totals = { count: 0, cpu: 0, mem: 0, disk: 0 };
  for (const p of procs) {
    if (p.session !== session) continue;
    t.count++;
    t.cpu += p.cpu;
    t.mem += p.mem;
    t.disk += p.diskRead + p.diskWrite;
  }
  t.cpu = Math.min(t.cpu, 100);
  return t;
}

/** Turns a processor bit mask (as text) into the list of processor numbers and back. */
export function maskToCpus(mask: string): number[] {
  let m: bigint;
  try {
    m = BigInt(mask);
  } catch {
    return [];
  }
  const out: number[] = [];
  for (let i = 0; i < 64; i++) if ((m >> BigInt(i)) & 1n) out.push(i);
  return out;
}

export function cpusToMask(cpus: readonly number[]): string {
  let m = 0n;
  for (const c of cpus) if (Number.isInteger(c) && c >= 0 && c < 64) m |= 1n << BigInt(c);
  return m.toString();
}

/** A change the user picked that Windows has not confirmed yet. */
export interface PendingPick {
  /** Which process (see `procKey`). */
  key: string;
  field: string;
  apply: (p: Proc) => void;
  /** Refreshes it is still shown for; endless while the request is on its way. */
  left: number;
}

/**
 * Changes the user picked (suspend, priority, efficiency mode) that are shown at once.
 *
 * The list is replaced by a fresh reading every refresh, and a reading may have been taken just
 * before Windows applied the change. So a pick is put back on top of every fresh reading until
 * Windows has answered, and for one refresh after that. A refused pick is taken back.
 */
export class Picks {
  private readonly waiting = new Map<string, PendingPick>();

  private static id(pick: PendingPick): string {
    return `${pick.key}|${pick.field}`;
  }

  private isCurrent(pick: PendingPick): boolean {
    return this.waiting.get(Picks.id(pick)) === pick;
  }

  get size(): number {
    return this.waiting.size;
  }

  /** Shows a pick now. A newer pick for the same thing on the same process replaces an older one. */
  start(procs: readonly Proc[], p: Pick<Proc, "pid" | "startMs">, field: string, apply: (p: Proc) => void): PendingPick {
    const pick: PendingPick = { key: procKey(p), field, apply, left: Infinity };
    this.waiting.set(Picks.id(pick), pick);
    const live = procs.find((x) => procKey(x) === pick.key);
    if (live) apply(live);
    return pick;
  }

  /** Windows accepted it: show it for one more refresh, then trust the readings again. */
  confirm(procs: readonly Proc[], pick: PendingPick): void {
    if (!this.isCurrent(pick)) return;
    pick.left = 1;
    const live = procs.find((x) => procKey(x) === pick.key);
    if (live) pick.apply(live);
  }

  /** Windows refused it: take it back, unless a newer pick for the same thing is showing. */
  refuse(procs: readonly Proc[], pick: PendingPick, undo: (p: Proc) => void): void {
    if (!this.isCurrent(pick)) return;
    this.waiting.delete(Picks.id(pick));
    const live = procs.find((x) => procKey(x) === pick.key);
    if (live) undo(live);
  }

  /** Puts every waiting pick on top of a fresh reading. Picks for ended processes are dropped. */
  applyTo(procs: readonly Proc[]): void {
    if (!this.waiting.size) return;
    const byKey = new Map(procs.map((p) => [procKey(p), p]));
    for (const [id, pick] of this.waiting) {
      const live = byKey.get(pick.key);
      if (!live) {
        this.waiting.delete(id);
        continue;
      }
      pick.apply(live);
      if (pick.left !== Infinity && --pick.left <= 0) this.waiting.delete(id);
    }
  }
}

/**
 * Keeps rows where they were while the user is aiming at one.
 *
 * With a live sort (by CPU, say) rows trade places every second, so the row under the pointer
 * can change between looking and clicking. While the pointer is busy in the list we keep the
 * previous order: numbers still update, rows that ended disappear, new rows go to the bottom.
 */
export function keepOrder(previousKeys: readonly string[], next: readonly Row[]): Row[] {
  const byKey = new Map(next.map((r) => [r.key, r]));
  const out: Row[] = [];
  for (const k of previousKeys) {
    const r = byKey.get(k);
    if (r) {
      out.push(r);
      byKey.delete(k);
    }
  }
  for (const r of next) {
    if (byKey.has(r.key)) out.push(r);
  }
  return out;
}

/** Which slice of a long list is on screen. `end` is exclusive. */
export function visibleRange(
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  count: number,
  overscan = 4,
): { start: number; end: number } {
  if (count <= 0 || rowHeight <= 0 || viewportHeight <= 0) return { start: 0, end: 0 };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const visible = Math.ceil(viewportHeight / rowHeight) + 1;
  const start = Math.max(0, Math.min(first - overscan, count));
  const end = Math.min(count, first + visible + overscan);
  return { start: Math.min(start, end), end };
}

const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "AltLeft",
  "AltRight",
  "MetaLeft",
  "MetaRight",
]);

export interface KeyLike {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/** Esc pressed on its own. With Ctrl, Alt or Shift held it is part of a shortcut instead. */
export function isPlainEscape(e: KeyLike): boolean {
  return e.code === "Escape" && !e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey;
}

/**
 * Turns a key press into a global-shortcut text such as "Ctrl+Alt+Escape".
 * Returns null while only modifiers are held, and for a plain key with no modifier
 * (a global shortcut on plain "A" would break typing everywhere). F-keys may stand alone.
 */
export function hotkeyFromEvent(e: KeyLike): string | null {
  if (!e.code || MODIFIER_CODES.has(e.code)) return null;
  let key = e.code;
  if (/^Key[A-Z]$/.test(key)) key = key.slice(3);
  else if (/^Digit\d$/.test(key)) key = key.slice(5);
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  const isFunctionKey = /^F([1-9]|1\d|2[0-4])$/.test(key);
  const onlyShift = mods.length === 1 && mods[0] === "Shift";
  if (!isFunctionKey && (mods.length === 0 || onlyShift)) return null;
  return [...mods, key].join("+");
}


/**
 * Remembers each program's icon. A miss is asked again a few times (waiting longer each time)
 * before the placeholder is final: one failed answer must never leave a program blank for good.
 */
export class IconBook {
  private readonly urls = new Map<string, string | null>();
  private readonly pending = new Set<string>();
  private readonly misses = new Map<string, number>();
  private readonly retryAt = new Map<string, number>();

  constructor(
    private readonly tries = 3,
    private readonly retryMs = 2000,
  ) {}

  /** The icon, `null` when it is settled that there is none, `undefined` while not known yet. */
  known(key: string): string | null | undefined {
    return this.urls.get(key);
  }

  /** Whether to ask for this icon now. Saying yes marks it as being asked. */
  shouldAsk(key: string, now: number): boolean {
    if (this.urls.has(key) || this.pending.has(key) || now < (this.retryAt.get(key) ?? 0)) return false;
    this.pending.add(key);
    return true;
  }

  /** Records the answer. Returns true when an icon arrived, so the list should repaint. */
  answer(key: string, url: string | null, now: number): boolean {
    this.pending.delete(key);
    if (url) {
      this.urls.set(key, url);
      return true;
    }
    const misses = (this.misses.get(key) ?? 0) + 1;
    this.misses.set(key, misses);
    if (misses >= this.tries) this.urls.set(key, null);
    else this.retryAt.set(key, now + this.retryMs * misses);
    return false;
  }
}
