// The Users tab: who is signed in, what their programs use, and disconnect / sign out.

import { api, asError, type UserSession } from "./api";
import { bytes, percent, rate } from "./format";
import { fromRaw, sessionTotals, type Proc, type RawProc } from "./model";
import { confirmDialog, h, Notice } from "./ui";
import { setText } from "./vlist";

const REFRESH_MS = 5000;

const STATE_TEXT: Record<UserSession["state"], string> = {
  active: "Signed in",
  disconnected: "Disconnected",
  other: "",
};

interface Refs {
  cpu: HTMLElement;
  mem: HTMLElement;
  disk: HTMLElement;
  count: HTMLElement;
}

export class UsersTab {
  private users: UserSession[] = [];
  private procs: Proc[] = [];
  private loaded = false;
  private visible = false;
  private timer = 0;
  private readonly refs = new Map<number, Refs>();
  private readonly listEl = h("div", { class: "users-list" });
  private readonly countEl = h("span", { class: "count" });
  private readonly notice = new Notice();

  constructor(
    root: HTMLElement,
    /** Opens the Processes tab showing only one person's processes. */
    private readonly showProcesses: (session: number) => void,
  ) {
    root.append(h("div", { class: "toolbar" }, this.countEl), this.listEl, this.notice.el);
  }

  onShow(): void {
    this.visible = true;
    void this.load();
    window.clearInterval(this.timer);
    this.timer = window.setInterval(() => {
      if (this.visible && !document.hidden) void this.load();
    }, REFRESH_MS);
  }

  onHide(): void {
    this.visible = false;
    window.clearInterval(this.timer);
  }

  update(raw: readonly RawProc[]): void {
    this.procs = raw.map(fromRaw);
    this.paintNumbers();
  }

  private async load(): Promise<void> {
    try {
      const fresh = await api.users();
      const same = this.loaded && JSON.stringify(fresh) === JSON.stringify(this.users);
      this.users = fresh;
      this.loaded = true;
      if (!same) this.paint();
    } catch (e) {
      this.notice.show(asError(e));
    }
  }

  private paint(): void {
    this.refs.clear();
    setText(this.countEl, this.users.length === 1 ? "1 person signed in" : `${this.users.length} people signed in`);
    const head = h(
      "div",
      { class: "users-row head" },
      h("span", null, "User"),
      h("span", null, "Status"),
      h("span", { class: "num" }, "CPU"),
      h("span", { class: "num" }, "Memory"),
      h("span", { class: "num" }, "Disk"),
      h("span", { class: "num" }, "Processes"),
      h("span"),
    );
    if (!this.users.length) {
      this.listEl.replaceChildren(head, h("div", { class: "empty" }, "Windows did not report any signed-in user."));
      return;
    }
    this.listEl.replaceChildren(
      head,
      ...this.users.map((u) => {
        const r: Refs = {
          cpu: h("span", { class: "num" }),
          mem: h("span", { class: "num" }),
          disk: h("span", { class: "num" }),
          count: h("span", { class: "num" }),
        };
        this.refs.set(u.id, r);
        return h(
          "div",
          { class: "users-row", "data-id": String(u.id) },
          h("span", { class: "nm", title: u.domain ? `${u.domain}\\${u.user}` : u.user }, u.user, u.current ? h("span", { class: "muted" }, "  (you)") : null),
          h("span", { class: u.state === "active" ? "status-running" : "muted" }, STATE_TEXT[u.state]),
          r.cpu,
          r.mem,
          r.disk,
          r.count,
          h(
            "span",
            { class: "acts" },
            h("button", { class: "btn", onclick: () => this.showProcesses(u.id) }, "Show processes"),
            h("button", { class: "btn", onclick: () => void this.act(u, "disconnect") }, "Disconnect"),
            h("button", { class: "btn danger", onclick: () => void this.act(u, "sign-off") }, "Sign out"),
          ),
        );
      }),
    );
    this.paintNumbers();
  }

  private paintNumbers(): void {
    for (const u of this.users) {
      const r = this.refs.get(u.id);
      if (!r) continue;
      const t = sessionTotals(this.procs, u.id);
      setText(r.cpu, percent(t.cpu));
      setText(r.mem, bytes(t.mem));
      setText(r.disk, rate(t.disk));
      setText(r.count, String(t.count));
    }
  }

  private async act(u: UserSession, action: "disconnect" | "sign-off"): Promise<void> {
    const ok =
      action === "disconnect"
        ? await confirmDialog(
            u.current
              ? "Disconnect? Your programs keep running and the PC goes to the sign-in screen."
              : `Disconnect ${u.user}? Their programs keep running.`,
            "Disconnect",
          )
        : await confirmDialog(
            u.current
              ? "Sign out? Every program you have open is closed and unsaved work is lost."
              : `Sign out ${u.user}? Every program they have open is closed and their unsaved work is lost.`,
            "Sign out",
          );
    if (!ok) return;
    this.notice.clear();
    try {
      await api.userAction(u.id, action);
    } catch (e) {
      this.notice.show(asError(e));
    }
    await this.load();
  }
}
