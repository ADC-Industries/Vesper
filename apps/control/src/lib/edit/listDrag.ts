// Pointer drag to reorder rows of a list (slides, objects): a press that moves more than a few
// px becomes a drag; an insertion line follows the pointer; the drop reports the row index the
// dragged row should go before (rows.length = at the end).

export interface ListDragOptions {
  /** The scrolling list holding the rows. */
  list: HTMLElement;
  /** Selector of the rows (in display order). */
  rows: string;
  /** Called with the insertion line (viewport px) while dragging, null when it ends. */
  line: (l: { y: number; left: number; width: number } | null) => void;
  /** The drop: insert before display row `index`. */
  drop: (index: number) => void;
}

const THRESHOLD = 5;

/** Starts tracking a press on a row; returns false when it is not a primary-button press. */
export function pressRow(e: PointerEvent, o: ListDragOptions): boolean {
  if (e.button !== 0) return false;
  const x0 = e.clientX,
    y0 = e.clientY;
  let dragging = false;
  let index = -1;
  const move = (ev: PointerEvent) => {
    if (!dragging && Math.hypot(ev.clientX - x0, ev.clientY - y0) < THRESHOLD) return;
    dragging = true;
    const rows = [...o.list.querySelectorAll<HTMLElement>(o.rows)];
    const box = o.list.getBoundingClientRect();
    index = rows.length;
    let y = rows.length ? rows[rows.length - 1]!.getBoundingClientRect().bottom : box.top + 2;
    for (let i = 0; i < rows.length; i++) {
      const b = rows[i]!.getBoundingClientRect();
      if (ev.clientY < b.top + b.height / 2) {
        index = i;
        y = b.top;
        break;
      }
    }
    o.line({ y, left: box.left + 4, width: box.width - 8 });
  };
  const end = (ev: PointerEvent) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", end);
    window.removeEventListener("keydown", key, true);
    o.line(null);
    if (dragging && ev.type === "pointerup" && index >= 0) {
      // The click after a drag must not select the row again.
      const swallow = (c: Event) => {
        c.stopPropagation();
        c.preventDefault();
      };
      window.addEventListener("click", swallow, { capture: true, once: true });
      setTimeout(() => window.removeEventListener("click", swallow, { capture: true }), 0);
      o.drop(index);
    }
  };
  const key = (ev: KeyboardEvent) => {
    if (ev.key === "Escape") {
      ev.stopPropagation();
      dragging = false;
      end(new PointerEvent("pointercancel"));
    }
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", end);
  window.addEventListener("keydown", key, true);
  return true;
}

/**
 * Where a row moved from display index `from` to before display index `to` ends up, as its
 * index in the list without it; null when it does not move.
 */
export function moveTarget(from: number, to: number): number | null {
  if (to === from || to === from + 1) return null;
  return to > from ? to - 1 : to;
}
