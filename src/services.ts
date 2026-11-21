// The Services tab: Windows services with start / stop / restart.

import { api, asError, type ServiceInfo } from "./api";
import { closeMenu, copy, h, Notice, openMenu } from "./ui";
import { setText, VirtualList } from "./vlist";

const ROW_HEIGHT = 26;
const REFRESH_MS = 3000;

type SortKey = "name" | "display" | "status" | "start" | "group" | "pid";
type Action = "start" | "stop" | "restart";

const STATUS_TEXT: Record<string, string> = {
  running: "Running",
  stopped: "Stopped",
  starting: "Starting…",
  stopping: "Stopping…",
  paused: "Paused",
  other: "–",
};
const START_TEXT: Record<string, string> = {
  automatic: "Automatic",
  manual: "Manual",
  disabled: "Disabled",
  system: "System",
  "": "–",
};

interface Refs {
  name: HTMLElement;
  display: HTMLElement;
  status: HTMLElement;
  start: HTMLElement;
  group: HTMLElement;
  pid: HTMLElement;
}
const refs = new WeakMap<HTMLElement, Refs>();

export class ServicesTab {
  private all: ServiceInfo[] = [];
  private rows: ServiceInfo[] = [];
  private filter = "";
  private sortKey: SortKey = "name";
  private desc = false;
  private selected: string | null = null;
  private visible = false;
  private timer = 0;
  private loaded = false;
  /** Services with an action on its way, and what to show meanwhile. */
  private readonly busy = new Map<string, "starting" | "stopping">();

  private readonly search: HTMLInputElement;
  private readonly countEl: HTMLElement;
  private readonly head: HTMLElement;
  private readonly list: VirtualList<ServiceInfo>;
  private readonly emptyEl: HTMLElement;
  private readonly notice = new Notice();
  private readonly buttons: Record<Action, HTMLButtonElement>;

