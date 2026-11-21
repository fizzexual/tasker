// The Startup apps tab: what starts with Windows, each with an on/off switch.

import { api, asError, type StartupEntry } from "./api";
import { copy, h, Notice, openMenu, type MenuItem } from "./ui";

export class StartupTab {
  private entries: StartupEntry[] = [];
  private filter = "";
  private loaded = false;
  /** Per entry: a save is on its way, and what the user wants once it lands. */
  private readonly saving = new Map<string, { want: boolean }>();
  private readonly listEl: HTMLElement;
  private readonly countEl: HTMLElement;
  private readonly search: HTMLInputElement;
  private readonly notice = new Notice();
  private readonly biosEl = h("span", { class: "count", title: "How long the PC took before Windows started loading" });

  /** "Last BIOS time", as Windows Task Manager shows it. */
  setBiosTime(ms: number): void {
    this.biosEl.textContent = ms > 0 ? `Last BIOS time: ${(ms / 1000).toFixed(1)} s` : "";
  }

  private menu(e: StartupEntry, ev: MouseEvent): void {
    ev.preventDefault();
    const fail = (err: unknown) => this.notice.show(asError(err));
    const items: (MenuItem | "sep")[] = [
      { label: e.enabled ? "Switch off" : "Switch on", run: () => void this.toggle(e.id, !e.enabled) },
      "sep",
    ];
    if (e.path) {
      items.push(
        { label: "Open file location", run: () => void api.openPath(e.path).catch(fail) },
        { label: "Properties", run: () => void api.propertiesOf(e.path).catch(fail) },
      );
    }
    items.push(
      { label: "Search online", run: () => void api.searchOnline(e.publisher ? `${e.name} ${e.publisher}` : e.name).catch(fail) },
      "sep",
      { label: "Copy name", run: () => copy(e.name) },
      { label: "Copy command", run: () => copy(e.command) },
    );
    openMenu(ev.clientX, ev.clientY, items);
  }

  constructor(root: HTMLElement) {
    this.search = h("input", {
      class: "search",
      type: "search",
      placeholder: "Search  (Ctrl+F)",
      spellcheck: false,
      "aria-label": "Search startup apps",
      oninput: () => {
        this.filter = this.search.value.trim().toLowerCase();
        this.paint();
      },
    });
    this.countEl = h("span", { class: "count" });
    this.listEl = h("div", { class: "startup-list" });
    root.append(
      h(
        "div",
        { class: "toolbar" },
        this.search,
        this.countEl,
        h("span", { class: "spacer" }),
        this.biosEl,
        h("button", { class: "btn", onclick: () => void this.load() }, "Refresh"),
      ),
      this.listEl,
      this.notice.el,
    );
  }

  async onShow(): Promise<void> {
    await this.load();
  }

  focusSearch(): void {
    this.search.focus();
    this.search.select();
  }

  escape(): boolean {
    if (!this.search.value) return false;
    this.search.value = "";
    this.filter = "";
    this.paint();
    return true;
  }

  private async load(): Promise<void> {
    try {
      const fresh = await api.startup();
      // An entry with a save still on its way keeps showing what the user picked.
      this.entries = fresh.map((e) => {
        const pending = this.saving.get(e.id);
        return pending ? { ...e, enabled: pending.want } : e;
      });
      this.loaded = true;
      this.paint();
    } catch (e) {
      this.notice.show(asError(e));
    }
  }

  private paint(): void {
    const shown = this.entries.filter(
      (e) =>
        !this.filter ||
        e.name.toLowerCase().includes(this.filter) ||
        e.command.toLowerCase().includes(this.filter) ||
        e.publisher.toLowerCase().includes(this.filter),
    );
    const on = this.entries.filter((e) => e.enabled).length;
    this.countEl.textContent = this.loaded ? `${on} of ${this.entries.length} switched on` : "";

    const head = h(
      "div",
      { class: "startup-row head" },
      h("span", null, "Name"),
      h("span", { class: "pub" }, "Publisher"),
      h("span", null, "Starts"),
      h("span", { class: "loc" }, "Where it is set"),
      h("span", null, "On"),
    );
    if (this.loaded && shown.length === 0) {
      this.listEl.replaceChildren(
        head,
        h("div", { class: "empty" }, this.entries.length ? `Nothing matches "${this.search.value.trim()}".` : "Nothing starts with Windows here."),
      );
      return;
    }
    this.listEl.replaceChildren(
      head,
      ...shown.map((e) =>
        h(
          "div",
          { class: "startup-row", "data-id": e.id, oncontextmenu: (ev: MouseEvent) => this.menu(e, ev) },
          h("span", { class: "nm", title: e.name }, e.name),
          h("span", { class: "pub cmd", title: e.publisher }, e.publisher),
          h("span", { class: "cmd", title: e.command }, e.command),
          h("span", { class: "loc cmd" }, e.location),
          h("input", {
            type: "checkbox",
            class: "switch",
            role: "switch",
            checked: e.enabled,
            "aria-label": `${e.name} starts with Windows`,
            title: e.all_users ? "Applies to every user of this PC (needs administrator rights)" : "",
            onchange: (ev: Event) => void this.toggle(e.id, (ev.target as HTMLInputElement).checked),
          }),
        ),
      ),
    );
  }

  private setShown(id: string, enabled: boolean): void {
    const entry = this.entries.find((e) => e.id === id);
    if (entry) entry.enabled = enabled;
    const box = this.listEl.querySelector<HTMLInputElement>(`[data-id="${CSS.escape(id)}"] .switch`);
    if (box && box.checked !== enabled) box.checked = enabled;
    const on = this.entries.filter((e) => e.enabled).length;
    this.countEl.textContent = `${on} of ${this.entries.length} switched on`;
  }

  /**
   * The switch moves at once and is never locked. If it is flipped again while the first save
   * is still on its way, the last pick wins: it is sent as soon as the first save returns.
   * A refused save puts the switch back where Windows really has it.
   */
  private async toggle(id: string, want: boolean): Promise<void> {
    this.notice.clear();
    this.setShown(id, want);
    const pending = this.saving.get(id);
    if (pending) {
      pending.want = want;
      return;
    }
    const slot = { want };
    this.saving.set(id, slot);
    let actual = !want;
    try {
      for (;;) {
        const sent = slot.want;
        actual = await api.setStartup(id, sent);
        if (slot.want === sent) break; // nothing newer was picked meanwhile
      }
      this.saving.delete(id);
      this.setShown(id, actual);
    } catch (e) {
      this.saving.delete(id);
      // Ask Windows what is really set, rather than guessing.
      const fresh = await api.startup().catch(() => null);
      const real = fresh?.find((x) => x.id === id)?.enabled ?? actual;
      this.setShown(id, real);
      this.notice.show(asError(e));
    }
  }
}
