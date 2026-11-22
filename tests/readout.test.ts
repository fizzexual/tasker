import { describe, expect, it } from "vitest";
import type { Hardware, PerfSample } from "../src/api";
import { cpuReadout, diskLabel, diskReadout, gpuReadout, memReadout, sideLines, usedOf } from "../src/readout";

const GB = 1024 ** 3;
const MB = 1024 ** 2;

function sample(over: Partial<PerfSample> = {}): PerfSample {
  return {
    t: 0,
    cpu: 37.2,
    cores: [],
    mem_used: 13 * GB,
    mem_total: 32 * GB,
    swap_used: 0,
    swap_total: 0,
    disks: [
      { name: "0 C:", active: 28.4, read: 1.2 * MB, write: 300 * 1024, response_ms: 1 },
      { name: "1 D:", active: 3, read: 0, write: 0, response_ms: 0 },
    ],
    nets: [{ name: "Ethernet", rx: 2.7 * MB, tx: 575 * 1024 }],
    gpus: [{ name: "Example GPU", util: 2.4, mem_used: 1.2 * GB, mem_total: 8 * GB, shared_used: 0, temp: 38.4, engines: [] }],
    procs: 420,
    threads: 5200,
    handles: 180000,
    cpu_mhz: 4450,
    committed: 0,
    commit_limit: 0,
    cached: 0,
    paged_pool: 0,
    nonpaged_pool: 0,
    ...over,
  };
}

const hw = {
  disks: [
    { index: 0, model: "Example NVMe 1TB", kind: "SSD (NVMe)", size: 1e12 },
    { index: 1, model: "Example HDD 2TB", kind: "HDD", size: 2e12 },
  ],
} as Hardware;

describe("usedOf (how much of how much)", () => {
  it("says the unit once when both sides share it", () => {
    expect(usedOf(13 * GB, 32 * GB)).toBe("13.0 / 32.0 GB");
  });
  it("keeps both units when they differ", () => {
    expect(usedOf(512 * MB, 8 * GB)).toBe("512 MB / 8.0 GB");
  });
  it("an unknown total shows only what is used", () => {
    expect(usedOf(2 * GB, 0)).toBe("2.0 GB");
  });
});

describe("top bar readouts: a percentage AND the real value", () => {
  it("CPU: percent and current speed", () => {
    expect(cpuReadout(sample())).toEqual({ pct: "37%", fill: 37, more: "4.45 GHz" });
  });
  it("CPU: no speed known leaves the value empty, never a dash on its own", () => {
    expect(cpuReadout(sample({ cpu_mhz: 0 }))).toEqual({ pct: "37%", fill: 37, more: "" });
  });
  it("memory: percent and used of total", () => {
    expect(memReadout(sample())).toEqual({ pct: "41%", fill: 41, more: "13.0 / 32.0 GB" });
  });
  it("memory: no total is 0%, not a division by zero", () => {
    expect(memReadout(sample({ mem_total: 0, mem_used: 0 }))).toEqual({ pct: "0%", fill: 0, more: "" });
  });
  it("GPU: the busiest one, with its memory and temperature", () => {
    const s = sample({
      gpus: [
        { name: "Idle GPU", util: 1, mem_used: 0, mem_total: 2 * GB, shared_used: 0, temp: null, engines: [] },
        { name: "Busy GPU", util: 64.6, mem_used: 6 * GB, mem_total: 8 * GB, shared_used: 0, temp: 71.2, engines: [] },
      ],
    });
    expect(gpuReadout(s)).toEqual({ label: "GPU", pct: "65%", fill: 65, more: "6.0 / 8.0 GB · 71 °C" });
  });
  it("GPU: no temperature sensor simply leaves it out", () => {
    const s = sample({ gpus: [{ name: "G", util: 5, mem_used: GB, mem_total: 4 * GB, shared_used: 0, temp: null, engines: [] }] });
    expect(gpuReadout(s)).toEqual({ label: "GPU", pct: "5%", fill: 5, more: "1.0 / 4.0 GB" });
  });
  it("GPU: a PC without one has no GPU readout", () => {
    expect(gpuReadout(sample({ gpus: [] }))).toBeNull();
  });
  it("disk: the busiest one, named by its kind and letter, with its transfer speed", () => {
    expect(diskReadout(sample(), hw)).toEqual({ label: "SSD C:", pct: "28%", fill: 28, more: "1.5 MB/s" });
  });
  it("disk: a spinning disk is called HDD; without hardware facts it is just a disk", () => {
    const s = sample({ disks: [{ name: "1 D:", active: 90, read: 0, write: 10 * MB, response_ms: 0 }] });
    expect(diskReadout(s, hw)).toEqual({ label: "HDD D:", pct: "90%", fill: 90, more: "10.0 MB/s" });
    expect(diskReadout(s, null)).toEqual({ label: "Disk D:", pct: "90%", fill: 90, more: "10.0 MB/s" });
  });
  it("the gauge number never leaves 0 to 100, whatever the reading says", () => {
    expect(cpuReadout(sample({ cpu: 104.2 })).fill).toBe(100);
    expect(cpuReadout(sample({ cpu: -3 })).fill).toBe(0);
    expect(cpuReadout(sample({ cpu: Number.NaN })).fill).toBe(0);
    expect(memReadout(sample({ mem_used: 40 * GB, mem_total: 32 * GB })).fill).toBe(100);
  });
  it("disk: none at all has no readout", () => {
    expect(diskReadout(sample({ disks: [] }), hw)).toBeNull();
  });
});

describe("diskLabel", () => {
  it("uses the short kind and the drive letters", () => {
    expect(diskLabel("0 C:", hw)).toBe("SSD C:");
    expect(diskLabel("1 D: E:", hw)).toBe("HDD D: E:");
  });
  it("a disk without a letter keeps its number", () => {
    expect(diskLabel("2", hw)).toBe("Disk 2");
    expect(diskLabel("0", hw)).toBe("SSD 0");
  });
});

describe("Performance side list: two lines, never a bare percentage", () => {
  it("CPU", () => {
    expect(sideLines("cpu", "", sample(), hw)).toEqual(["37% · 4.45 GHz", "420 processes"]);
  });
  it("memory", () => {
    expect(sideLines("mem", "", sample(), hw)).toEqual(["41% · 13.0 / 32.0 GB", "19.0 GB free"]);
  });
  it("disk: kind, how busy, and read / write speed", () => {
    expect(sideLines("disk", "0 C:", sample(), hw)).toEqual(["28% · SSD (NVMe)", "R 1.2 MB/s · W 300 KB/s"]);
    expect(sideLines("disk", "1 D:", sample(), hw)).toEqual(["3% · HDD", "R 0 B/s · W 0 B/s"]);
    expect(sideLines("disk", "1 D:", sample(), null)).toEqual(["3%", "R 0 B/s · W 0 B/s"]);
  });
  it("network", () => {
    expect(sideLines("net", "Ethernet", sample(), hw)).toEqual(["↓ 2.7 MB/s", "↑ 575 KB/s"]);
  });
  it("GPU: use and temperature, then memory", () => {
    expect(sideLines("gpu", "Example GPU", sample(), hw)).toEqual(["2% · 38 °C", "1.2 / 8.0 GB"]);
  });
  it("something that has gone (unplugged disk) reads as zero, not an error", () => {
    expect(sideLines("disk", "9 Z:", sample(), hw)).toEqual(["0%", "R 0 B/s · W 0 B/s"]);
    expect(sideLines("gpu", "nope", sample(), hw)).toEqual(["0%", "0 B"]);
  });
});
