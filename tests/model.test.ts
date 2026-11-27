import { describe, expect, it } from "vitest";
import {
  buildRows,
  cpusToMask,
  F_APP,
  F_ECO,
  F_HUNG,
  F_SUSPENDED,
  F_WINDOWS,
  friendlyName,
  fromRaw,
  hotkeyFromEvent,
  isPlainEscape,
  keepOrder,
  maskToCpus,
  matches,
  membersOf,
  IconBook,
  Picks,
  priorityName,
  procKey,
  sectionOf,
  sessionTotals,
  sortValue,
  statusText,
  visibleRange,
  type Proc,
  type Row,
  type StaticInfo,
  type ViewOptions,
} from "../src/model";

const p = (pid: number, name: string, o: Partial<Proc> = {}): Proc => ({
  pid,
  ppid: 4,
  name,
  cpu: 0,
  mem: 0,
  diskRead: 0,
  diskWrite: 0,
  gpu: 0,
  threads: 1,
  handles: 10,
  session: 1,
  priority: 8,
  cpuTimeMs: 0,
  startMs: 1_000_000 + pid,
  creationTime: "1",
  ws: 0,
  commit: 0,
  flags: 0,
  title: "",
  ...o,
});

const facts = (o: Partial<StaticInfo> = {}): StaticInfo => ({
  path: "",
  user: "",
  description: "",
  company: "",
  architecture: "",
  elevated: null,
  command: "",
  ...o,
});

const opts = (o: Partial<ViewOptions> = {}): ViewOptions => ({
  view: "list",
  sortKey: "cpu",
  desc: true,
  filter: "",
  expanded: new Set(),
  collapsed: new Set(),
  info: () => undefined,
  ...o,
});

const keys = (rows: { key: string }[]) => rows.map((r) => r.key);

describe("fromRaw", () => {
  it("reads the compact array the backend sends", () => {
    expect(fromRaw([12, 4, "a.exe", 1.5, 2048, 10, 20, 7, 3, 99, 1, 8, 5000, "133444736000000001", 4096, 8192, F_APP | F_ECO, "A window"])).toEqual({
      pid: 12,
      ppid: 4,
      name: "a.exe",
      cpu: 1.5,
      mem: 2048,
      diskRead: 10,
      diskWrite: 20,
      gpu: 7,
      threads: 3,
      handles: 99,
      session: 1,
      priority: 8,
      cpuTimeMs: 5000,
      startMs: 1700000000000,
      creationTime: "133444736000000001",
      ws: 4096,
      commit: 8192,
      flags: F_APP | F_ECO,
      title: "A window",
    });
  });
  it("a reused PID is a different process", () => {
    expect(procKey(p(8, "a", { creationTime: "1" }))).not.toBe(procKey(p(8, "a", { creationTime: "2" })));
  });
  it("clears a selection when a same-named replacement starts within the same millisecond", () => {
    const first = p(8, "worker.exe", { creationTime: "134000000000000001" });
    const replacement = p(8, "worker.exe", { creationTime: "134000000000000002" });
    const [selected] = buildRows([first], opts());
    expect(buildRows([first], opts())[0].key).toBe(selected.key);
    expect(buildRows([replacement], opts()).some((row) => row.key === selected.key)).toBe(false);
    expect(membersOf([replacement], selected)).toEqual([]);
  });
});

