// The Settings dialog. Every control is the value: picking it saves it. No Save button.

import { api, asError, type AppStatus, type Settings, type TabName, type Theme } from "./api";
import { hotkeyFromEvent, isPlainEscape } from "./model";
import { h, restartAsAdmin } from "./ui";

const SPEEDS: [number, string][] = [
  [0, "Paused"],
  [500, "0.5 s"],
  [1000, "1 s"],
  [2000, "2 s"],
  [5000, "5 s"],
];
const THEMES: [Theme, string][] = [
  ["system", "As Windows"],
  ["light", "Light"],
  ["dark", "Dark"],
];
const START_TABS: [TabName, string][] = [
  ["processes", "Processes"],
  ["performance", "Performance"],
  ["users", "Users"],
  ["startup", "Startup apps"],
  ["services", "Services"],
];

type Toggle = "autostart" | "close_to_tray" | "always_on_top" | "ctrl_shift_esc" | "hide_when_minimized";

export class SettingsDialog {
  private status: AppStatus;
  private lastCost = 0;
  private listening = false;
  /** Saves run one after another so two quick picks can never land out of order. */
  private queue: Promise<void> = Promise.resolve();
  private readonly dlg = document.getElementById("settings") as HTMLDialogElement;

  constructor(
    status: AppStatus,
    private readonly onChange: (status: AppStatus) => void,
  ) {
    this.status = status;
    this.dlg.addEventListener("close", () => {
      this.listening = false;
    });
    // Clicking the dimmed area outside the dialog closes it.
    this.dlg.addEventListener("click", (e) => {
      if (e.target === this.dlg) this.dlg.close();
    });
    this.dlg.addEventListener(
      "keydown",
      (e) => {
        if (!this.listening) return;
        e.preventDefault();
        e.stopPropagation();
        // Only Esc on its own cancels: Ctrl+Alt+Esc is a shortcut like any other.
        if (isPlainEscape(e)) {
          this.listening = false;
          this.paint();
          return;
        }
        const combo = hotkeyFromEvent(e);
        if (!combo) return; // still only modifiers held
        this.listening = false;
        this.save({ hotkey: combo }, "hotkey");
      },
      true,
    );
  }

  open(): void {
    this.paint();
    if (!this.dlg.open) this.dlg.showModal();
  }

  setCost(ms: number): void {
    this.lastCost = ms;
    const el = this.dlg.querySelector("#cost");
    if (el && this.dlg.open) el.textContent = this.costText();
  }

  private costText(): string {
    return this.lastCost ? `Last reading took ${this.lastCost.toFixed(1)} ms` : "";
  }

  /**
   * Shows the pick at once, then saves. If the save is refused, the control goes back to what
   * is really in effect and the reason is shown beside it.
   */
  private save(change: Partial<Settings>, field: keyof Settings): void {
    const wanted = { ...this.status.settings, ...change };
    const before = this.status;
    this.status = { ...this.status, settings: wanted };
    this.onChange(this.status);
    this.paint();
    this.queue = this.queue.then(async () => {
      try {
        // Build on the latest saved state, changing only this field.
        const latest = await api.status();
        const saved = await api.saveSettings({ ...latest.settings, ...change });
        this.status = saved;
        this.onChange(saved);
        this.paint();
      } catch (e) {
        const real = await api.status().catch(() => before);
        this.status = real;
        this.onChange(real);
        this.paint({ field, message: asError(e).message });
      }
    });
  }

