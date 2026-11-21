// Small shared pieces: element builder, context menu, error notice, confirm dialog.

import { api, asError, type CmdError } from "./api";

type Child = Node | string | null | undefined | false;

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  props: Record<string, unknown> | null = null,
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (props) {
    for (const [k, v] of Object.entries(props)) {
      if (v === null || v === undefined || v === false) continue;
      if (k === "class") el.className = String(v);
      else if (k.startsWith("on") && typeof v === "function") el.addEventListener(k.slice(2), v as EventListener);
      else if (k in el && k !== "list") (el as unknown as Record<string, unknown>)[k] = v;
      else el.setAttribute(k, v === true ? "" : String(v));
    }
  }
  for (const c of children) {
    if (c === null || c === undefined || c === false) continue;
    el.append(c);
  }
  return el;
}

// ── context menu ──

export interface MenuItem {
  label: string;
  hint?: string;
  danger?: boolean;
  /** Shows a tick in front: the item is the current choice, or a switch that is on. */
  checked?: boolean;
  disabled?: boolean;
  run: () => void;
}

const menuEl = () => document.getElementById("menu") as HTMLDivElement;

export function isMenuOpen(): boolean {
  return !menuEl().hidden;
}

export function closeMenu(): void {
  const m = menuEl();
  if (!m.hidden) {
    m.hidden = true;
    m.replaceChildren();
  }
}

export function openMenu(x: number, y: number, items: (MenuItem | "sep")[]): void {
  const m = menuEl();
  m.replaceChildren(
    ...items.map((it) =>
      it === "sep"
        ? h("hr")
        : h(
            "button",
            {
              role: it.checked === undefined ? "menuitem" : "menuitemcheckbox",
              "aria-checked": it.checked === undefined ? null : String(it.checked),
              class: it.danger ? "danger" : "",
              disabled: it.disabled,
              onclick: () => {
                closeMenu();
                it.run();
              },
            },
            h("span", { class: "tick" }, it.checked ? "✓" : ""),
            h("span", { class: "lbl" }, it.label),
            it.hint ? h("kbd", null, it.hint) : null,
          ),
    ),
  );
  m.hidden = false;
  // Keep the whole menu on screen.
  const r = m.getBoundingClientRect();
  m.style.left = `${Math.max(4, Math.min(x, window.innerWidth - r.width - 4))}px`;
  m.style.top = `${Math.max(4, Math.min(y, window.innerHeight - r.height - 4))}px`;
  (m.querySelector("button:not(:disabled)") as HTMLButtonElement | null)?.focus();
}

export function installMenuDismiss(): void {
  window.addEventListener("pointerdown", (e) => {
    if (isMenuOpen() && !menuEl().contains(e.target as Node)) closeMenu();
  });
  window.addEventListener("blur", closeMenu);
  window.addEventListener("resize", closeMenu);
  menuEl().addEventListener("keydown", (e) => {
    const buttons = [...menuEl().querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
    if (e.key === "ArrowDown") buttons[(at + 1) % buttons.length]?.focus();
    else if (e.key === "ArrowUp") buttons[(at - 1 + buttons.length) % buttons.length]?.focus();
    else if (e.key === "Escape") closeMenu();
    else return;
    e.preventDefault();
    e.stopPropagation();
  });
}

// ── error notice ──
// Only failures get a message. A success is visible in the data itself (the row is gone,
// the switch moved), so it needs no pop-up.

export class Notice {
  readonly el = h("div", { class: "notice", role: "alert", hidden: true });

  show(error: CmdError): void {
    if (error.code === "cancelled") return;
    const parts: Child[] = [h("span", { class: "msg" }, error.message)];
    if (error.code === "needs-admin") {
      parts.push(
        h(
          "button",
          {
            class: "btn",
            onclick: () => restartAsAdmin(this),
          },
          "Restart as administrator",
        ),
      );
    }
    parts.push(h("button", { class: "x", title: "Dismiss", "aria-label": "Dismiss", onclick: () => this.clear() }, "×"));
    this.el.replaceChildren(...(parts as Node[]));
    this.el.classList.remove("info");
    this.el.hidden = false;
  }

  /**
   * A result the user has to be told because it is not visible anywhere else (where a dump
   * file was written). Never used for a plain "it worked".
   */
  info(message: string, actions: { label: string; run: () => void }[] = []): void {
    this.el.replaceChildren(
      h("span", { class: "msg" }, message),
      ...actions.map((a) => h("button", { class: "btn", onclick: a.run }, a.label)),
      h("button", { class: "x", title: "Dismiss", "aria-label": "Dismiss", onclick: () => this.clear() }, "×"),
    );
    this.el.classList.add("info");
    this.el.hidden = false;
  }

  clear(): void {
    this.el.hidden = true;
    this.el.classList.remove("info");
    this.el.replaceChildren();
  }
}

export async function restartAsAdmin(notice?: Notice): Promise<void> {
  try {
    await api.restartElevated();
  } catch (e) {
    notice?.show(asError(e));
  }
}

// ── confirm dialog ──

export function confirmDialog(text: string, action: string): Promise<boolean> {
  const dlg = document.getElementById("confirm") as HTMLDialogElement;
  return new Promise((resolve) => {
    const done = (yes: boolean) => {
      dlg.close();
      resolve(yes);
    };
    dlg.replaceChildren(
      h("p", { class: "confirm-text" }, text),
      h(
        "div",
        { class: "dialog-foot" },
        h("button", { class: "btn", onclick: () => done(false) }, "Cancel"),
        h("button", { class: "btn danger", id: "confirm-yes", onclick: () => done(true) }, action),
      ),
    );
    dlg.oncancel = (e) => {
      e.preventDefault();
      done(false);
    };
    dlg.showModal();
    (dlg.querySelector("#confirm-yes") as HTMLButtonElement).focus();
  });
}

// ── form dialog ──

/**
 * A small dialog with its own content and one action button. `submit` does the work and
 * returns an error to show (the dialog then stays open with what was typed) or null when
 * done. Resolves true when the action went through, false when it was cancelled.
 */
export function formDialog(
  title: string,
  body: Child[],
  action: string,
  submit: () => Promise<CmdError | null>,
): Promise<boolean> {
  const dlg = document.getElementById("modal") as HTMLDialogElement;
  return new Promise((resolve) => {
    const error = h("p", { class: "set-err", role: "alert", hidden: true });
    const ok = h("button", { class: "btn primary", type: "submit" }, action);
    const close = (done: boolean) => {
      dlg.close();
      dlg.replaceChildren();
      resolve(done);
    };
    const form = h(
      "form",
      {
        method: "dialog",
        onsubmit: async (e: Event) => {
          e.preventDefault();
          if (ok.disabled) return;
          ok.disabled = true;
          const failed = await submit();
          ok.disabled = false;
          if (!failed) return close(true);
          if (failed.code === "cancelled") return;
          error.textContent = failed.message;
          error.hidden = false;
        },
      },
      h("h2", null, title),
      ...body,
      error,
      h("div", { class: "dialog-foot" }, h("button", { class: "btn", type: "button", onclick: () => close(false) }, "Cancel"), ok),
    );
    dlg.replaceChildren(form);
    dlg.oncancel = (e) => {
      e.preventDefault();
      close(false);
    };
    dlg.showModal();
    (form.querySelector("input, select, button.primary") as HTMLElement | null)?.focus();
  });
}

/** Copies text; a refused clipboard is not worth a message. */
export function copy(text: string): void {
  void navigator.clipboard.writeText(text).catch(() => undefined);
}