describe("search", () => {
  it("finds by part of the name, any case", () => {
    expect(matches(p(1, "Chrome.exe"), "chro")).toBe(true);
    expect(matches(p(1, "Chrome.exe"), "CHROME")).toBe(true);
    expect(matches(p(1, "Chrome.exe"), "edge")).toBe(false);
  });
  it("finds by the window title", () => {
    expect(matches(p(1, "app.exe", { title: "Inbox — Mail" }), "inbox")).toBe(true);
  });
  it("finds by the friendly name and by the user, once those are known", () => {
    const info = facts({ description: "Google Chrome", user: "Stan" });
    expect(matches(p(1, "chrome.exe"), "google", info)).toBe(true);
    expect(matches(p(1, "chrome.exe"), "stan", info)).toBe(true);
    expect(matches(p(1, "chrome.exe"), "google")).toBe(false);
  });
  it("finds by the start of the PID when the search is a number", () => {
    expect(matches(p(1234, "a.exe"), "12")).toBe(true);
    expect(matches(p(1234, "a.exe"), "34")).toBe(false);
  });
  it("a number still matches names that contain it", () => {
    expect(matches(p(8, "app64.exe"), "64")).toBe(true);
  });
  it("blank search shows everything", () => {
    expect(matches(p(1, "a.exe"), "   ")).toBe(true);
  });
  it("session:N shows one person's processes and nothing else", () => {
    expect(matches(p(1, "a.exe", { session: 2 }), "session:2")).toBe(true);
    expect(matches(p(1, "a.exe", { session: 1 }), "Session:2")).toBe(false);
    // The name is not searched for the word "session:2".
    expect(matches(p(1, "session:2.exe", { session: 1 }), "session:2")).toBe(false);
  });
});

describe("what a process is called and how it is filed", () => {
  it("a window makes it an app, even when it is part of Windows", () => {
    expect(sectionOf(F_APP)).toBe("apps");
    expect(sectionOf(F_APP | F_WINDOWS)).toBe("apps");
    expect(sectionOf(F_WINDOWS)).toBe("windows");
    expect(sectionOf(0)).toBe("background");
    expect(sectionOf(F_ECO)).toBe("background");
  });
  it("status says the worst thing first", () => {
    expect(statusText(0)).toBe("");
    expect(statusText(F_ECO)).toBe("Efficiency mode");
    expect(statusText(F_SUSPENDED | F_ECO)).toBe("Suspended");
    expect(statusText(F_HUNG | F_SUSPENDED | F_ECO)).toBe("Not responding");
  });
  it("names the Windows priority levels and shows an odd one as its number", () => {
    expect(priorityName(8)).toBe("Normal");
    expect(priorityName(13)).toBe("High");
    expect(priorityName(4)).toBe("Low");
    expect(priorityName(9)).toBe("9");
    expect(priorityName(0)).toBe("");
  });
  it("uses the friendly name when there is one", () => {
    expect(friendlyName(p(1, "chrome.exe"), facts({ description: "Google Chrome" }))).toBe("Google Chrome");
    expect(friendlyName(p(1, "chrome.exe"), facts())).toBe("chrome.exe");
    expect(friendlyName(p(1, "chrome.exe"), undefined)).toBe("chrome.exe");
  });
});

