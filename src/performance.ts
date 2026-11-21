// The Performance tab: live graphs for CPU, memory, each disk, each network and each GPU.

import { api, type Hardware, type PerfSample, type SystemInfo } from "./api";
import { drawChart, seriesColor, type Series } from "./chart";
import { bytes, count, driverDate, duration, niceCeil, percent, rateFull, speed } from "./format";
import { diskLabel, sideLines } from "./readout";
import { copy, h } from "./ui";
import { setText } from "./vlist";

/** How many readings a graph shows across its width. */
export const CAPACITY = 120;

type Kind = "cpu" | "mem" | "disk" | "net" | "gpu";

interface Item {
  key: string;
  kind: Kind;
  title: string;
  /** Name inside the samples (disk / network / GPU name). */
  name: string;
}

interface ChartSpec {
  caption: string;
  small?: boolean;
  legend?: [string, string][];
  /** Returns the series and the top of the scale. */
  data: (hist: readonly PerfSample[]) => { series: Series[]; max: number; maxLabel: string };
}

interface PanelSpec {
  sub: () => string;
  charts: ChartSpec[];
  stats: (last: PerfSample) => [string, string][];
  extra?: (last: PerfSample) => HTMLElement | null;
}

/** "4210 MHz" as "4.21 GHz". */
export { speed };

/** Link speed in bits per second, as network gear is labelled. */
export function linkSpeed(bps: number): string {
  if (!bps) return "Not connected";
  if (bps >= 1e9) return `${+(bps / 1e9).toFixed(1)} Gbps`;
  if (bps >= 1e6) return `${+(bps / 1e6).toFixed(1)} Mbps`;
  return `${Math.round(bps / 1e3)} Kbps`;
}

const disk = (s: PerfSample, name: string) => s.disks.find((d) => d.name === name);
const net = (s: PerfSample, name: string) => s.nets.find((d) => d.name === name);
const gpu = (s: PerfSample, name: string) => s.gpus.find((d) => d.name === name);

export function itemsOf(sample: PerfSample | undefined): Item[] {
  const items: Item[] = [
    { key: "cpu", kind: "cpu", title: "CPU", name: "" },
    { key: "mem", kind: "mem", title: "Memory", name: "" },
  ];
  if (!sample) return items;
  for (const d of sample.disks) items.push({ key: `disk:${d.name}`, kind: "disk", title: `Disk ${d.name}`, name: d.name });
  for (const n of sample.nets) items.push({ key: `net:${n.name}`, kind: "net", title: n.name, name: n.name });
  sample.gpus.forEach((g, i) => items.push({ key: `gpu:${g.name}`, kind: "gpu", title: `GPU ${i}`, name: g.name }));
  return items;
}

export class PerformanceTab {
  private hist: readonly PerfSample[] = [];
  private info: SystemInfo | null = null;
  private hw: Hardware | null = null;
  private items: Item[] = [];
  private selected = "cpu";
  private builtFor = "";
  private readonly side: HTMLElement;
  private readonly main: HTMLElement;
  private readonly sideRefs = new Map<string, { canvas: HTMLCanvasElement; value: HTMLElement; value2: HTMLElement; title: HTMLElement }>();
  private panel: { spec: PanelSpec; canvases: HTMLCanvasElement[]; maxLabels: HTMLElement[]; stats: HTMLElement; extra: HTMLElement; sub: HTMLElement } | null =
    null;

