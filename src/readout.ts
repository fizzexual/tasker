// What to show beside a percentage: the real value behind it (speed, GB used of total, transfer
// speed, temperature, the kind of disk). Pure text building, so it can be tested without a screen.

import type { Hardware, PerfSample } from "./api";
import { bytes, count, percent0, rateFull, speed } from "./format";

export interface Readout {
  /** "37%" */
  pct: string;
  /** The same share as a whole number from 0 to 100, for the gauge under the readout. */
  fill: number;
  /** The value behind it, "" when nothing is known. */
  more: string;
}
export interface NamedReadout extends Readout {
  label: string;
}

/** "13.0 / 32.0 GB": how much of how much, naming the unit once when both sides share it. */
export function usedOf(used: number, total: number): string {
  if (!(total > 0)) return bytes(used);
  const [a, unitA] = bytes(used).split(" ");
  const [b, unitB] = bytes(total).split(" ");
  return unitA === unitB ? `${a} / ${b} ${unitB}` : `${a} ${unitA} / ${b} ${unitB}`;
}

const share = (used: number, total: number) => (total > 0 ? (used / total) * 100 : 0);
const gauge = (v: number) => (Number.isFinite(v) ? Math.min(100, Math.max(0, Math.round(v))) : 0);
const degrees = (t: number | null | undefined) => (t ? `${Math.round(t)} °C` : "");
const join = (...parts: string[]) => parts.filter(Boolean).join(" · ");

export function cpuReadout(s: PerfSample): Readout {
  return { pct: percent0(s.cpu), fill: gauge(s.cpu), more: s.cpu_mhz ? speed(s.cpu_mhz) : "" };
}

export function memReadout(s: PerfSample): Readout {
  const used = share(s.mem_used, s.mem_total);
  return { pct: percent0(used), fill: gauge(used), more: s.mem_total > 0 ? usedOf(s.mem_used, s.mem_total) : "" };
}

/** The busiest graphics card. */
export function gpuReadout(s: PerfSample): NamedReadout | null {
  if (!s.gpus.length) return null;
  const g = s.gpus.reduce((a, b) => (b.util > a.util ? b : a));
  return { label: "GPU", pct: percent0(g.util), fill: gauge(g.util), more: join(usedOf(g.mem_used, g.mem_total), degrees(g.temp)) };
}

/** A disk's name in samples is its number, then its drive letters: "0 C: D:". */
function diskParts(name: string): { index: number; letters: string } {
  const [first, ...rest] = name.trim().split(/\s+/);
  return { index: Number.parseInt(first, 10), letters: rest.join(" ") };
}

function diskKind(name: string, hw: Hardware | null): string {
  const { index } = diskParts(name);
  return hw?.disks.find((d) => d.index === index)?.kind ?? "";
}

/** "SSD C:", "HDD D: E:", "Disk 2": the kind in one word, then the letters (or the number). */
export function diskLabel(name: string, hw: Hardware | null): string {
  const { index, letters } = diskParts(name);
  const short = /^(SSD|HDD)/i.exec(diskKind(name, hw))?.[1].toUpperCase() ?? "Disk";
  return `${short} ${letters || (Number.isFinite(index) ? index : name)}`;
}

/** The busiest disk. */
export function diskReadout(s: PerfSample, hw: Hardware | null): NamedReadout | null {
  if (!s.disks.length) return null;
  const d = s.disks.reduce((a, b) => (b.active > a.active ? b : a));
  return { label: diskLabel(d.name, hw), pct: percent0(d.active), fill: gauge(d.active), more: rateFull(d.read + d.write) };
}

export type SideKind = "cpu" | "mem" | "disk" | "net" | "gpu";

/** The two lines under a name in the Performance list. */
export function sideLines(kind: SideKind, name: string, s: PerfSample, hw: Hardware | null): [string, string] {
  switch (kind) {
    case "cpu": {
      const r = cpuReadout(s);
      return [join(r.pct, r.more), `${count(s.procs)} processes`];
    }
    case "mem": {
      const r = memReadout(s);
      return [join(r.pct, r.more), `${bytes(Math.max(0, s.mem_total - s.mem_used))} free`];
    }
    case "disk": {
      const d = s.disks.find((x) => x.name === name);
      return [join(percent0(d?.active ?? 0), d ? diskKind(name, hw) : ""), `R ${rateFull(d?.read ?? 0)} · W ${rateFull(d?.write ?? 0)}`];
    }
    case "net": {
      const n = s.nets.find((x) => x.name === name);
      return [`↓ ${rateFull(n?.rx ?? 0)}`, `↑ ${rateFull(n?.tx ?? 0)}`];
    }
    case "gpu": {
      const g = s.gpus.find((x) => x.name === name);
      return [join(percent0(g?.util ?? 0), degrees(g?.temp)), usedOf(g?.mem_used ?? 0, g?.mem_total ?? 0)];
    }
  }
}