describe("buildRows, list (every process on its own row)", () => {
  const procs = [p(1, "b.exe", { cpu: 5, mem: 100 }), p(2, "a.exe", { cpu: 20, mem: 50 }), p(3, "c.exe", { cpu: 5, mem: 300 })];

  it("sorts by CPU, highest first", () => {
    expect(keys(buildRows(procs, opts()))).toEqual(["p:2:1", "p:1:1", "p:3:1"]);
  });
  it("equal values keep a fixed order instead of shuffling", () => {
    const idle = [p(30, "x"), p(10, "y"), p(20, "z")];
    expect(keys(buildRows(idle, opts()))).toEqual(["p:10:1", "p:20:1", "p:30:1"]);
    expect(keys(buildRows([...idle].reverse(), opts()))).toEqual(["p:10:1", "p:20:1", "p:30:1"]);
  });
  it("sorts by name ascending without caring about case", () => {
    const rows = buildRows([p(1, "beta"), p(2, "Alpha"), p(3, "gamma")], opts({ sortKey: "name", desc: false }));
    expect(rows.map((r) => r.name)).toEqual(["Alpha", "beta", "gamma"]);
  });
  it("sorts by memory, and by disk counting reads and writes together", () => {
    expect(keys(buildRows(procs, opts({ sortKey: "mem" })))).toEqual(["p:3:1", "p:1:1", "p:2:1"]);
    const d = [p(1, "a", { diskRead: 5, diskWrite: 100 }), p(2, "b", { diskRead: 50 })];
    expect(keys(buildRows(d, opts({ sortKey: "disk" })))).toEqual(["p:1:1", "p:2:1"]);
  });
  it("sorts by a column that comes from the once-per-process facts", () => {
    const users: Record<number, string> = { 1: "zoe", 2: "SYSTEM", 3: "adam" };
    const info = (x: Proc) => facts({ user: users[x.pid] });
    expect(keys(buildRows(procs, opts({ sortKey: "user", desc: false, info })))).toEqual(["p:3:1", "p:2:1", "p:1:1"]);
  });
  it("same-named processes are NOT gathered in this view", () => {
    const rows = buildRows([p(1, "x.exe"), p(2, "x.exe")], opts());
    expect(rows.map((r) => r.kind)).toEqual(["proc", "proc"]);
  });
  it("applies the search", () => {
    expect(keys(buildRows(procs, opts({ filter: "a.e" })))).toEqual(["p:2:1"]);
  });
  it("an empty list gives no rows", () => {
    expect(buildRows([], opts())).toEqual([]);
    expect(buildRows([], opts({ view: "type" }))).toEqual([]);
    expect(buildRows([], opts({ view: "name" }))).toEqual([]);
  });
});

describe("buildRows, by name (one row per program)", () => {
  const procs = [
    p(10, "chrome.exe", { cpu: 3, mem: 100, threads: 2 }),
    p(11, "Chrome.exe", { cpu: 4, mem: 200, threads: 3 }),
    p(12, "chrome.exe", { cpu: 1, mem: 50, threads: 4 }),
    p(20, "solo.exe", { cpu: 5, mem: 10 }),
  ];
  const view = (o: Partial<ViewOptions> = {}) => opts({ view: "name", ...o });

  it("puts same-named processes in one row with their totals", () => {
    const rows = buildRows(procs, view());
    expect(keys(rows)).toEqual(["g:chrome.exe", "p:20:1"]);
    expect(rows[0]).toMatchObject({ kind: "group", count: 3, cpu: 8, mem: 350, threads: 9, open: false, pid: 10 });
  });
  it("a program with one process stays a plain row", () => {
    expect(buildRows(procs, view())[1]).toMatchObject({ kind: "proc", child: false, name: "solo.exe" });
  });
  it("an open group lists its processes under it, sorted the same way", () => {
    const rows = buildRows(procs, view({ expanded: new Set(["chrome.exe"]) }));
    expect(keys(rows)).toEqual(["g:chrome.exe", "p:11:1", "p:10:1", "p:12:1", "p:20:1"]);
    expect(rows[1].child).toBe(true);
    expect(rows[4].child).toBe(false);
  });
  it("a group's CPU never shows above 100%", () => {
    const hot = [p(1, "x", { cpu: 70 }), p(2, "x", { cpu: 60 })];
    expect(buildRows(hot, view())[0].cpu).toBe(100);
  });
  it("a group shows the busiest graphics use of its processes, not the sum", () => {
    const g = [p(1, "x", { gpu: 30 }), p(2, "x", { gpu: 45 })];
    expect(buildRows(g, view())[0].gpu).toBe(45);
  });
  it("search narrows what is grouped", () => {
    expect(keys(buildRows(procs, view({ filter: "solo" })))).toEqual(["p:20:1"]);
  });
  it("shows the file name even when a friendly name is known", () => {
    const info = () => facts({ description: "Google Chrome" });
    expect(buildRows(procs, view({ info }))[0].name).toBe("chrome.exe");
  });
  it("one frozen window marks the whole program; efficiency and suspended need every process", () => {
    const mixed = [p(1, "x", { flags: F_APP | F_HUNG | F_ECO }), p(2, "x", { flags: F_ECO }), p(3, "x", { flags: F_ECO | F_SUSPENDED })];
    const row = buildRows(mixed, view())[0];
    expect(row.flags & F_APP).toBeTruthy();
    expect(row.flags & F_HUNG).toBeTruthy();
    expect(row.flags & F_ECO).toBeTruthy();
    expect(row.flags & F_SUSPENDED).toBe(0);
    expect(statusText(row.flags)).toBe("Not responding");
  });
});

