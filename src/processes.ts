// The Processes tab: a live table of running programs and everything you can do to one.
//
// Three ways to look at the same list: "By type" (Apps / Background / Windows, as Windows
// Task Manager's Processes tab), "By name" (one row per program) and "List" (every process
// with any columns you pick, as its Details tab).

import { api, asError, type CmdError, type Priority, type ProcDetails } from "./api";
import { cleanColumns, clock, COLUMNS, gridTemplate, type Column } from "./columns";
import { bytes, count as countText } from "./format";
import {
  buildRows,
  cpusToMask,
  F_APP,
  F_ECO,
  F_HUNG,
  F_SUSPENDED,
  fromRaw,
  IconBook,
  Picks,
  keepOrder,
  maskToCpus,
  membersOf,
  procKey,
  type ColumnKey,
  type Proc,
  type RawProc,
  type Row,
  type Section,
  type StaticInfo,
  type ViewMode,
} from "./model";
import { closeMenu, confirmDialog, copy, formDialog, h, isMenuOpen, Notice, openMenu, type MenuItem } from "./ui";
import { setText, VirtualList } from "./vlist";

const ROW_HEIGHT = 26;
/** How long after the pointer last moved in the list the row order stays put. */
const AIM_MS = 1500;
/** How long a process that was jumped to is kept on screen while the live sort moves it. */
const FOLLOW_MS = 5000;
const STORE_KEY = "ftm.processes";
/** How many processes' once-only facts are asked for in one request. */
const INFO_BATCH = 150;

const PRIORITIES: { key: Priority; label: string; base: number }[] = [
  { key: "realtime", label: "Realtime", base: 24 },
  { key: "high", label: "High", base: 13 },
  { key: "above-normal", label: "Above normal", base: 10 },
  { key: "normal", label: "Normal", base: 8 },
  { key: "below-normal", label: "Below normal", base: 6 },
  { key: "low", label: "Low", base: 4 },
];

const VIEWS: { key: ViewMode; label: string; hint: string }[] = [
  { key: "type", label: "By type", hint: "Apps, background processes and Windows processes" },
  { key: "name", label: "By name", hint: "One row per program" },
  { key: "list", label: "List", hint: "Every process on its own row" },
];

interface RowRefs {
  twist: HTMLElement;
  icon: HTMLImageElement;
  noicon: HTMLElement;
  name: HTMLElement;
  count: HTMLElement;
  cells: HTMLElement[];
  heat: string[];
  version: number;
  key: string;
  iconUrl: string;
}

const refs = new WeakMap<HTMLElement, RowRefs>();

interface Saved {
  view: ViewMode;
  columns: ColumnKey[];
  sortKey: ColumnKey;
  desc: boolean;
  collapsed: Section[];
}

function loadSaved(): Saved {
  const fallback: Saved = { view: "type", columns: cleanColumns(null), sortKey: "cpu", desc: true, collapsed: [] };
  try {
    const raw = JSON.parse(localStorage.getItem(STORE_KEY) ?? "null") as Partial<Saved> | null;
    if (!raw || typeof raw !== "object") return fallback;
    const columns = cleanColumns(raw.columns);
    const sortable = new Set<string>(["name", ...columns]);
    return {
      view: VIEWS.some((v) => v.key === raw.view) ? (raw.view as ViewMode) : fallback.view,
      columns,
      sortKey: typeof raw.sortKey === "string" && sortable.has(raw.sortKey) ? (raw.sortKey as ColumnKey) : fallback.sortKey,
      desc: typeof raw.desc === "boolean" ? raw.desc : fallback.desc,
      collapsed: Array.isArray(raw.collapsed)
        ? raw.collapsed.filter((s): s is Section => s === "apps" || s === "background" || s === "windows")
        : [],
    };
  } catch {
    return fallback;
  }
}

export class ProcessesTab {
  private procs: Proc[] = [];
  private rows: Row[] = [];
  private view: ViewMode;
  private columns: ColumnKey[];
  private shownColumns: Column[] = [];
  private columnsVersion = 0;
  private sortKey: ColumnKey;
  private desc: boolean;
  private filter = "";
  private readonly expanded = new Set<string>();
  private readonly collapsed: Set<Section>;
  private selected: string | null = null;
  private lastAim = 0;
  private followUntil = 0;
  private memTotal = 1;
  private ownPid = 0;
  /** Program icons by lower-case program name; null = it has none. */
  private readonly icons = new IconBook();
  /** Once-only facts by process identity (PID + start time). */
  private info = new Map<string, StaticInfo>();
  private infoBusy = false;
  private detailsPid = 0;
  private ticks = 0;
  private readonly picks = new Picks();
  /** Asked to show this process as soon as it is in the list. */
  private wantPid = 0;

