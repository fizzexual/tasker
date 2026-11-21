import { describe, expect, it } from "vitest";
import { cleanColumns, clock, COLUMNS, DEFAULT_COLUMNS, gridTemplate, INFO_COLUMNS } from "../src/columns";
import { buildRows, F_APP, F_HUNG, type Proc, type StaticInfo, type ViewOptions } from "../src/model";

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
  startMs: 0,
  ws: 0,
  commit: 0,
  flags: 0,
  title: "",
  ...o,
});
const opts = (o: Partial<ViewOptions> = {}): ViewOptions => ({
  view: "list",
  sortKey: "pid",
  desc: false,
  filter: "",
  expanded: new Set(),
  collapsed: new Set(),
  info: () => undefined,
  ...o,
});
const info: StaticInfo = { path: "C:\\x.exe", user: "me", description: "X", company: "X Ltd", architecture: "x64", elevated: true, command: "x --y" };
const col = (key: string) => COLUMNS.find((c) => c.key === key)!;

describe("clock (CPU time)", () => {
  it("writes hours:minutes:seconds", () => {
    expect(clock(73_000)).toBe("0:01:13");
    expect(clock(3_723_000)).toBe("1:02:03");
    expect(clock(999)).toBe("0:00:00");
  });
  it("goes past a day without wrapping", () => {
    expect(clock(100 * 3600 * 1000)).toBe("100:00:00");
  });
  it("a negative time shows as zero", () => {
    expect(clock(-5000)).toBe("0:00:00");
  });
});

describe("cleanColumns (the saved column choice)", () => {
  it("anything that is not a list gives the default columns", () => {
    expect(cleanColumns(undefined)).toEqual([...DEFAULT_COLUMNS]);
    expect(cleanColumns("cpu")).toEqual([...DEFAULT_COLUMNS]);
    expect(cleanColumns({ cpu: true })).toEqual([...DEFAULT_COLUMNS]);
  });
  it("drops unknown names and repeats, and puts the rest in the table's order", () => {
    expect(cleanColumns(["gpu", "pid", "nope", 7, null, "pid"])).toEqual(["pid", "gpu"]);
  });
  it("an empty choice stays empty: only the Name column is shown", () => {
    expect(cleanColumns([])).toEqual([]);
  });
  it("the Name column is always there, so it cannot be chosen or removed", () => {
    expect(cleanColumns(["name", "cpu"])).toEqual(["cpu"]);
  });
  it("every default column exists", () => {
    expect(cleanColumns([...DEFAULT_COLUMNS])).toEqual([...DEFAULT_COLUMNS]);
  });
});

describe("gridTemplate", () => {
  it("is the Name column and then each chosen column at its width", () => {
    expect(gridTemplate(["pid", "cpu"])).toEqual({ template: "minmax(200px, 1fr) 62px 62px", minWidth: 356 });
  });
  it("does not depend on the order the columns were picked in", () => {
    expect(gridTemplate(["cpu", "pid"])).toEqual(gridTemplate(["pid", "cpu"]));
  });
  it("with no columns only the Name column is left", () => {
    expect(gridTemplate([])).toEqual({ template: "minmax(200px, 1fr)", minWidth: 216 });
  });
});

describe("what the cells say", () => {
  const group = buildRows([p(5, "x"), p(6, "x")], opts({ view: "name" }))[0];
  const single = buildRows([p(7, "y", { priority: 13, session: 2, cpuTimeMs: 61_000, flags: F_APP | F_HUNG })], opts())[0];

  it("a single process shows its own PID, priority, session, status and CPU time", () => {
    expect(col("pid").text(single, info)).toBe("7");
    expect(col("priority").text(single, info)).toBe("High");
    expect(col("session").text(single, info)).toBe("2");
    expect(col("status").text(single, info)).toBe("Not responding");
    expect(col("cputime").text(single, info)).toBe("0:01:01");
    expect(col("type").text(single, info)).toBe("Apps");
  });
  it("a group leaves blank what only one process can have", () => {
    expect(col("pid").text(group, info)).toBe("");
    expect(col("priority").text(group, info)).toBe("");
    expect(col("started").text(group, info)).toBe("");
    expect(col("command").text(group, info)).toBe("");
  });
  it("facts that have not arrived yet show as blank, never as 'undefined'", () => {
    for (const key of INFO_COLUMNS) expect(col(key).text(single, undefined)).toBe("");
  });
  it("facts are written out once known", () => {
    expect(col("user").text(single, info)).toBe("me");
    expect(col("elevated").text(single, info)).toBe("Yes");
    expect(col("elevated").text(single, { ...info, elevated: false })).toBe("No");
    expect(col("elevated").text(single, { ...info, elevated: null })).toBe("");
    expect(col("publisher").text(single, info)).toBe("X Ltd");
    expect(col("path").text(single, info)).toBe("C:\\x.exe");
  });
  it("a process with no start time shows no date instead of 1970", () => {
    expect(col("started").text(single, info)).toBe("");
  });
});