describe("buildRows, by type (Apps / Background / Windows)", () => {
  const procs = [
    p(1, "word.exe", { cpu: 1, flags: F_APP, title: "Letter.docx" }),
    p(2, "bg.exe", { cpu: 50 }),
    p(3, "svc.exe", { cpu: 2, flags: F_WINDOWS }),
    p(4, "bg2.exe", { cpu: 9 }),
  ];
  const view = (o: Partial<ViewOptions> = {}) => opts({ view: "type", ...o });

  it("files every program under its heading, apps first", () => {
    const rows = buildRows(procs, view());
    expect(keys(rows)).toEqual(["s:apps", "p:1:1", "s:background", "p:2:1", "p:4:1", "s:windows", "p:3:1"]);
    expect(rows[0]).toMatchObject({ kind: "section", name: "Apps", count: 1, open: true });
    expect(rows[2]).toMatchObject({ kind: "section", name: "Background processes", count: 2 });
  });
  it("a folded heading stays, with its count, and hides its rows", () => {
    const rows = buildRows(procs, view({ collapsed: new Set(["background"]) }));
    expect(keys(rows)).toEqual(["s:apps", "p:1:1", "s:background", "s:windows", "p:3:1"]);
    expect(rows[2]).toMatchObject({ open: false, count: 2 });
  });
  it("a heading with nothing under it is left out", () => {
    expect(keys(buildRows([procs[1]], view()))).toEqual(["s:background", "p:2:1"]);
    expect(keys(buildRows(procs, view({ filter: "svc" })))).toEqual(["s:windows", "p:3:1"]);
  });
  it("shows the friendly name, falling back to the file name", () => {
    const info = (x: Proc) => (x.pid === 1 ? facts({ description: "Microsoft Word" }) : undefined);
    const rows = buildRows(procs, view({ info }));
    expect(rows[1].name).toBe("Microsoft Word");
    expect(rows[3].name).toBe("bg.exe");
  });
  it("a program with one windowed process is an app, and its open rows carry window titles", () => {
    const chrome = [p(10, "chrome.exe", { flags: F_APP, title: "News - Chrome" }), p(11, "chrome.exe"), p(12, "chrome.exe")];
    const rows = buildRows(chrome, view({ expanded: new Set(["chrome.exe"]), sortKey: "pid", desc: false }));
    expect(keys(rows)).toEqual(["s:apps", "g:chrome.exe", "p:10:1", "p:11:1", "p:12:1"]);
    expect(rows[2].name).toBe("News - Chrome");
    expect(rows[3].name).toBe("chrome.exe");
  });
  it("sorting never moves a program out of its heading", () => {
    const rows = buildRows(procs, view({ sortKey: "cpu", desc: true }));
    // bg.exe uses the most CPU but still sits under Background, below the Apps heading.
    expect(keys(rows).indexOf("p:2:1")).toBeGreaterThan(keys(rows).indexOf("s:background"));
    expect(keys(rows)[1]).toBe("p:1:1");
  });
});

