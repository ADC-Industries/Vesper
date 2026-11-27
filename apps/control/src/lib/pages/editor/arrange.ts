// The Arrange tab's maths (several objects selected): align to each other or to the slide,
// distribute, match size and order. Pure: each function returns the ops for one undo step.

import type { Element, PresentationOp, Slide } from "$protocol";
import { boundsOf } from "../../edit/elements";
import { align, distribute, rotatedBounds, type AlignMode, type DistributeMode, type Rect, type Size } from "../../edit/geometry";
import { boundsOps } from "../../edit/ops";

/** Moves each element by the offset its rotated outline got (align and distribute work on outlines). */
function movedBy(els: Element[], out: { id: string; rect: Rect }[], items: { id: string; rect: Rect }[]) {
  return out.map((o) => {
    const from = items.find((i) => i.id === o.id)!.rect;
    const b = boundsOf(els.find((e) => e.id === o.id)!);
    return { id: o.id, rect: { ...b, x: b.x + o.rect.x - from.x, y: b.y + o.rect.y - from.y }, was: b };
  });
}

function outlines(els: Element[]) {
  return els.map((e) => ({ id: e.id, rect: rotatedBounds(boundsOf(e), e.rotation ?? 0) }));
}

/**
 * Aligns the elements' edges or centres: to each other (their common bounds), or to the slide
 * when `slide` is given. Locked elements stay where they are.
 */
export function alignOps(slideId: string, els: Element[], mode: AlignMode, slide?: Size): PresentationOp[] {
  const free = els.filter((e) => !e.locked);
  if (!free.length || (!slide && els.length < 2)) return [];
  const items = outlines(free);
  // To each other: the common bounds of every selected object (locked ones too, as anchors).
  const to = slide ? { x: 0, y: 0, width: slide.width, height: slide.height } : undefined;
  const all = to ?? unionOf(outlines(els).map((i) => i.rect));
  return boundsOps(slideId, movedBy(free, align(items, mode, all), items));
}

function unionOf(rs: Rect[]): Rect {
  const x = Math.min(...rs.map((r) => r.x));
  const y = Math.min(...rs.map((r) => r.y));
  const r = Math.max(...rs.map((r) => r.x + r.width));
  const b = Math.max(...rs.map((r) => r.y + r.height));
  return { x, y, width: r - x, height: b - y };
}

/** Equal gaps between three or more elements (the outermost two stay). */
export function distributeOps(slideId: string, els: Element[], mode: DistributeMode): PresentationOp[] {
  const free = els.filter((e) => !e.locked);
  if (free.length < 3) return [];
  const items = outlines(free);
  return boundsOps(slideId, movedBy(free, distribute(items, mode), items));
}

/**
 * Same width, height or both as the first selected element (the selection's anchor), keeping
 * each element's centre.
 */
export function matchSizeOps(slideId: string, els: Element[], dim: "width" | "height" | "both"): PresentationOp[] {
  const [first, ...rest] = els;
  if (!first || !rest.length) return [];
  const ref = boundsOf(first);
  return boundsOps(
    slideId,
    rest
      .filter((e) => !e.locked)
      .map((e) => {
        const b = boundsOf(e);
        const width = dim === "height" ? b.width : ref.width;
        const height = dim === "width" ? b.height : ref.height;
        return { id: e.id, rect: { x: b.x + (b.width - width) / 2, y: b.y + (b.height - height) / 2, width, height }, was: b };
      }),
  );
}

export type OrderMove = "front" | "forward" | "backward" | "back";

/** The slide's element IDs bottom to top after moving the selected ones. */
export function orderAfter(order: string[], selected: Set<string>, move: OrderMove): string[] {
  const sel = order.filter((id) => selected.has(id));
  const rest = order.filter((id) => !selected.has(id));
  if (move === "front") return [...rest, ...sel];
  if (move === "back") return [...sel, ...rest];
  const out = [...order];
  if (move === "forward") {
    // From the top down: a selected element swaps with the unselected one above it.
    for (let i = out.length - 2; i >= 0; i--) {
      if (selected.has(out[i]!) && !selected.has(out[i + 1]!)) [out[i], out[i + 1]] = [out[i + 1]!, out[i]!];
    }
  } else {
    for (let i = 1; i < out.length; i++) {
      if (selected.has(out[i]!) && !selected.has(out[i - 1]!)) [out[i], out[i - 1]] = [out[i - 1]!, out[i]!];
    }
  }
  return out;
}

/**
 * `reorderElement` ops (applied in order: take the element out, put it in at `index`, 0 =
 * bottom) that turn the slide's order into `orderAfter`'s.
 */
export function orderOps(slide: Slide, ids: Iterable<string>, move: OrderMove): PresentationOp[] {
  const now = (slide.elements ?? []).map((e) => e.id);
  const want = orderAfter(now, new Set(ids), move);
  const cur = [...now];
  const ops: PresentationOp[] = [];
  for (let i = 0; i < want.length; i++) {
    const id = want[i]!;
    if (cur[i] === id) continue;
    cur.splice(cur.indexOf(id), 1);
    cur.splice(i, 0, id);
    ops.push({ type: "reorderElement", slide: slide.id, element: id, index: i });
  }
  return ops;
}
