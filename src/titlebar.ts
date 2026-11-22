// The app draws its own title bar (Windows' frame is switched off), so minimise, maximise and
// close live here. Dragging and double-click-to-maximise are done by the bar's drag attribute.

import { api } from "./api";

export interface MaxButton {
  label: "Maximize" | "Restore";
  icon: "max" | "restore";
}

/** What the middle button offers: fill the screen, or go back to the window's own size. */
export function maxButton(maximized: boolean): MaxButton {
  return maximized ? { label: "Restore", icon: "restore" } : { label: "Maximize", icon: "max" };
}

/**
 * Wires the three window buttons. Called before anything else starts, so the window can always
 * be closed, even when the rest of the screen failed to load.
 */
export function initTitlebar(): void {
  const root = document.documentElement;
  const min = document.getElementById("win-min");
  const max = document.getElementById("win-max");
  const close = document.getElementById("win-close");
  if (!min || !max || !close) return;

  const paint = (maximized: boolean) => {
    const b = maxButton(maximized);
    max.dataset.icon = b.icon;
    max.title = b.label;
    max.setAttribute("aria-label", b.label);
    // A window that fills the screen has no edge to grab, and its corner button reaches the corner.
    root.toggleAttribute("data-maximized", maximized);
  };

  // The newest question wins: a slow answer about an older state never repaints over a newer one.
  let asked = 0;
  const ask = (question: () => Promise<boolean>) => {
    const mine = ++asked;
    question()
      .then((maximized) => {
        if (mine === asked) paint(maximized);
      })
      .catch(() => undefined);
  };

  min.addEventListener("click", () => void api.windowMinimize().catch(() => undefined));
  max.addEventListener("click", () => ask(api.windowToggleMaximize));
  close.addEventListener("click", () => void api.windowClose().catch(() => undefined));

  // Maximising by double click, by snapping or from the keyboard also arrives as a resize.
  window.addEventListener("resize", () => ask(api.windowIsMaximized));

  const focus = () => root.toggleAttribute("data-inactive", !document.hasFocus());
  window.addEventListener("focus", focus);
  window.addEventListener("blur", focus);

  ask(api.windowIsMaximized);
  focus();
}