describe("sortValue", () => {
  const info = () => facts({ command: "x --flag", user: "me" });
  const group = buildRows([p(5, "x", { session: 3 }), p(6, "x", { session: 3 })], opts({ view: "name" }))[0];
  const single = buildRows([p(7, "y", { priority: 13 })], opts())[0];

  it("a group has no PID, priority, start time or command line of its own", () => {
    expect(sortValue(group, "pid", info)).toBe(-1);
    expect(sortValue(group, "priority", info)).toBe(-1);
    expect(sortValue(group, "started", info)).toBe(0);
    expect(sortValue(group, "command", info)).toBe("");
  });
  it("a group still has its user and its session", () => {
    expect(sortValue(group, "user", info)).toBe("me");
    expect(sortValue(group, "session", info)).toBe(3);
  });
  it("a single process gives its own values", () => {
    expect(sortValue(single, "pid", info)).toBe(7);
    expect(sortValue(single, "priority", info)).toBe(13);
    expect(sortValue(single, "command", info)).toBe("x --flag");
    expect(sortValue(single, "type", info)).toBe("Background processes");
  });
  it("copes with facts that have not arrived yet", () => {
    expect(sortValue(single, "user", () => undefined)).toBe("");
    expect(sortValue(single, "elevated", () => undefined)).toBe("");
    expect(sortValue(single, "elevated", () => facts({ elevated: false }))).toBe("No");
  });
});

describe("membersOf (what End task and the Users tab act on)", () => {
  const procs = [p(1, "x.exe"), p(2, "X.EXE"), p(3, "y.exe")];

  it("a group stands for every process of that program, any case", () => {
    const group = buildRows(procs, opts({ view: "name", sortKey: "name", desc: false }))[0];
    expect(membersOf(procs, group).map((m) => m.pid)).toEqual([1, 2]);
  });
  it("a process row stands for itself", () => {
    const row = buildRows([procs[2]], opts())[0];
    expect(membersOf(procs, row).map((m) => m.pid)).toEqual([3]);
  });
  it("a process that ended since the row was drawn gives nothing", () => {
    const row = buildRows([procs[2]], opts())[0];
    expect(membersOf([procs[0]], row)).toEqual([]);
  });
  it("a heading stands for nothing, so it can never be ended", () => {
    const heading = buildRows(procs, opts({ view: "type" }))[0];
    expect(heading.kind).toBe("section");
    expect(membersOf(procs, heading)).toEqual([]);
  });
});

describe("sessionTotals (the Users tab)", () => {
  const procs = [
    p(1, "a", { session: 1, cpu: 10, mem: 100, diskRead: 5, diskWrite: 7 }),
    p(2, "b", { session: 1, cpu: 15, mem: 50 }),
    p(3, "c", { session: 2, cpu: 40, mem: 999 }),
  ];
  it("adds up only that person's processes", () => {
    expect(sessionTotals(procs, 1)).toEqual({ count: 2, cpu: 25, mem: 150, disk: 12 });
  });
  it("someone with nothing running gets zeros", () => {
    expect(sessionTotals(procs, 7)).toEqual({ count: 0, cpu: 0, mem: 0, disk: 0 });
  });
  it("CPU never shows above 100%", () => {
    expect(sessionTotals([p(1, "a", { cpu: 80 }), p(2, "b", { cpu: 70 })], 1).cpu).toBe(100);
  });
});

describe("processor masks (Set affinity)", () => {
  it("reads a mask into processor numbers", () => {
    expect(maskToCpus("5")).toEqual([0, 2]);
    expect(maskToCpus("65535")).toEqual(Array.from({ length: 16 }, (_, i) => i));
    expect(maskToCpus("0")).toEqual([]);
  });
  it("writes processor numbers back into a mask", () => {
    expect(cpusToMask([0, 2])).toBe("5");
    expect(cpusToMask([2, 0, 2])).toBe("5");
    expect(cpusToMask([])).toBe("0");
  });
  it("keeps processor 63, which does not fit an ordinary JavaScript number", () => {
    expect(cpusToMask([63])).toBe("9223372036854775808");
    expect(maskToCpus("9223372036854775808")).toEqual([63]);
    expect(maskToCpus(cpusToMask([0, 31, 32, 63]))).toEqual([0, 31, 32, 63]);
  });
  it("ignores numbers that are not processors and text that is not a mask", () => {
    expect(cpusToMask([-1, 64, 1.5, 3])).toBe("8");
    expect(maskToCpus("not a number")).toEqual([]);
    expect(maskToCpus("")).toEqual([]);
  });
});

