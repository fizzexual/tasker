// Start-up and the frame around the five tabs.

import { api, type AppStatus, type Hardware, type PerfSample, type TabName, type Tick } from "./api";
import { cpuReadout, diskReadout, gpuReadout, memReadout, type Readout } from "./readout";
import { CAPACITY, PerformanceTab } from "./performance";
import { ProcessesTab } from "./processes";
import { ServicesTab } from "./services";
import { SettingsDialog } from "./settings";
import { StartupTab } from "./startup";
import { initTitlebar } from "./titlebar";
import { closeMenu, installMenuDismiss, isMenuOpen, restartAsAdmin } from "./ui";
import { UsersTab } from "./users";
import { setText } from "./vlist";

const TABS: TabName[] = ["processes", "performance", "users", "startup", "services"];

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

async function boot(): Promise<void> {
  installMenuDismiss();
  // The page is an app, not a document: no browser menu, no text-drag.
  window.addEventListener("contextmenu", (e) => {
    if (!(e.target as HTMLElement).closest("input")) e.preventDefault();
  });

  let current: TabName = "processes";
  // Declared before the tabs because they call back into each other.
  let show: (tab: TabName) => void = () => undefined;

  const services = new ServicesTab($("tab-services"), (pid) => {
    show("processes");
    processes.showPid(pid);
  });
  const processes = new ProcessesTab($("tab-processes"), (pid) => {
    show("services");
    services.showForPid(pid);
  });
  const performance_ = new PerformanceTab($("tab-performance"));
  const users = new UsersTab($("tab-users"), (session) => {
    show("processes");
    processes.filterBy(`session:${session}`);
  });
  const startup = new StartupTab($("tab-startup"));

  const history: PerfSample[] = [];
  performance_.setHistory(history);

  let status: AppStatus = await api.status();
  const paintStatus = (s: AppStatus) => {
    const themeChanged = s.settings.theme !== document.documentElement.dataset.theme;
    status = s;
    $("admin-btn").hidden = s.elevated;
    $("admin-badge").hidden = !s.elevated;
    $("paused-chip").hidden = s.settings.interval_ms !== 0;
    document.documentElement.dataset.theme = s.settings.theme;
    processes.setOwnPid(s.own_pid);
    // The graphs read their colours from the theme, so they are redrawn when it changes.
    if (themeChanged && current === "performance") performance_.render();
  };
  paintStatus(status);
  const settings = new SettingsDialog(status, paintStatus);
  $("settings-btn").addEventListener("click", () => settings.open());
  $("admin-btn").addEventListener("click", () => void restartAsAdmin());
  $("paused-chip").addEventListener("click", () => void api.refreshNow());

  show = (tab: TabName) => {
    if (tab === current) return;
    closeMenu();
    if (current === "services") services.onHide();
    if (current === "users") users.onHide();
    current = tab;
    for (const t of TABS) {
      $(`tab-${t}`).hidden = t !== tab;
      document.querySelector(`.tab[data-tab="${t}"]`)!.setAttribute("aria-selected", String(t === tab));
    }
    void api.setView(tab);
    if (tab === "processes") processes.onShow();
    else if (tab === "performance") {
      performance_.render();
      void refreshInfo();
    } else if (tab === "users") users.onShow();
    else if (tab === "startup") void startup.onShow();
    else services.onShow();
  };
  for (const b of document.querySelectorAll<HTMLElement>(".tab")) {
    b.addEventListener("click", () => show(b.dataset.tab as TabName));
  }

  const refreshInfo = async () => {
    try {
      const [info, hardware] = await Promise.all([api.systemInfo(), api.hardware()]);
      performance_.setInfo(info);
      performance_.setHardware(hardware);
      hw = hardware;
      startup.setBiosTime(hardware.bios_ms);
      if (current === "performance") performance_.render();
    } catch {
      /* the graphs still work without the static details */
    }
  };

  /** Hardware facts (which disk is an SSD); arrive a moment after the first numbers. */
  let hw: Hardware | null = null;
  /** One top-bar readout: the percentage and, beside it, the real value. Hidden when the PC has no such part. */
  const paintChip = (name: "cpu" | "mem" | "gpu" | "disk", r: Readout | null) => {
    const chip = $(`chip-${name}`).parentElement!;
    if (chip.hidden !== !r) chip.hidden = !r;
    if (!r) return;
    setText($(`chip-${name}`), r.pct);
    const fill = String(r.fill);
    if (chip.dataset.fill !== fill) {
      chip.dataset.fill = fill;
      chip.style.setProperty("--p", fill);
    }
    setText($(`chip-${name}-more`), r.more);
  };

  const onTick = (tick: Tick) => {
    history.push(tick.perf);
    if (history.length > CAPACITY) history.splice(0, history.length - CAPACITY);
    paintChip("cpu", cpuReadout(tick.perf));
    paintChip("mem", memReadout(tick.perf));
    const g = gpuReadout(tick.perf);
    const d = diskReadout(tick.perf, hw);
    paintChip("gpu", g);
    paintChip("disk", d);
    if (d) setText($("chip-disk-label"), d.label);
    settings.setCost(tick.cost_ms);
    if (tick.procs && current === "processes") processes.update(tick.procs, tick.perf.mem_total);
    if (tick.procs && current === "users") users.update(tick.procs);
    if (current === "performance") performance_.render();
  };

  // Fill the graphs with what was measured while the window was closed.
  const refill = async () => {
    const past = await api.history();
    history.splice(0, history.length, ...past.slice(-CAPACITY));
    if (current === "performance") performance_.render();
  };
  await refill();
  await api.onTick(onTick);
  await api.setView(current);
  if (status.settings.start_tab !== current) show(status.settings.start_tab);
  document.addEventListener("visibilitychange", () => {
    if (!document.hidden) void refill();
  });

  window.addEventListener("keydown", (e) => {
    if (document.querySelector("dialog[open]")) return;
    const typing = (e.target as HTMLElement).tagName === "INPUT";
    const key = e.key.toLowerCase();
    if (e.ctrlKey && key === "f") {
      e.preventDefault();
      if (current === "processes") processes.focusSearch();
      else if (current === "startup") startup.focusSearch();
      else if (current === "services") services.focusSearch();
      return;
    }
    if (e.ctrlKey && key === "n") {
      e.preventDefault();
      void processes.runNewTask();
      return;
    }
    if (e.ctrlKey && e.key >= "1" && e.key <= String(TABS.length)) {
      e.preventDefault();
      show(TABS[Number(e.key) - 1]);
      return;
    }
    if (e.key === "F5") {
      // "Refresh now", as in Windows Task Manager. Works while paused too.
      e.preventDefault();
      void api.refreshNow();
      if (current === "startup") void startup.onShow();
      return;
    }
    if (e.key === "Escape") {
      if (isMenuOpen()) {
        closeMenu();
        return;
      }
      const used =
        current === "processes" ? processes.escape() : current === "startup" ? startup.escape() : current === "services" ? services.escape() : false;
      if (used) e.preventDefault();
      return;
    }
    // Browser shortcuts that make no sense in an app.
    if ((e.ctrlKey && ["r", "p", "g", "u", "j"].includes(key)) || (e.key === "F3" && !typing)) {
      e.preventDefault();
    }
  });

  void refreshInfo();
  // Tell the app the screen is drawn; only now does the window appear (no white flash).
  requestAnimationFrame(() => void api.uiReady());
}

// First, and not part of boot: the window must stay closable even when the screen fails to load.
initTitlebar();

boot().catch((e) => {
  document.body.append(Object.assign(document.createElement("pre"), { textContent: `Could not start: ${String(e)}` }));
  void api.uiReady().catch(() => undefined);
});