  private readonly search: HTMLInputElement;
  private readonly countEl: HTMLElement;
  private readonly viewSeg: HTMLElement;
  private readonly endBtn: HTMLButtonElement;
  private readonly treeBtn: HTMLButtonElement;
  private readonly head: HTMLElement;
  private readonly inner: HTMLElement;
  private readonly list: VirtualList<Row>;
  private readonly drawer: HTMLElement;
  private readonly notice = new Notice();
  private readonly emptyEl: HTMLElement;

  constructor(
    root: HTMLElement,
    /** Shows the services that run inside a process (the Services tab). */
    private readonly goToServices: (pid: number) => void,
  ) {
    const saved = loadSaved();
    this.view = saved.view;
    this.columns = saved.columns;
    this.sortKey = saved.sortKey;
    this.desc = saved.desc;
    this.collapsed = new Set(saved.collapsed);

    this.search = h("input", {
      class: "search",
      type: "search",
      placeholder: "Search name, window, user or PID  (Ctrl+F)",
      spellcheck: false,
      "aria-label": "Search processes",
      oninput: () => {
        this.filter = this.search.value;
        this.lastAim = 0;
        this.rebuild();
      },
    });
    this.viewSeg = h(
      "div",
      { class: "seg", role: "group", "aria-label": "How to list processes" },
      ...VIEWS.map((v) => h("button", { "data-view": v.key, title: v.hint, onclick: () => this.setView(v.key) }, v.label)),
    );
    this.countEl = h("span", { class: "count" });
    this.endBtn = h("button", { class: "btn danger", disabled: true, onclick: () => void this.endSelected(false) }, "End task");
    this.treeBtn = h(
      "button",
      { class: "btn tree", disabled: true, title: "End it and everything it started", onclick: () => void this.endSelected(true) },
      "End tree",
    );
    const runBtn = h("button", { class: "btn", title: "Start a program  (Ctrl+N)", onclick: () => void this.runNewTask() }, "Run new task");
    const colBtn = h("button", { class: "btn", title: "Choose which columns to show", onclick: () => void this.chooseColumns() }, "Columns");

    this.head = h("div", { class: "thead", role: "row" });
    this.head.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      openMenu(e.clientX, e.clientY, [{ label: "Choose columns…", run: () => void this.chooseColumns() }]);
    });

    const scroller = h("div", { class: "vscroll", tabindex: "0", role: "grid", "aria-label": "Processes" });
    this.emptyEl = h("div", { class: "empty", hidden: true });
    scroller.append(this.emptyEl);
    this.list = new VirtualList<Row>(scroller, ROW_HEIGHT, () => this.createRow(), (el, row) => this.bindRow(el, row));

    const aim = () => {
      this.lastAim = performance.now();
      // The user took over: stop moving the list for them.
      this.followUntil = 0;
    };
    scroller.addEventListener("pointermove", aim, { passive: true });
    scroller.addEventListener("pointerdown", aim, { passive: true });
    scroller.addEventListener("wheel", aim, { passive: true });
    scroller.addEventListener("pointerleave", () => {
      this.lastAim = 0;
    });
    scroller.addEventListener("click", (e) => this.onClick(e));
    scroller.addEventListener("dblclick", (e) => this.onDoubleClick(e));
    scroller.addEventListener("contextmenu", (e) => this.onContextMenu(e));
    scroller.addEventListener("keydown", (e) => this.onListKey(e));

    this.drawer = h("aside", { class: "drawer", hidden: true, "aria-label": "Process details" });
    this.inner = h("div", { class: "hinner" }, this.head, scroller);

    root.append(
      h(
        "div",
        { class: "toolbar" },
        this.search,
        this.viewSeg,
        this.countEl,
        h("span", { class: "spacer" }),
        colBtn,
        runBtn,
        this.treeBtn,
        this.endBtn,
      ),
      h("div", { class: "split" }, h("div", { class: "table procs" }, h("div", { class: "hscroll" }, this.inner)), this.drawer),
      this.notice.el,
    );
    this.applyColumns();
    this.paintView();
  }

  // ── data in ──

  setOwnPid(pid: number): void {
    this.ownPid = pid;
  }

  update(raw: readonly RawProc[], memTotal: number): void {
    this.procs = raw.map(fromRaw);
    // A fresh reading may predate a change the user just picked: keep showing the pick.
    this.picks.applyTo(this.procs);
    this.memTotal = memTotal || 1;
    this.ticks++;
    this.rebuild();
    // A process the user just jumped to ("Go to process") must not drift off screen when the
    // next reading re-sorts the list.
    if (this.selected && performance.now() < this.followUntil) {
      const at = this.rows.findIndex((r) => r.key === this.selected);
      if (at >= 0) this.list.scrollToIndex(at);
    }
    void this.fetchInfo();
    if (this.wantPid) this.showPid(this.wantPid);
    // Keep the open details fresh, but never while text in them is being selected to copy.
    const selecting = !!window.getSelection()?.toString() && this.drawer.contains(window.getSelection()!.anchorNode);
    if (this.detailsPid && this.ticks % 3 === 0 && !selecting) void this.showDetails(this.detailsPid);
  }

  private readonly infoOf = (p: Proc): StaticInfo | undefined => this.info.get(procKey(p));

  private rebuild(): void {
    let rows = buildRows(this.procs, {
      view: this.view,
      sortKey: this.sortKey,
      desc: this.desc,
      filter: this.filter,
      expanded: this.expanded,
      collapsed: this.collapsed,
      info: this.infoOf,
    });
    const aiming = isMenuOpen() || performance.now() - this.lastAim < AIM_MS;
    if (aiming && this.rows.length) {
      rows = keepOrder(
        this.rows.map((r) => r.key),
        rows,
      );
    }
    this.rows = rows;
    if (this.selected && !rows.some((r) => r.key === this.selected)) {
      this.selected = null;
      if (!this.drawer.hidden) this.closeDrawer();
    }
    this.list.setItems(rows);

    const total = this.procs.length;
    const shown = rows.filter((r) => r.kind !== "section").reduce((n, r) => n + (r.child ? 0 : r.count), 0);
    setText(this.countEl, this.filter.trim() ? `${shown} shown of ${total}` : `${total} processes`);
    const empty = rows.length === 0 && total > 0;
    this.emptyEl.hidden = !empty;
    if (empty) setText(this.emptyEl, `Nothing matches "${this.filter.trim()}".`);
    this.paintButtons();
  }

  /** Fetches the once-only facts (friendly name, user, program file…) for processes not seen before. */
  private async fetchInfo(): Promise<void> {
    if (this.infoBusy) return;
    const missing = this.procs.filter((p) => !this.info.has(procKey(p))).slice(0, INFO_BATCH);
    if (!missing.length) {
      // Forget processes that ended, so the memory does not grow for ever.
      if (this.info.size > this.procs.length + 200) {
        const live = new Set(this.procs.map(procKey));
        this.info = new Map([...this.info].filter(([k]) => live.has(k)));
      }
      return;
    }
    this.infoBusy = true;
    const keys = new Map(missing.map((p) => [p.pid, procKey(p)]));
    try {
      for (const item of await api.staticInfo(missing.map((p) => p.pid))) {
        const key = keys.get(item.pid);
        if (key) this.info.set(key, item);
      }
    } catch {
      // Without them the table still works, with file names instead of friendly names.
      for (const key of keys.values()) {
        this.info.set(key, { path: "", user: "", description: "", company: "", architecture: "", elevated: null, command: "" });
      }
    }
    this.infoBusy = false;
    this.rebuild();
    void this.fetchInfo();
  }

  // ── view, columns, sorting ──

  private save(): void {
    const saved: Saved = {
      view: this.view,
      columns: this.columns,
      sortKey: this.sortKey,
      desc: this.desc,
      collapsed: [...this.collapsed],
    };
    try {
      localStorage.setItem(STORE_KEY, JSON.stringify(saved));
    } catch {
      /* a full or blocked store only means the choice is not remembered */
    }
  }

  private setView(view: ViewMode): void {
    if (view === this.view) return;
    this.view = view;
    this.lastAim = 0;
    this.paintView();
    this.rebuild();
    this.save();
  }

  private paintView(): void {
    for (const b of this.viewSeg.querySelectorAll<HTMLElement>("button")) {
      b.setAttribute("aria-pressed", String(b.dataset.view === this.view));
    }
  }

  private applyColumns(): void {
    this.shownColumns = COLUMNS.filter((c) => this.columns.includes(c.key));
    if (this.sortKey !== "name" && !this.columns.includes(this.sortKey)) {
      this.sortKey = this.columns.includes("cpu") ? "cpu" : "name";
      this.desc = this.sortKey !== "name";
    }
    this.columnsVersion++;
    const { template, minWidth } = gridTemplate(this.columns);
    this.inner.style.setProperty("--cols", template);
    this.inner.style.minWidth = `${minWidth}px`;
    const th = (key: ColumnKey, label: string, num: boolean) =>
      h("button", { class: `th${num ? " num" : ""}`, "data-key": key, onclick: () => this.sortBy(key) }, label);
    this.head.replaceChildren(th("name", "Name", false), ...this.shownColumns.map((c) => th(c.key, c.label, c.num)));
    this.paintHeader();
    this.list?.render();
  }

  private async chooseColumns(): Promise<void> {
    const boxes = COLUMNS.map((c) => {
      const box = h("input", { type: "checkbox", checked: this.columns.includes(c.key), value: c.key });
      return { box, row: h("label", { class: "check" }, box, c.label) };
    });
    const done = await formDialog("Columns", [h("div", { class: "checks down", style: `--rows:${Math.ceil(boxes.length / 2)}` }, ...boxes.map((b) => b.row))], "Apply", async () => {
      this.columns = cleanColumns(boxes.filter((b) => b.box.checked).map((b) => b.box.value));
      return null;
    });
    if (!done) return;
    this.applyColumns();
    this.rebuild();
    this.save();
  }

  private sortBy(key: ColumnKey): void {
    if (this.sortKey === key) this.desc = !this.desc;
    else {
      this.sortKey = key;
      this.desc = COLUMNS.find((c) => c.key === key)?.num === true && key !== "pid" && key !== "session";
    }
    this.lastAim = 0;
    this.paintHeader();
    this.rebuild();
    this.save();
  }

  private paintHeader(): void {
    for (const b of this.head.querySelectorAll<HTMLElement>(".th")) {
      if (b.dataset.key === this.sortKey) b.dataset.sorted = this.desc ? "desc" : "asc";
      else delete b.dataset.sorted;
    }
  }

  // ── rows ──

  private createRow(): HTMLElement {
    const twist = h("span", { class: "twist" });
    const icon = h("img", { alt: "", hidden: true, draggable: false });
    const noicon = h("span", { class: "noicon" });
    const name = h("span", { class: "txt" });
    const count = h("span", { class: "groupcount" });
    const el = h("div", { role: "row" }, h("div", { class: "name-cell" }, twist, icon, noicon, name, count));
    refs.set(el, { twist, icon, noicon, name, count, cells: [], heat: [], version: -1, key: "", iconUrl: "" });
    return el;
  }

  private heatOf(kind: Column["heat"], row: Row): string {
    let level = 0;
    if (kind === "cpu" && row.cpu > 0.05) level = Math.min(0.08 + row.cpu / 70, 0.6);
    else if (kind === "gpu" && row.gpu > 0.5) level = Math.min(0.08 + row.gpu / 70, 0.6);
    else if (kind === "mem") {
      const share = row.mem / this.memTotal;
      if (share > 0.005) level = Math.min(0.05 + share * 4, 0.6);
    }
    return level ? level.toFixed(2) : "0";
  }

  private bindRow(el: HTMLElement, row: Row): void {
    const r = refs.get(el)!;
    if (r.version !== this.columnsVersion) {
      // The chosen columns changed: give the row the new set of cells.
      for (const c of r.cells) c.remove();
      r.cells = this.shownColumns.map((c) => h("div", { class: `cell${c.num ? " num" : ""}${c.heat ? ` heat heat-${c.heat}` : ""}` }));
      r.heat = r.cells.map(() => "");
      el.append(...r.cells);
      r.version = this.columnsVersion;
    }
    if (r.key !== row.key) {
      r.key = row.key;
      el.dataset.key = row.key;
    }
    const section = row.kind === "section";
    el.classList.toggle("sel", row.key === this.selected);
    el.classList.toggle("child", row.child);
    el.classList.toggle("section", section);
    el.classList.toggle("hung", (row.flags & F_HUNG) !== 0);
    el.classList.toggle("frozen", (row.flags & F_SUSPENDED) !== 0);
    setText(r.twist, row.kind === "proc" ? "" : row.open ? "▾" : "▸");
    setText(r.name, row.name);
    setText(r.count, row.kind === "proc" ? "" : `(${row.count})`);

    const info = row.rep ? this.infoOf(row.rep) : undefined;
    for (let i = 0; i < this.shownColumns.length; i++) {
      const column = this.shownColumns[i];
      const cell = r.cells[i];
      setText(cell, section ? "" : column.text(row, info));
      if (column.heat) {
        const heat = section ? "0" : this.heatOf(column.heat, row);
        if (r.heat[i] !== heat) {
          r.heat[i] = heat;
          cell.style.setProperty("--h", heat);
        }
      }
    }

    const url = section ? "" : (this.iconFor(row) ?? "");
    if (r.iconUrl !== url || r.icon.hidden === !!url) {
      r.iconUrl = url;
      if (url) r.icon.src = url;
      r.icon.hidden = !url;
    }
    r.noicon.hidden = section || !!url;
  }

  /** Icons are fetched once per program name, only for rows that actually reach the screen. */
  private iconFor(row: Row): string | null {
    const key = (row.rep?.name ?? row.name).toLowerCase();
    const have = this.icons.known(key);
    if (have !== undefined) return have;
    if (this.icons.shouldAsk(key, performance.now())) {
      api
        .icon(row.pid)
        .catch(() => null)
        .then((url) => {
          if (this.icons.answer(key, url, performance.now())) this.list.render();
        });
    }
    return null;
  }

  // ── selection ──

  private paintButtons(): void {
    const row = this.selectedRow();
    const usable = !!row && row.kind !== "section";
    this.endBtn.disabled = !usable;
    this.treeBtn.disabled = !row || row.kind !== "proc";
    setText(this.endBtn, row?.kind === "group" ? `End all ${row.count}` : "End task");
  }

  private selectedRow(): Row | undefined {
    return this.selected ? this.rows.find((r) => r.key === this.selected) : undefined;
  }

  private select(key: string | null, scroll = false): void {
    this.selected = key;
    if (scroll && key) {
      const i = this.rows.findIndex((r) => r.key === key);
      if (i >= 0) this.list.scrollToIndex(i);
    }
    this.list.render();
    this.paintButtons();
    const row = this.selectedRow();
    if (!this.drawer.hidden) {
      if (row?.kind === "proc") void this.showDetails(row.pid);
      else this.closeDrawer();
    }
  }

  /** Opens or closes a group or a section heading. A deliberate re-layout, so it happens now even while aiming. */
  private toggle(row: Row): void {
    if (row.kind === "section" && row.section) {
      if (this.collapsed.has(row.section)) this.collapsed.delete(row.section);
      else this.collapsed.add(row.section);
      this.save();
    } else if (row.kind === "group") {
      const k = row.key.slice(2);
      if (this.expanded.has(k)) this.expanded.delete(k);
      else this.expanded.add(k);
    } else return;
    const hold = this.lastAim;
    this.lastAim = 0;
    this.rebuild();
    this.lastAim = hold;
  }

  private rowAt(e: MouseEvent): Row | undefined {
    const i = this.list.indexAt(e.clientY);
    return i >= 0 ? this.rows[i] : undefined;
  }

  private onClick(e: MouseEvent): void {
    const row = this.rowAt(e);
    if (!row) {
      this.select(null);
      return;
    }
    this.select(row.key);
    if (row.kind === "section" || (row.kind === "group" && (e.target as HTMLElement).classList.contains("twist"))) this.toggle(row);
  }

  private onDoubleClick(e: MouseEvent): void {
    const row = this.rowAt(e);
    if (!row) return;
    if (row.kind === "group") this.toggle(row);
    else if (row.kind === "proc") void this.showDetails(row.pid);
  }

  // ── menus ──

  private onContextMenu(e: MouseEvent): void {
    e.preventDefault();
    const row = this.rowAt(e);
    if (!row) return;
    this.select(row.key);
    if (row.kind === "section") {
      openMenu(e.clientX, e.clientY, [{ label: row.open ? "Collapse" : "Expand", run: () => this.toggle(row) }]);
      return;
    }
    openMenu(e.clientX, e.clientY, row.kind === "group" ? this.groupMenu(row) : this.procMenu(row, e.clientX, e.clientY));
  }

  private fail = (e: unknown): void => this.notice.show(asError(e));

  private shellItems(row: Row): (MenuItem | "sep")[] {
    const p = row.rep!;
    const info = this.infoOf(p);
    return [
      { label: "Open file location", run: () => void api.openLocation(p.pid).catch(this.fail) },
      { label: "Properties", run: () => void api.properties(p.pid).catch(this.fail) },
      { label: "Search online", run: () => void api.searchOnline(info?.description ? `${p.name} ${info.description}` : p.name).catch(this.fail) },
    ];
  }

  private groupMenu(row: Row): (MenuItem | "sep")[] {
    return [
      { label: row.open ? "Collapse" : "Expand", run: () => this.toggle(row) },
      "sep",
      { label: `End all ${row.count} processes`, danger: true, hint: "Del", run: () => void this.endSelected(false) },
      "sep",
      ...this.shellItems(row),
      "sep",
      { label: "Copy name", run: () => copy(row.name) },
    ];
  }

  private procMenu(row: Row, x: number, y: number): (MenuItem | "sep")[] {
    const p = row.rep!;
    const info = this.infoOf(p);
    const frozen = (p.flags & F_SUSPENDED) !== 0;
    const eco = (p.flags & F_ECO) !== 0;
    const items: (MenuItem | "sep")[] = [
      { label: "End task", danger: true, hint: "Del", run: () => void this.endSelected(false) },
      { label: "End process tree", danger: true, run: () => void this.endSelected(true) },
    ];
    if (p.name.toLowerCase() === "explorer.exe") {
      items.push({ label: "Restart Windows Explorer", run: () => void this.restartExplorer() });
    }
    items.push("sep");
    if (p.flags & F_APP) items.push({ label: "Switch to", run: () => void api.switchTo(p.pid).catch(this.fail) });
    items.push(
      { label: "Efficiency mode", checked: eco, run: () => void this.setEfficiency(p, !eco) },
      { label: frozen ? "Resume" : "Suspend", run: () => void this.setPaused(p, !frozen) },
      {
        label: "Set priority…",
        run: () =>
          openMenu(
            x,
            y,
            PRIORITIES.map((pr) => ({ label: pr.label, checked: pr.base === p.priority, run: () => void this.setPriority(p, pr.key, pr.base) })),
          ),
      },
      { label: "Set affinity…", run: () => void this.chooseAffinity(p) },
      "sep",
      { label: "Details", hint: "Enter", run: () => void this.showDetails(p.pid) },
      { label: "Go to services", run: () => this.goToServices(p.pid) },
      { label: "Create dump file", run: () => void this.createDump(p) },
      "sep",
      ...this.shellItems(row),
      "sep",
      { label: "Copy name", run: () => copy(p.name) },
      { label: "Copy PID", run: () => copy(String(p.pid)) },
    );
    if (info?.path) items.push({ label: "Copy path", run: () => copy(info.path) });
    if (info?.command) items.push({ label: "Copy command line", run: () => copy(info.command) });
    return items;
  }

  private onListKey(e: KeyboardEvent): void {
    const at = this.selected ? this.rows.findIndex((r) => r.key === this.selected) : -1;
    const row = at >= 0 ? this.rows[at] : undefined;
    const foldable = row && row.kind !== "proc";
    const move = (to: number) => {
      if (!this.rows.length) return;
      const i = Math.max(0, Math.min(this.rows.length - 1, to));
      this.select(this.rows[i].key, true);
    };
    switch (e.key) {
      case "ArrowDown":
        move(at + 1);
        break;
      case "ArrowUp":
        move(at < 0 ? 0 : at - 1);
        break;
      case "PageDown":
        move(at + this.list.pageSize());
        break;
      case "PageUp":
        move(at - this.list.pageSize());
        break;
      case "Home":
        move(0);
        break;
      case "End":
        move(this.rows.length - 1);
        break;
      case "ArrowRight":
        if (foldable && !row.open) this.toggle(row);
        break;
      case "ArrowLeft":
        if (foldable && row.open) this.toggle(row);
        break;
      case "Enter":
        if (foldable) this.toggle(row);
        else if (row) void this.showDetails(row.pid);
        break;
      case "Delete":
        void this.endSelected(false);
        break;
      default:
        return;
    }
    e.preventDefault();
  }

  // ── actions ──

  /**
   * Shows a change at once and keeps showing it while Windows works on it; puts it back if
   * Windows refuses. `p` may be an older copy of the row (the list is re-read all the time), so
   * the change is always made on the row as it is now.
   */
  private async change(
    p: Proc,
    field: string,
    apply: (p: Proc) => void,
    undo: (p: Proc) => void,
    request: () => Promise<unknown>,
  ): Promise<void> {
    this.notice.clear();
    const pick = this.picks.start(this.procs, p, field, apply);
    this.rebuild();
    try {
      await request();
      this.picks.confirm(this.procs, pick);
      this.rebuild();
    } catch (e) {
      this.picks.refuse(this.procs, pick, undo);
      this.rebuild();
      this.notice.show(asError(e));
    }
  }

  private setEfficiency(p: Proc, on: boolean): Promise<void> {
    const was = p.flags & F_ECO;
    return this.change(
      p,
      "efficiency",
      (x) => void (x.flags = on ? x.flags | F_ECO : x.flags & ~F_ECO),
      (x) => void (x.flags = (x.flags & ~F_ECO) | was),
      () => api.setEfficiency(p.pid, p.name, on),
    );
  }

  private setPaused(p: Proc, pause: boolean): Promise<void> {
    const was = p.flags & F_SUSPENDED;
    return this.change(
      p,
      "paused",
      (x) => void (x.flags = pause ? x.flags | F_SUSPENDED : x.flags & ~F_SUSPENDED),
      (x) => void (x.flags = (x.flags & ~F_SUSPENDED) | was),
      () => api.setPaused(p.pid, p.name, pause),
    );
  }

  private setPriority(p: Proc, priority: Priority, base: number): Promise<void> {
    const before = p.priority;
    let shown = base;
    return this.change(
      p,
      "priority",
      (x) => void (x.priority = shown),
      (x) => void (x.priority = before),
      async () => {
        // Windows may give less than was asked for (Realtime without administrator rights).
        const actual = await api.setPriority(p.pid, p.name, priority);
        if (actual === priority) return;
        const got = PRIORITIES.find((x) => x.key === actual);
        if (got) shown = got.base;
        const asked = PRIORITIES.find((x) => x.key === priority)?.label ?? priority;
        this.notice.show({
          code: priority === "realtime" ? "needs-admin" : "failed",
          message:
            priority === "realtime"
              ? `Windows set ${p.name} to ${got?.label ?? actual}: Realtime needs this app to run as administrator.`
              : `Windows set ${p.name} to ${got?.label ?? actual} instead of ${asked}.`,
        });
      },
    );
  }

  private async chooseAffinity(p: Proc): Promise<void> {
    let d: ProcDetails;
    try {
      d = await api.details(p.pid);
    } catch (e) {
      this.notice.show(asError(e));
      return;
    }
    const all = maskToCpus(d.system_affinity);
    if (!all.length) {
      this.notice.show({ code: "needs-admin", message: `Changing the processors of ${p.name} needs administrator rights.` });
      return;
    }
    const allowed = new Set(maskToCpus(d.affinity));
    const boxes = all.map((cpu) => {
      const box = h("input", { type: "checkbox", checked: allowed.has(cpu), value: String(cpu) });
      return { cpu, box, row: h("label", { class: "check" }, box, `CPU ${cpu}`) };
    });
    const everyone = h("input", {
      type: "checkbox",
      checked: allowed.size === all.length,
      onchange: () => {
        for (const b of boxes) b.box.checked = everyone.checked;
      },
    });
    for (const b of boxes) {
      b.box.addEventListener("change", () => {
        everyone.checked = boxes.every((x) => x.box.checked);
      });
    }
    await formDialog(
      `Processors "${p.name}" may use`,
      [h("label", { class: "check all" }, everyone, "All processors"), h("div", { class: "checks cpus" }, ...boxes.map((b) => b.row))],
      "Apply",
      async () => {
        try {
          await api.setAffinity(p.pid, p.name, cpusToMask(boxes.filter((b) => b.box.checked).map((b) => b.cpu)));
          return null;
        } catch (e) {
          return asError(e);
        }
      },
    );
  }

  private async createDump(p: Proc): Promise<void> {
    this.notice.info(`Writing a dump file of ${p.name}…`);
    try {
      const path = await api.createDump(p.pid, p.name);
      this.notice.info(`Dump file written: ${path}`, [
        { label: "Open file location", run: () => void api.openPath(path).catch(this.fail) },
        { label: "Copy path", run: () => copy(path) },
      ]);
    } catch (e) {
      this.notice.show(asError(e));
    }
  }

  private async restartExplorer(): Promise<void> {
    const ok = await confirmDialog("Restart Windows Explorer? The taskbar and desktop disappear for a moment and open folder windows close.", "Restart");
    if (!ok) return;
    this.notice.clear();
    await api.restartExplorer().catch(this.fail);
  }

  async runNewTask(): Promise<void> {
    const input = h("input", {
      class: "search wide",
      type: "text",
      spellcheck: false,
      placeholder: "Program, folder, document or web address",
      "aria-label": "What to run",
    });
    const admin = h("input", { type: "checkbox" });
    await formDialog(
      "Run new task",
      [input, h("label", { class: "check" }, admin, "Run as administrator")],
      "Run",
      async () => {
        try {
          await api.runTask(input.value, admin.checked);
          return null;
        } catch (e) {
          return asError(e);
        }
      },
    );
  }

  // ── ending processes ──

  private async endSelected(tree: boolean): Promise<void> {
    const row = this.selectedRow();
    if (!row || row.kind === "section") return;
    closeMenu();
    this.notice.clear();

    if (row.kind === "group") {
      const members = membersOf(this.procs, row);
      const ok = await confirmDialog(
        `End all ${members.length} "${row.name}" processes? Unsaved work in them will be lost.`,
        `End all ${members.length}`,
      );
      if (!ok) return;
      const results = await Promise.allSettled(members.map((p) => api.kill(p.pid, p.name, false)));
      this.afterKill(
        members.filter((_, i) => results[i].status === "fulfilled").map((p) => p.pid),
        results.flatMap((r) => (r.status === "rejected" ? [asError(r.reason)] : [])).filter((e) => e.code !== "not-found"),
        members.length,
      );
      return;
    }

    const p = row.rep!;
    if (tree) {
      const ok = await confirmDialog(`End "${p.name}" and every process it started? Unsaved work in them will be lost.`, "End tree");
      if (!ok) return;
    } else if (p.pid === this.ownPid) {
      const ok = await confirmDialog("This is the task manager itself. End it?", "End task manager");
      if (!ok) return;
    }
    try {
      const report = await api.kill(p.pid, p.name, tree);
      const failures = report.failed.map((f) => ({ code: f.code, message: f.message }) as CmdError);
      this.afterKill(report.ended, failures, report.ended.length + report.failed.length);
    } catch (e) {
      const err = asError(e);
      // It ended by itself first: the row is simply gone on the next refresh.
      if (err.code === "not-found") this.afterKill([p.pid], [], 1);
      else this.notice.show(err);
    }
  }

  /** Takes ended processes off the screen at once and reports only what could not be ended. */
  private afterKill(ended: number[], failures: CmdError[], total: number): void {
    if (ended.length) {
      const gone = new Set(ended);
      this.procs = this.procs.filter((p) => !gone.has(p.pid));
      this.rebuild();
    }
    if (!failures.length) return;
    const needsAdmin = failures.some((f) => f.code === "needs-admin");
    const first = failures[0].message;
    this.notice.show({
      code: needsAdmin ? "needs-admin" : failures[0].code,
      message: total > 1 ? `Ended ${ended.length} of ${total}. ${first}` : first,
    });
  }

  // ── details ──

  private async showDetails(pid: number): Promise<void> {
    this.detailsPid = pid;
    this.drawer.hidden = false;
    let d: ProcDetails;
    try {
      d = await api.details(pid);
    } catch (e) {
      if (this.detailsPid !== pid) return;
      this.drawer.replaceChildren(this.drawerHead("Details"), h("p", { class: "empty" }, asError(e).message));
      return;
    }
    if (this.detailsPid !== pid) return; // a newer request replaced this one
    const live = this.procs.find((p) => p.pid === pid);
    const status = [
      live && live.flags & F_HUNG ? "Not responding" : "",
      d.suspended ? "Suspended" : "",
      d.efficiency ? "Efficiency mode" : "",
    ].filter(Boolean);
    const cpus = maskToCpus(d.affinity).length;
    const allCpus = maskToCpus(d.system_affinity).length;
    const priority = PRIORITIES.find((p) => p.key === d.priority)?.label ?? "";
    const item = (k: string, v: string) => [h("dt", null, k), h("dd", null, v || "–")];
    const scrollTop = this.drawer.scrollTop;
    const list = h(
      "dl",
      null,
      ...item("Description", d.description),
      ...item("Publisher", d.company),
      ...item("PID", String(d.pid)),
      ...item("Status", status.join(", ") || "Running"),
      ...item("User", d.user),
      ...item("Runs as administrator", d.elevated === null ? "" : d.elevated ? "Yes" : "No"),
      ...item("Architecture", d.architecture),
      ...item("Priority", priority),
      ...item("Processors it may use", allCpus ? (cpus === allCpus ? `All ${allCpus}` : `${cpus} of ${allCpus}`) : ""),
      ...item("CPU time used", clock(d.cpu_time_ms)),
      ...item("Memory", bytes(d.memory)),
      ...item("Working set", `${bytes(d.working_set)} (peak ${bytes(d.peak_working_set)})`),
      ...item("Commit size", bytes(d.commit)),
      ...item("Page faults", countText(d.page_faults)),
      ...item("Threads", countText(d.threads)),
      ...item("Handles", countText(d.handles)),
      ...item("Session", String(d.session)),
      ...item("Started", d.start_ms ? new Date(d.start_ms).toLocaleString() : ""),
      ...item("Started by", d.parent_pid ? `${d.parent_name || "a process that has ended"} (${d.parent_pid})` : ""),
      ...item("Program file", d.exe || "Windows does not reveal this (protected process)"),
      ...item("Command line", d.command),
    );
    this.drawer.replaceChildren(this.drawerHead(d.name), list);
    this.drawer.scrollTop = scrollTop;
  }

  private drawerHead(title: string): HTMLElement {
    return h(
      "h3",
      null,
      h("span", null, title),
      h("button", { class: "icon-btn", title: "Close details", "aria-label": "Close details", onclick: () => this.closeDrawer() }, "×"),
    );
  }

  private closeDrawer(): void {
    this.detailsPid = 0;
    this.drawer.hidden = true;
    this.drawer.replaceChildren();
  }

  // ── called by the shell ──

  focusSearch(): void {
    this.search.focus();
    this.search.select();
  }

  /** Puts text in the search box (the Users tab uses `session:2` to show one person's processes). */
  filterBy(text: string): void {
    this.search.value = text;
    this.filter = text;
    this.lastAim = 0;
    this.rebuild();
  }

  /** Selects one process and scrolls to it, opening whatever it is folded inside. */
  showPid(pid: number): void {
    const p = this.procs.find((x) => x.pid === pid);
    if (!p) {
      // The list has not arrived yet (the tab was just opened): try again on the next refresh.
      this.wantPid = pid;
      return;
    }
    this.wantPid = 0;
    if (this.filter) this.filterBy("");
    const key = `p:${pid}`;
    if (!this.rows.some((r) => r.key === key)) {
      this.expanded.add(p.name.toLowerCase());
      this.collapsed.clear();
      this.lastAim = 0;
      this.rebuild();
    }
    this.followUntil = performance.now() + FOLLOW_MS;
    this.select(key, true);
  }

  /** Esc: first clears the search, then closes details. Returns true when it did something. */
  escape(): boolean {
    if (this.search.value) {
      this.filterBy("");
      return true;
    }
    if (!this.drawer.hidden) {
      this.closeDrawer();
      return true;
    }
    if (this.selected) {
      this.select(null);
      return true;
    }
    return false;
  }

  onShow(): void {
    this.list.render();
  }
}