describe("Picks (a change shows at once and survives the next refresh)", () => {
  const eco = (x: Proc) => void (x.flags |= F_ECO);
  const noEco = (x: Proc) => void (x.flags &= ~F_ECO);
  const fresh = () => [p(1, "a"), p(2, "b")];

  it("shows the pick at once on the row as it is now, even when given an older copy of it", () => {
    const picks = new Picks();
    const old = p(1, "a");
    const now = fresh();
    picks.start(now, old, "efficiency", eco);
    expect(now[0].flags & F_ECO).toBeTruthy();
    expect(old.flags).toBe(0);
    expect(now[1].flags).toBe(0);
  });
  it("keeps showing it on every fresh reading while Windows has not answered", () => {
    const picks = new Picks();
    picks.start(fresh(), p(1, "a"), "efficiency", eco);
    for (let i = 0; i < 5; i++) {
      const reading = fresh();
      picks.applyTo(reading);
      expect(reading[0].flags & F_ECO).toBeTruthy();
    }
    expect(picks.size).toBe(1);
  });
  it("once accepted, covers one more reading (it may predate the change), then trusts Windows again", () => {
    const picks = new Picks();
    const pick = picks.start(fresh(), p(1, "a"), "efficiency", eco);
    picks.confirm(fresh(), pick);
    const stale = fresh();
    picks.applyTo(stale);
    expect(stale[0].flags & F_ECO).toBeTruthy();
    const truth = fresh();
    picks.applyTo(truth);
    expect(truth[0].flags).toBe(0);
    expect(picks.size).toBe(0);
  });
  it("a refused pick is taken back and never shown again", () => {
    const picks = new Picks();
    const now = fresh();
    const pick = picks.start(now, now[0], "efficiency", eco);
    picks.refuse(now, pick, noEco);
    expect(now[0].flags).toBe(0);
    const next = fresh();
    picks.applyTo(next);
    expect(next[0].flags).toBe(0);
    expect(picks.size).toBe(0);
  });
  it("two fast picks of the same thing: the late refusal of the first does not undo the second", () => {
    const picks = new Picks();
    const now = fresh();
    const first = picks.start(now, now[0], "priority", (x) => void (x.priority = 13));
    picks.start(now, now[0], "priority", (x) => void (x.priority = 6));
    picks.refuse(now, first, (x) => void (x.priority = 8));
    expect(now[0].priority).toBe(6);
    const next = fresh();
    picks.applyTo(next);
    expect(next[0].priority).toBe(6);
  });
  it("two fast picks of the same thing: the late success of the first does not end the second early", () => {
    const picks = new Picks();
    const now = fresh();
    const first = picks.start(now, now[0], "priority", (x) => void (x.priority = 13));
    picks.start(now, now[0], "priority", (x) => void (x.priority = 6));
    picks.confirm(now, first);
    for (let i = 0; i < 3; i++) {
      const next = fresh();
      picks.applyTo(next);
      expect(next[0].priority).toBe(6);
    }
  });
  it("picks of different things on one process are both kept", () => {
    const picks = new Picks();
    const now = fresh();
    picks.start(now, now[0], "efficiency", eco);
    picks.start(now, now[0], "paused", (x) => void (x.flags |= F_SUSPENDED));
    const next = fresh();
    picks.applyTo(next);
    expect(next[0].flags).toBe(F_ECO | F_SUSPENDED);
  });
  it("a pick for a process that ended is dropped, and never lands on a new process with the same PID", () => {
    const picks = new Picks();
    picks.start(fresh(), p(1, "a"), "efficiency", eco);
    const reused = [p(1, "other.exe", { creationTime: "999" })];
    picks.applyTo(reused);
    expect(reused[0].flags).toBe(0);
    expect(picks.size).toBe(0);
  });
  it("what is shown can change when Windows gives something else than asked", () => {
    const picks = new Picks();
    let shown = 24;
    const now = fresh();
    const pick = picks.start(now, now[0], "priority", (x) => void (x.priority = shown));
    expect(now[0].priority).toBe(24);
    shown = 13;
    picks.confirm(now, pick);
    expect(now[0].priority).toBe(13);
  });
});