  constructor(root: HTMLElement) {
    this.side = h("div", { class: "perf-side", role: "tablist", "aria-label": "Resources" });
    this.main = h("div", { class: "perf-main" });
    root.append(h("div", { class: "perf" }, this.side, this.main));
    // Canvases must be redrawn when their size changes.
    new ResizeObserver(() => this.render()).observe(this.main);
    window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => this.render());
  }

  setInfo(info: SystemInfo): void {
    this.info = info;
    this.builtFor = "";
  }

  setHardware(hw: Hardware): void {
    this.hw = hw;
    this.builtFor = "";
  }

  /** What is on screen, as plain text (the "Copy" button). */
  private asText(): string {
    const last = this.hist[this.hist.length - 1];
    const item = this.items.find((i) => i.key === this.selected);
    if (!last || !item || !this.panel) return "";
    const lines = [item.title, this.panel.spec.sub()].filter(Boolean);
    for (const [k, v] of this.panel.spec.stats(last)) lines.push(`${k}: ${v}`);
    return lines.join("\n");
  }

  setHistory(hist: readonly PerfSample[]): void {
    this.hist = hist;
  }

  /** Draws the tab. Cheap to call; does nothing useful while the tab is hidden, so the caller skips it then. */
  render(): void {
    const last = this.hist[this.hist.length - 1];
    if (!last || this.main.clientWidth === 0) return;

    const items = itemsOf(last);
    const sig = items.map((i) => i.key).join("|");
    if (sig !== this.items.map((i) => i.key).join("|")) {
      this.items = items;
      this.buildSide();
    }
    if (!items.some((i) => i.key === this.selected)) this.selected = "cpu";
    if (this.builtFor !== this.selected) this.buildPanel();

    for (const item of this.items) {
      const r = this.sideRefs.get(item.key);
      if (!r) continue;
      const [line1, line2] = sideLines(item.kind, item.name, last, this.hw);
      setText(r.value, line1);
      setText(r.value2, line2);
      // A disk is named by what it is once the hardware facts are in: "SSD C:", not "Disk 0 C:".
      if (item.kind === "disk") setText(r.title, diskLabel(item.name, this.hw));
      const d = this.sideSeries(item);
      drawChart(r.canvas, d.series, { max: d.max, capacity: CAPACITY, grid: false });
    }

    const p = this.panel;
    if (!p) return;
    setText(p.sub, p.spec.sub());
    p.spec.charts.forEach((c, i) => {
      const d = c.data(this.hist);
      drawChart(p.canvases[i], d.series, { max: d.max, capacity: CAPACITY });
      setText(p.maxLabels[i], d.maxLabel);
    });
    const stats = p.spec.stats(last);
    if (p.stats.childElementCount !== stats.length) {
      p.stats.replaceChildren(
        ...stats.map(([k]) => h("div", { class: "stat" }, h("div", { class: "k" }, k), h("div", { class: "val" }))),
      );
    }
    stats.forEach(([, v], i) => setText(p.stats.children[i].lastElementChild!, v));
    const extra = p.spec.extra?.(last) ?? null;
    if (extra) p.extra.replaceChildren(extra);
  }

  // ── sidebar ──

  private buildSide(): void {
    this.sideRefs.clear();
    this.side.replaceChildren(
      ...this.items.map((item) => {
        const canvas = h("canvas");
        const value = h("div", { class: "v" });
        const value2 = h("div", { class: "v v2" });
        const title = h("div", { class: "t" }, item.title);
        this.sideRefs.set(item.key, { canvas, value, value2, title });
        return h(
          "button",
          {
            class: "perf-item",
            role: "tab",
            "aria-selected": String(item.key === this.selected),
            "data-key": item.key,
            title: item.title,
            onclick: () => {
              this.selected = item.key;
              for (const b of this.side.children) b.setAttribute("aria-selected", String((b as HTMLElement).dataset.key === item.key));
              this.render();
            },
          },
          canvas,
          h("div", { class: "txts" }, title, value, value2),
        );
      }),
    );
  }

  private sideSeries(item: Item): { series: Series[]; max: number } {
    const color = seriesColor(item.kind);
    const one = (f: (s: PerfSample) => number, max: number) => ({
      series: [{ values: this.hist.map(f), color, fill: true }],
      max,
    });
    switch (item.kind) {
      case "cpu":
        return one((s) => s.cpu, 100);
      case "mem":
        return one((s) => (s.mem_total ? (s.mem_used / s.mem_total) * 100 : 0), 100);
      case "disk":
        return one((s) => disk(s, item.name)?.active ?? 0, 100);
      case "gpu":
        return one((s) => gpu(s, item.name)?.util ?? 0, 100);
      case "net": {
        const rx = this.hist.map((s) => net(s, item.name)?.rx ?? 0);
        const tx = this.hist.map((s) => net(s, item.name)?.tx ?? 0);
        return { series: [{ values: rx, color, fill: true }, { values: tx, color }], max: niceCeil(Math.max(...rx, ...tx)) };
      }
    }
  }

  // ── main panel ──

  private buildPanel(): void {
    const item = this.items.find((i) => i.key === this.selected) ?? this.items[0];
    this.builtFor = item.key;
    const spec = this.spec(item);
    const canvases: HTMLCanvasElement[] = [];
    const maxLabels: HTMLElement[] = [];
    const sub = h("div", { class: "sub" });
    const stats = h("div", { class: "stats" });
    const extra = h("div");
    this.main.replaceChildren(
      h(
        "div",
        { class: "perf-head" },
        h("h2", null, item.title),
        sub,
        h("button", { class: "ghost", title: "Copy these numbers as text", onclick: () => copy(this.asText()) }, "Copy"),
        h(
          "button",
          { class: "ghost", title: "Open the Windows Resource Monitor", onclick: () => void api.openTool("resource-monitor").catch(() => undefined) },
          "Resource Monitor",
        ),
      ),
      ...spec.charts.map((c) => {
        const canvas = h("canvas", { class: c.small ? "chart small" : "chart" });
        const max = h("span");
        canvases.push(canvas);
        maxLabels.push(max);
        const legend = c.legend
          ? h("span", { class: "legend" }, ...c.legend.map(([label, color]) => h("span", { style: `color:${color}` }, h("i"), label)))
          : null;
        return h("div", { class: "chart-box" }, h("div", { class: "chart-cap" }, h("span", null, c.caption, legend ? "  " : "", legend), max), canvas);
      }),
      extra,
      stats,
    );
    this.panel = { spec, canvases, maxLabels, stats, extra, sub };
  }

  private spec(item: Item): PanelSpec {
    const color = seriesColor(item.kind);
    const pct = (caption: string, f: (s: PerfSample) => number, small = false): ChartSpec => ({
      caption,
      small,
      data: (hist) => ({ series: [{ values: hist.map(f), color, fill: true }], max: 100, maxLabel: "100%" }),
    });
    const info = this.info;
    const hw = this.hw;

    switch (item.kind) {
      case "cpu":
        return {
          sub: () => info?.cpu_name ?? "",
          charts: [pct("% use over the last readings", (s) => s.cpu)],
          stats: (s) => [
            ["Use", percent(s.cpu)],
            ["Speed", speed(s.cpu_mhz)],
            ["Processes", count(s.procs)],
            ["Threads", count(s.threads)],
            ["Handles", count(s.handles)],
            ["Up time", info ? duration(Date.now() / 1000 - info.boot_time) : "–"],
            ["Base speed", speed(hw?.cpu.base_mhz ?? 0)],
            ["Sockets", hw ? String(hw.cpu.sockets) : "–"],
            ["Cores", info ? String(info.physical_cores || "?") : "–"],
            ["Logical processors", info ? String(info.logical_cores) : "–"],
            ["Virtualization", hw ? (hw.cpu.virtualization ? "Enabled" : "Disabled") : "–"],
            ["L1 cache", hw ? bytes(hw.cpu.l1) : "–"],
            ["L2 cache", hw ? bytes(hw.cpu.l2) : "–"],
            ["L3 cache", hw ? bytes(hw.cpu.l3) : "–"],
          ],
          extra: (s) =>
            h(
              "div",
              { class: "cores", title: "Use of each logical processor" },
              ...s.cores.map((c) => h("div", { class: "core", style: `--p:${Math.round(c)}%` }, String(Math.round(c)))),
            ),
        };
      case "mem":
        return {
          sub: () => {
            const total = this.hist.length ? this.hist[this.hist.length - 1].mem_total : 0;
            return total ? `${bytes(hw?.memory.installed || total)} installed` : "";
          },
          charts: [pct("% in use", (s) => (s.mem_total ? (s.mem_used / s.mem_total) * 100 : 0))],
          stats: (s) => [
            ["In use", bytes(s.mem_used)],
            ["Available", bytes(Math.max(0, s.mem_total - s.mem_used))],
            ["Share in use", percent(s.mem_total ? (s.mem_used / s.mem_total) * 100 : 0)],
            ["Committed", `${bytes(s.committed)} / ${bytes(s.commit_limit)}`],
            ["Cached", bytes(s.cached)],
            ["Paged pool", bytes(s.paged_pool)],
            ["Non-paged pool", bytes(s.nonpaged_pool)],
            ["Page file in use", `${bytes(s.swap_used)} / ${bytes(s.swap_total)}`],
            ["Speed", hw?.memory.speed_mhz ? `${hw.memory.speed_mhz} MHz` : "–"],
            ["Slots used", hw?.memory.slots_total ? `${hw.memory.slots_used} of ${hw.memory.slots_total}` : "–"],
            ["Form factor", hw?.memory.form_factor || "–"],
            ["Hardware reserved", hw?.memory.installed ? bytes(Math.max(0, hw.memory.installed - s.mem_total)) : "–"],
          ],
        };
      case "disk": {
        const write = seriesColor("net");
        const part = hw?.disks.find((d) => d.index === Number.parseInt(item.name, 10));
        return {
          sub: () => (part ? [part.model, part.kind].filter(Boolean).join("  ·  ") : ""),
          charts: [
            pct("% of time busy", (s) => disk(s, item.name)?.active ?? 0),
            {
              caption: "Transfer speed",
              small: true,
              legend: [
                ["Read", color],
                ["Write", write],
              ],
              data: (hist) => {
                const r = hist.map((s) => disk(s, item.name)?.read ?? 0);
                const w = hist.map((s) => disk(s, item.name)?.write ?? 0);
                const max = niceCeil(Math.max(...r, ...w));
                return { series: [{ values: r, color, fill: true }, { values: w, color: write }], max, maxLabel: rateFull(max) };
              },
            },
          ],
          stats: (s) => {
            const d = disk(s, item.name);
            return [
              ["Busy", percent(d?.active ?? 0)],
              ["Read speed", rateFull(d?.read ?? 0)],
              ["Write speed", rateFull(d?.write ?? 0)],
              ["Average response time", `${(d?.response_ms ?? 0).toFixed(1)} ms`],
              ["Capacity", part?.size ? bytes(part.size) : "–"],
              ["Type", part?.kind || "–"],
            ];
          },
          extra: () => {
            // "0 C: D:" → the drive letters that live on this disk.
            const letters = item.name.split(" ").filter((p) => /^[A-Za-z]:$/.test(p)).map((p) => p.toUpperCase());
            const vols = (info?.volumes ?? []).filter((v) => letters.includes(v.mount.slice(0, 2).toUpperCase()));
            if (!vols.length) return null;
            return h(
              "div",
              { class: "vols" },
              ...vols.map((v) => {
                const used = v.total - v.free;
                return h(
                  "div",
                  { class: "vol" },
                  h(
                    "div",
                    { class: "lbl" },
                    h("span", null, `${v.mount.slice(0, 2)} ${v.label}`.trim()),
                    h("span", null, `${bytes(v.free)} free of ${bytes(v.total)}`),
                  ),
                  h("div", { class: "bar" }, h("i", { style: `width:${v.total ? ((used / v.total) * 100).toFixed(1) : 0}%` })),
                );
              }),
            );
          },
        };
      }
      case "net": {
        const send = seriesColor("gpu");
        const adapter = () => this.hw?.nets.find((n) => n.name === item.name);
        return {
          sub: () => adapter()?.description || item.name,
          charts: [
            {
              caption: "Speed",
              legend: [
                ["Receive", color],
                ["Send", send],
              ],
              data: (hist) => {
                const rx = hist.map((s) => net(s, item.name)?.rx ?? 0);
                const tx = hist.map((s) => net(s, item.name)?.tx ?? 0);
                const max = niceCeil(Math.max(...rx, ...tx));
                return { series: [{ values: rx, color, fill: true }, { values: tx, color: send }], max, maxLabel: rateFull(max) };
              },
            },
          ],
          stats: (s) => {
            const n = net(s, item.name);
            return [
              ["Receive", rateFull(n?.rx ?? 0)],
              ["Send", rateFull(n?.tx ?? 0)],
              ["Connection type", adapter()?.kind || "–"],
              ["Link speed", linkSpeed(adapter()?.link_bps ?? 0)],
              ["IPv4 address", adapter()?.ipv4 || "–"],
              ["IPv6 address", adapter()?.ipv6 || "–"],
            ];
          },
        };
      }
      case "gpu":
        return {
          sub: () => item.name,
          charts: [
            pct("% use (busiest engine)", (s) => gpu(s, item.name)?.util ?? 0),
            {
              caption: "Dedicated memory in use",
              small: true,
              data: (hist) => {
                const total = gpu(hist[hist.length - 1], item.name)?.mem_total ?? 0;
                const used = hist.map((s) => gpu(s, item.name)?.mem_used ?? 0);
                const max = total || Math.max(1, ...used);
                return { series: [{ values: used, color, fill: true }], max, maxLabel: bytes(max) };
              },
            },
          ],
          stats: (s) => {
            const g = gpu(s, item.name);
            return [
              ["Use", percent(g?.util ?? 0)],
              ["Dedicated memory", `${bytes(g?.mem_used ?? 0)} / ${bytes(g?.mem_total ?? 0)}`],
              ["Shared memory in use", bytes(g?.shared_used ?? 0)],
              ["Temperature", g?.temp ? `${Math.round(g.temp)} °C` : "–"],
              ...(g?.engines ?? []).slice(0, 4).map(([kind, load]) => [kind.replace(/([a-z])([A-Z])/g, "$1 $2"), percent(load)] as [string, string]),
              ["Driver version", hw?.gpus.find((x) => x.name === item.name)?.driver_version || "–"],
              ["Driver date", driverDate(hw?.gpus.find((x) => x.name === item.name)?.driver_date ?? "") || "–"],
            ];
          },
        };
    }
  }
}