  private paint(error?: { field: keyof Settings; message: string }): void {
    const s = this.status.settings;
    const scroll = this.dlg.querySelector(".set-body")?.scrollTop ?? 0;
    const err = (field: keyof Settings) =>
      error?.field === field ? h("div", { class: "set-err", role: "alert" }, error.message) : null;

    const toggle = (field: Toggle, label: string, hint: string, extra: Node | null = null) =>
      h(
        "label",
        { class: "set-row" },
        h("div", { class: "lab" }, h("div", null, label), h("div", { class: "hint" }, hint), err(field), extra),
        h("input", {
          type: "checkbox",
          class: "switch",
          role: "switch",
          "data-field": field,
          checked: s[field],
          onchange: (e: Event) => this.save({ [field]: (e.target as HTMLInputElement).checked } as Partial<Settings>, field),
        }),
      );
    const seg = <T extends string | number>(field: keyof Settings, label: string, hint: string, name: string, options: [T, string][], current: T) =>
      h(
        "div",
        { class: "set-row" },
        h("div", { class: "lab" }, h("div", null, label), h("div", { class: "hint" }, hint), err(field)),
        h(
          "div",
          { class: "seg", role: "group", "aria-label": name, "data-field": field },
          ...options.map(([value, text]) =>
            h("button", { "aria-pressed": String(current === value), onclick: () => this.save({ [field]: value } as Partial<Settings>, field) }, text),
          ),
        ),
      );

    const hotkeyError = error?.field === "hotkey" ? null : this.status.hotkey_error;
    const takeoverError =
      error?.field === "ctrl_shift_esc" || !s.ctrl_shift_esc || !this.status.takeover_error
        ? null
        : h("div", { class: "set-err", role: "alert" }, "Windows did not let this app take the keys. Switch it off and on to try again.");
    const body = h(
      "div",
      { class: "set-body" },
      seg("interval_ms", "Refresh speed", "How often the numbers update. F5 refreshes now.", "Refresh speed", SPEEDS, s.interval_ms),
      toggle(
        "ctrl_shift_esc",
        "Ctrl+Shift+Esc opens this app",
        this.status.elevated
          ? "Instead of Windows Task Manager, while this app is running."
          : "Instead of Windows Task Manager, while this app is running. Inside a window that runs as administrator Windows still opens its own, unless this app runs as administrator too.",
        takeoverError,
      ),
      h(
        "div",
        { class: "set-row" },
        h(
          "div",
          { class: "lab" },
          h("div", null, "Extra shortcut to show or hide"),
          h("div", { class: "hint" }, "Works from any program. Click the box, then press the keys."),
          err("hotkey"),
          hotkeyError ? h("div", { class: "set-err", role: "alert" }, hotkeyError) : null,
        ),
        h(
          "button",
          {
            class: `hotkey${this.listening ? " listening" : ""}`,
            id: "hotkey-box",
            onclick: () => {
              this.listening = !this.listening;
              this.paint();
              (this.dlg.querySelector("#hotkey-box") as HTMLElement).focus();
            },
          },
          this.listening ? "Press keys…  (Esc cancels)" : s.hotkey || "None",
        ),
        s.hotkey && !this.listening
          ? h("button", { class: "btn", title: "Remove the shortcut", onclick: () => this.save({ hotkey: "" }, "hotkey") }, "Clear")
          : null,
      ),
      toggle("autostart", "Start with Windows", "Starts hidden in the tray, ready for the shortcuts"),
      h(
        "div",
        { class: "set-row" },
        h("div", { class: "lab" }, h("div", null, "Open on"), h("div", { class: "hint" }, "The tab shown when the app starts"), err("start_tab")),
        h(
          "select",
          {
            class: "select",
            "aria-label": "Tab shown when the app starts",
            "data-field": "start_tab",
            onchange: (e: Event) => this.save({ start_tab: (e.target as HTMLSelectElement).value as TabName }, "start_tab"),
          },
          ...START_TABS.map(([value, text]) => h("option", { value, selected: s.start_tab === value }, text)),
        ),
      ),
      seg("theme", "Colours", "Light or dark", "Colours", THEMES, s.theme),
      toggle("close_to_tray", "Closing the window keeps it in the tray", "Off: closing the window quits the app"),
      toggle("hide_when_minimized", "Minimising sends it to the tray", "Off: it stays on the taskbar"),
      toggle("always_on_top", "Always on top", "Stays above other windows"),
      h(
        "div",
        { class: "set-row" },
        h(
          "div",
          { class: "lab" },
          h("div", null, this.status.elevated ? "Running as administrator" : "Running without administrator rights"),
          h(
            "div",
            { class: "hint" },
            this.status.elevated
              ? "System processes and services can be controlled."
              : "Needed to end system processes and to start or stop services.",
          ),
        ),
        this.status.elevated ? null : h("button", { class: "btn", onclick: () => void restartAsAdmin() }, "Restart as administrator"),
      ),
    );
    this.dlg.replaceChildren(
      h("h2", null, "Settings"),
      body,
      h(
        "div",
        { class: "dialog-foot" },
        h("span", { class: "about" }, `Version ${this.status.version}  ·  `, h("span", { id: "cost" }, this.costText())),
        h("button", { class: "btn", onclick: () => this.dlg.close() }, "Close"),
      ),
    );
    body.scrollTop = scroll;
  }
}