  constructor(
    root: HTMLElement,
    /** Shows the process a service runs in (the Processes tab). */
    private readonly goToProcess: (pid: number) => void,
  ) {
    this.search = h("input", {
      class: "search",
      type: "search",
      placeholder: "Search name or PID  (Ctrl+F)",
      spellcheck: false,
      "aria-label": "Search services",
      oninput: () => {
        this.filter = this.search.value.trim().toLowerCase();
        this.rebuild();
      },
    });
    this.countEl = h("span", { class: "count" });
    const btn = (action: Action, label: string) =>
      h("button", { class: "btn", disabled: true, onclick: () => void this.act(action) }, label);
    this.buttons = { start: btn("start", "Start"), stop: btn("stop", "Stop"), restart: btn("restart", "Restart") };

    const th = (key: SortKey, label: string, num = false) =>
      h("button", { class: `th${num ? " num" : ""}`, "data-key": key, onclick: () => this.sortBy(key) }, label);
    this.head = h(
      "div",
      { class: "thead" },
      th("name", "Name"),
      th("display", "Description"),
      th("status", "Status"),
      th("start", "Starts"),
      th("group", "Group"),
      th("pid", "PID", true),
    );
    const scroller = h("div", { class: "vscroll", tabindex: "0", role: "grid", "aria-label": "Services" });
    this.emptyEl = h("div", { class: "empty", hidden: true });
    scroller.append(this.emptyEl);
    this.list = new VirtualList<ServiceInfo>(scroller, ROW_HEIGHT, () => this.createRow(), (el, s) => this.bindRow(el, s));
    scroller.addEventListener("click", (e) => {
      const i = this.list.indexAt(e.clientY);
      this.select(i >= 0 ? this.rows[i].name : null);
    });
    scroller.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      const i = this.list.indexAt(e.clientY);
      if (i < 0) return;
      const s = this.rows[i];
      this.select(s.name);
      const state = this.shownStatus(s);
      openMenu(e.clientX, e.clientY, [
        ...(state === "stopped" ? [{ label: "Start", run: () => void this.act("start") }] : []),
        ...(state === "running"
          ? [
              { label: "Stop", run: () => void this.act("stop") },
              { label: "Restart", run: () => void this.act("restart") },
            ]
          : []),
        ...(s.pid ? [{ label: "Go to process", run: () => this.goToProcess(s.pid) }] : []),
        "sep" as const,
        { label: "Open Services", run: () => void api.openTool("services").catch((err) => this.notice.show(asError(err))) },
        { label: "Search online", run: () => void api.searchOnline(`${s.name} ${s.display} service`).catch((err) => this.notice.show(asError(err))) },
        "sep" as const,
        { label: "Copy name", run: () => copy(s.name) },
        { label: "Copy description", run: () => copy(s.description || s.display) },
      ]);
    });
    scroller.addEventListener("keydown", (e) => {
      const at = this.selected ? this.rows.findIndex((r) => r.name === this.selected) : -1;
      let to = at;
      if (e.key === "ArrowDown") to = at + 1;
      else if (e.key === "ArrowUp") to = at < 0 ? 0 : at - 1;
      else if (e.key === "PageDown") to = at + this.list.pageSize();
      else if (e.key === "PageUp") to = at - this.list.pageSize();
      else if (e.key === "Home") to = 0;
      else if (e.key === "End") to = this.rows.length - 1;
      else return;
      e.preventDefault();
      if (!this.rows.length) return;
      to = Math.max(0, Math.min(this.rows.length - 1, to));
      this.select(this.rows[to].name);
      this.list.scrollToIndex(to);
    });

    root.append(
      h(
        "div",
        { class: "toolbar" },
        this.search,
        this.countEl,
        h("span", { class: "spacer" }),
        h(
          "button",
          { class: "btn", title: "Open the Windows Services console", onclick: () => void api.openTool("services").catch((err) => this.notice.show(asError(err))) },
          "Open Services",
        ),
        this.buttons.start,
        this.buttons.stop,
        this.buttons.restart,
      ),
      h("div", { class: "table services" }, this.head, scroller),
      this.notice.el,
    );
    this.paintHeader();
  }

  onShow(): void {
    this.visible = true;
    void this.load(!this.loaded);
    window.clearInterval(this.timer);
    this.timer = window.setInterval(() => {
      if (this.visible && !document.hidden) void this.load(false);
    }, REFRESH_MS);
  }

  onHide(): void {
    this.visible = false;
    window.clearInterval(this.timer);
  }

  focusSearch(): void {
    this.search.focus();
    this.search.select();
  }

  /** Shows only the services that run inside one process. */
  showForPid(pid: number): void {
    this.search.value = String(pid);
    this.filter = String(pid);
    this.rebuild();
  }

  escape(): boolean {
    if (this.search.value) {
      this.search.value = "";
      this.filter = "";
      this.rebuild();
      return true;
    }
    if (this.selected) {
      this.select(null);
      return true;
    }
    return false;
  }

  private async load(fresh: boolean): Promise<void> {
    try {
      this.all = await api.services(fresh);
      this.loaded = true;
      this.rebuild();
    } catch (e) {
      this.notice.show(asError(e));
    }
  }

  private shownStatus(s: ServiceInfo): string {
    return this.busy.get(s.name) ?? s.status;
  }

  private rebuild(): void {
    const f = this.filter;
    const rows = this.all.filter(
      (s) => !f || s.name.toLowerCase().includes(f) || s.display.toLowerCase().includes(f) || String(s.pid) === f,
    );
    const key = this.sortKey;
    const sign = this.desc ? -1 : 1;
    rows.sort((a, b) => {
      let d: number;
      if (key === "pid") d = a.pid - b.pid;
      else d = String(a[key]).localeCompare(String(b[key]), undefined, { sensitivity: "base" });
      return d !== 0 ? d * sign : a.name.localeCompare(b.name);
    });
    this.rows = rows;
    if (this.selected && !rows.some((r) => r.name === this.selected)) this.selected = null;
    this.list.setItems(rows);
    const running = this.all.filter((s) => s.status === "running").length;
    setText(this.countEl, f ? `${rows.length} shown of ${this.all.length}` : `${running} running of ${this.all.length}`);
    const empty = this.loaded && rows.length === 0;
    this.emptyEl.hidden = !empty;
    if (empty) {
      setText(this.emptyEl, /^\d+$/.test(f) ? `No service runs in process ${f}.` : `Nothing matches "${this.search.value.trim()}".`);
    }
    this.paintButtons();
  }

  private createRow(): HTMLElement {
    const name = h("div", { class: "cell" });
    const display = h("div", { class: "cell muted" });
    const status = h("div", { class: "cell" });
    const start = h("div", { class: "cell muted" });
    const group = h("div", { class: "cell muted" });
    const pid = h("div", { class: "cell num muted" });
    const el = h("div", { role: "row" }, name, display, status, start, group, pid);
    refs.set(el, { name, display, status, start, group, pid });
    return el;
  }

  private bindRow(el: HTMLElement, s: ServiceInfo): void {
    const r = refs.get(el)!;
    el.classList.toggle("sel", s.name === this.selected);
    const state = this.shownStatus(s);
    setText(r.name, s.name);
    setText(r.display, s.display);
    setText(r.status, STATUS_TEXT[state] ?? "–");
    r.status.classList.toggle("status-running", state === "running");
    setText(r.start, START_TEXT[s.start] ?? "–");
    setText(r.group, s.group);
    setText(r.pid, s.pid ? String(s.pid) : "");
    const tip = s.description || s.display;
    if (el.title !== tip) el.title = tip;
  }

  private sortBy(key: SortKey): void {
    if (this.sortKey === key) this.desc = !this.desc;
    else {
      this.sortKey = key;
      this.desc = false;
    }
    this.paintHeader();
    this.rebuild();
  }

  private paintHeader(): void {
    for (const b of this.head.querySelectorAll<HTMLElement>(".th")) {
      if (b.dataset.key === this.sortKey) b.dataset.sorted = this.desc ? "desc" : "asc";
      else delete b.dataset.sorted;
    }
  }

  private select(name: string | null): void {
    this.selected = name;
    this.list.render();
    this.paintButtons();
  }

  private paintButtons(): void {
    const s = this.rows.find((r) => r.name === this.selected);
    const state = s ? this.shownStatus(s) : "";
    this.buttons.start.disabled = state !== "stopped";
    this.buttons.stop.disabled = state !== "running";
    this.buttons.restart.disabled = state !== "running";
  }

  private async act(action: Action): Promise<void> {
    const name = this.selected;
    if (!name || this.busy.has(name)) return;
    closeMenu();
    this.notice.clear();
    this.busy.set(name, action === "start" ? "starting" : "stopping");
    this.list.render();
    this.paintButtons();
    try {
      await api.serviceAction(name, action);
    } catch (e) {
      this.notice.show(asError(e));
    } finally {
      this.busy.delete(name);
      // Show what Windows says now, whether the action worked or was refused.
      await this.load(false);
    }
  }
}