describe("keepOrder (rows stay put while the pointer is aiming)", () => {
  const row = (key: string) => ({ key }) as Row;

  it("keeps the previous order even though the fresh sort differs", () => {
    const next = [row("p:3"), row("p:1"), row("p:2")];
    expect(keys(keepOrder(["p:1", "p:2", "p:3"], next))).toEqual(["p:1", "p:2", "p:3"]);
  });
  it("drops rows that ended and adds new ones at the bottom", () => {
    const next = [row("p:9"), row("p:3"), row("p:1")];
    expect(keys(keepOrder(["p:1", "p:2", "p:3"], next))).toEqual(["p:1", "p:3", "p:9"]);
  });
  it("uses the fresh data, not the old row objects", () => {
    const fresh = { ...row("p:1"), cpu: 42 };
    expect(keepOrder(["p:1"], [fresh])[0]).toBe(fresh);
  });
  it("with nothing before, the fresh order is used as is", () => {
    expect(keys(keepOrder([], [row("p:2"), row("p:1")]))).toEqual(["p:2", "p:1"]);
  });
});

describe("visibleRange", () => {
  it("covers the screen plus a small margin", () => {
    expect(visibleRange(0, 260, 26, 1000)).toEqual({ start: 0, end: 15 });
    expect(visibleRange(2600, 260, 26, 1000)).toEqual({ start: 96, end: 115 });
  });
  it("never runs past the end of the list", () => {
    expect(visibleRange(26 * 995, 260, 26, 1000)).toEqual({ start: 991, end: 1000 });
  });
  it("copes with a list shorter than the screen", () => {
    expect(visibleRange(0, 600, 26, 3)).toEqual({ start: 0, end: 3 });
  });
  it("a stale scroll position past a list that just shrank gives an empty, valid range", () => {
    const r = visibleRange(26 * 500, 260, 26, 10);
    expect(r.start).toBeLessThanOrEqual(r.end);
    expect(r.end).toBeLessThanOrEqual(10);
  });
  it("nothing to show for an empty list or a zero-height area", () => {
    expect(visibleRange(0, 260, 26, 0)).toEqual({ start: 0, end: 0 });
    expect(visibleRange(0, 0, 26, 50)).toEqual({ start: 0, end: 0 });
    expect(visibleRange(-50, 260, 26, 50).start).toBe(0);
  });
  it("draws only a few dozen rows however long the list is", () => {
    const r = visibleRange(123456, 600, 26, 100000);
    expect(r.end - r.start).toBeLessThan(40);
  });
});

