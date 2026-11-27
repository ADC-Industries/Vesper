// Dragging media tiles onto a bin or media playlist in the Library's tree (G17). Pointer events
// (as the Show page's drag and drop: works with touch and in the desktop shell, whose file drop
// handling takes over HTML drag and drop): a press on a tile that moves past a few pixels starts
// the drag, a ghost follows the pointer, and the tree's rows marked `data-drop-bin="bin:<id>"`
// or `"playlist:<id>"` light up under it. Releasing over one adds the files.

import type { CollectionMedia } from "$protocol";
import { binsPage, type BinTarget } from "../../state/bins.svelte";

const THRESHOLD = 5;

/** `data-drop-bin`'s value → the target. */
export function parseDropTarget(v: string | null | undefined): BinTarget | null {
  const m = /^(bin|playlist):(.+)$/.exec(v ?? "");
  return m ? { kind: m[1] as "bin" | "playlist", id: m[2]! } : null;
}

class BinDrag {
  /** The files dragged, while dragging. */
  files = $state.raw<CollectionMedia[] | null>(null);
  x = $state(0);
  y = $state(0);
  /** The drop row under the pointer (`data-drop-bin`'s value). */
  over = $state<string | null>(null);
  /** A drag just ended: the click that follows the release is not a tile click. */
  private suppress = false;

  /** A press on a media tile: may become a drag. */
  press(e: PointerEvent, files: CollectionMedia[]) {
    if (e.button !== 0 || !files.length) return;
    const x0 = e.clientX;
    const y0 = e.clientY;
    const move = (m: PointerEvent) => {
      if (!this.files) {
        if (Math.hypot(m.clientX - x0, m.clientY - y0) < THRESHOLD) return;
        this.files = files;
      }
      this.x = m.clientX;
      this.y = m.clientY;
      const el = document.elementFromPoint(m.clientX, m.clientY)?.closest<HTMLElement>("[data-drop-bin]");
      this.over = el?.dataset.dropBin ?? null;
    };
    const end = (u: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("pointercancel", end);
      const dragged = this.files;
      const target = u.type === "pointerup" ? parseDropTarget(this.over) : null;
      this.files = null;
      this.over = null;
      if (!dragged) return;
      this.suppress = true;
      setTimeout(() => (this.suppress = false), 0);
      if (target) void binsPage.addMedia(target, dragged);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  }

  /** True for the click right after a drag (ignore it). */
  swallowClick(): boolean {
    return this.suppress;
  }
}

export const binDrag = new BinDrag();
