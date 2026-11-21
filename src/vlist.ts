// A list that only builds the rows you can see.
//
// 500 processes would be 500 rows of DOM to lay out and repaint every second. Here only the
// ~30 visible rows exist; scrolling and refreshing re-use them and rewrite just the text that
// changed. That is the main reason the table stays smooth.

import { visibleRange } from "./model";

export class VirtualList<T> {
  private items: readonly T[] = [];
  private readonly space: HTMLDivElement;
  private readonly pool: HTMLElement[] = [];
  private readonly shown: number[] = [];
  private frame = 0;
  private viewport = 0;

  constructor(
    readonly scroller: HTMLElement,
    private readonly rowHeight: number,
    private readonly create: () => HTMLElement,
    private readonly bind: (el: HTMLElement, item: T, index: number) => void,
  ) {
    this.space = document.createElement("div");
    this.space.className = "vspace";
    scroller.appendChild(this.space);
    scroller.addEventListener("scroll", () => this.schedule(), { passive: true });
    new ResizeObserver(() => {
      this.viewport = scroller.clientHeight;
      this.schedule();
    }).observe(scroller);
    this.viewport = scroller.clientHeight;
  }

  setItems(items: readonly T[]): void {
    this.items = items;
    this.space.style.height = `${items.length * this.rowHeight}px`;
    this.render();
  }

  get length(): number {
    return this.items.length;
  }

  item(index: number): T | undefined {
    return this.items[index];
  }

  /** Index of the row under a mouse event, or -1. */
  indexAt(clientY: number): number {
    const rect = this.scroller.getBoundingClientRect();
    const y = clientY - rect.top + this.scroller.scrollTop;
    const i = Math.floor(y / this.rowHeight);
    return i >= 0 && i < this.items.length ? i : -1;
  }

  scrollToIndex(index: number): void {
    const top = index * this.rowHeight;
    const s = this.scroller;
    if (top < s.scrollTop) s.scrollTop = top;
    else if (top + this.rowHeight > s.scrollTop + s.clientHeight) s.scrollTop = top + this.rowHeight - s.clientHeight;
  }

  /** How many rows fit on screen (for PageUp/PageDown). */
  pageSize(): number {
    return Math.max(1, Math.floor(this.viewport / this.rowHeight) - 1);
  }

  private schedule(): void {
    if (this.frame) return;
    this.frame = requestAnimationFrame(() => {
      this.frame = 0;
      this.render();
    });
  }

  render(): void {
    if (!this.viewport) this.viewport = this.scroller.clientHeight;
    const { start, end } = visibleRange(this.scroller.scrollTop, this.viewport, this.rowHeight, this.items.length);
    const need = end - start;
    while (this.pool.length < need) {
      const el = this.create();
      el.classList.add("row");
      this.scroller.appendChild(el);
      this.pool.push(el);
      this.shown.push(-1);
    }
    for (let i = 0; i < this.pool.length; i++) {
      const el = this.pool[i];
      if (i >= need) {
        if (this.shown[i] !== -2) {
          el.style.display = "none";
          this.shown[i] = -2;
        }
        continue;
      }
      const index = start + i;
      if (this.shown[i] !== index) {
        if (this.shown[i] === -2) el.style.display = "";
        el.style.transform = `translateY(${index * this.rowHeight}px)`;
        this.shown[i] = index;
      }
      this.bind(el, this.items[index], index);
    }
  }
}

/** Sets text only when it changed, so an unchanged cell costs nothing to repaint. */
export function setText(el: Element, text: string): void {
  if (el.textContent !== text) el.textContent = text;
}