const ev = (code: string, mods: Partial<Record<"ctrlKey" | "altKey" | "shiftKey" | "metaKey", boolean>> = {}) => ({
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe("hotkeyFromEvent", () => {
  it("builds the text in a fixed modifier order", () => {
    expect(hotkeyFromEvent(ev("Escape", { altKey: true, ctrlKey: true }))).toBe("Ctrl+Alt+Escape");
    expect(hotkeyFromEvent(ev("KeyK", { ctrlKey: true, shiftKey: true }))).toBe("Ctrl+Shift+K");
    expect(hotkeyFromEvent(ev("Digit1", { altKey: true }))).toBe("Alt+1");
    expect(hotkeyFromEvent(ev("Backquote", { metaKey: true }))).toBe("Super+Backquote");
  });
  it("waits while only modifier keys are held", () => {
    expect(hotkeyFromEvent(ev("ControlLeft", { ctrlKey: true }))).toBeNull();
    expect(hotkeyFromEvent(ev("AltRight", { altKey: true, ctrlKey: true }))).toBeNull();
  });
  it("refuses a plain key, which would break typing everywhere", () => {
    expect(hotkeyFromEvent(ev("KeyA"))).toBeNull();
    expect(hotkeyFromEvent(ev("KeyA", { shiftKey: true }))).toBeNull();
    expect(hotkeyFromEvent(ev("Space"))).toBeNull();
  });
  it("allows a function key on its own", () => {
    expect(hotkeyFromEvent(ev("F9"))).toBe("F9");
    expect(hotkeyFromEvent(ev("F12", { shiftKey: true }))).toBe("Shift+F12");
  });
  it("ignores an event with no key code", () => {
    expect(hotkeyFromEvent(ev("", { ctrlKey: true }))).toBeNull();
  });
});

describe("isPlainEscape (cancels picking a shortcut)", () => {
  it("only Esc on its own cancels", () => {
    expect(isPlainEscape(ev("Escape"))).toBe(true);
  });
  it("Esc with a modifier is a shortcut to record, not a cancel", () => {
    expect(isPlainEscape(ev("Escape", { ctrlKey: true, altKey: true }))).toBe(false);
    expect(isPlainEscape(ev("Escape", { shiftKey: true }))).toBe(false);
    expect(isPlainEscape(ev("Escape", { metaKey: true }))).toBe(false);
    // The exact case that was broken: Ctrl+Alt+Escape could not be picked.
    expect(hotkeyFromEvent(ev("Escape", { ctrlKey: true, altKey: true }))).toBe("Ctrl+Alt+Escape");
  });
  it("other keys never cancel", () => {
    expect(isPlainEscape(ev("KeyA"))).toBe(false);
  });
});

describe("IconBook (every program that has an icon ends up showing it)", () => {
  it("asks once, and not again while the answer is on its way", () => {
    const book = new IconBook();
    expect(book.known("a.exe")).toBeUndefined();
    expect(book.shouldAsk("a.exe", 0)).toBe(true);
    expect(book.shouldAsk("a.exe", 1)).toBe(false);
  });
  it("an icon that arrives is kept and asks for a repaint", () => {
    const book = new IconBook();
    book.shouldAsk("a.exe", 0);
    expect(book.answer("a.exe", "data:x", 5)).toBe(true);
    expect(book.known("a.exe")).toBe("data:x");
    expect(book.shouldAsk("a.exe", 99_999)).toBe(false);
  });
  it("a miss is asked again later instead of showing a blank for good", () => {
    // The bug: one failed answer was remembered for ever, so Brave and Discord stayed blank.
    const book = new IconBook(3, 2000);
    book.shouldAsk("a.exe", 0);
    expect(book.answer("a.exe", null, 10)).toBe(false);
    expect(book.known("a.exe")).toBeUndefined();
    expect(book.shouldAsk("a.exe", 500)).toBe(false); // not yet: do not hammer on every refresh
    expect(book.shouldAsk("a.exe", 2010)).toBe(true);
    expect(book.answer("a.exe", "data:x", 2020)).toBe(true);
    expect(book.known("a.exe")).toBe("data:x");
  });
  it("waits longer after each miss and settles on the placeholder after the last try", () => {
    const book = new IconBook(3, 2000);
    book.shouldAsk("a.exe", 0);
    book.answer("a.exe", null, 0);
    expect(book.shouldAsk("a.exe", 2000)).toBe(true);
    book.answer("a.exe", null, 2000);
    expect(book.shouldAsk("a.exe", 5999)).toBe(false); // second wait is twice as long
    expect(book.shouldAsk("a.exe", 6000)).toBe(true);
    book.answer("a.exe", null, 6000);
    expect(book.known("a.exe")).toBeNull(); // final: the placeholder
    expect(book.shouldAsk("a.exe", 999_999)).toBe(false);
  });
  it("keeps programs apart", () => {
    const book = new IconBook();
    book.shouldAsk("a.exe", 0);
    book.answer("a.exe", null, 0);
    expect(book.shouldAsk("b.exe", 1)).toBe(true);
    book.answer("b.exe", "data:b", 2);
    expect(book.known("a.exe")).toBeUndefined();
    expect(book.known("b.exe")).toBe("data:b");
  });
});
