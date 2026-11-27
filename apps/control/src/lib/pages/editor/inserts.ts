// Inserting objects (the Edit page's toolbar and the canvas's floating insert bar in the
// Library): each is one undo step; the new object is selected, and a new text box opens for
// typing.

import type { Element, MediaBinItemInfo } from "$protocol";
import type { Connection } from "../../server";
import type { EditState } from "../../edit/editState.svelte";
import { SHAPE_CHOICES, newDataLinkElement, newImageElement, newScrollingTextElement, newShapeElement, newTextElement, newVideoElement, prevailingStyle, type ShapeChoice } from "../../edit/elements";
import { uuid } from "../../edit/ids";
import type { DataLinkChoice } from "../stage/stage";

/** Adds an element to the current slide and selects it; `edit` opens it for typing. */
export async function insertElement(st: EditState, el: Element, label: string, edit?: (id: string) => void): Promise<boolean> {
  const slide = st.slide;
  if (!slide) return false;
  await st.finishText?.();
  const ok = await st.run(label, [{ type: "addElement", slide: slide.id, element: el }]);
  if (!ok) return false;
  st.select([el.id]);
  edit?.(el.id);
  return true;
}

function style(st: EditState) {
  return prevailingStyle(st.slide, st.fonts?.fallbackFamily || "DejaVu Sans");
}

export function insertText(st: EditState, edit?: (id: string) => void) {
  return insertElement(st, newTextElement(uuid(), st.canvasSize(), style(st)), "Add Text", edit);
}

/** A scrolling text ticker (moving left, looping), opened for typing. */
export function insertScrollingText(st: EditState, edit?: (id: string) => void) {
  return insertElement(st, newScrollingTextElement(uuid(), st.canvasSize(), style(st)), "Add Scrolling Text", edit);
}

/** A text object showing live data (current / next slide, notes, clock, a timer, …). */
export function insertDataLink(st: EditState, c: DataLinkChoice) {
  return insertElement(st, newDataLinkElement(uuid(), c, st.canvasSize(), style(st)), `Add ${c.label}`);
}

export function insertShape(st: EditState, c: ShapeChoice) {
  return insertElement(st, newShapeElement(uuid(), c, st.canvasSize()), `Add ${SHAPE_CHOICES.find((x) => x.id === c)?.label ?? "Shape"}`);
}

/** An image (or a video: it plays in its box) from the media bins, at its own aspect ratio
 * when the server knows its size. */
export async function insertImage(st: EditState, conn: Connection, it: MediaBinItemInfo) {
  let dims: { width: number; height: number } | null = null;
  try {
    const info = await conn.rest.media(it.asset);
    const p = info.asset.probe;
    if (p?.width && p?.height) dims = { width: p.width, height: p.height };
  } catch {
    /* size unknown: half the canvas */
  }
  if (it.kind === "video") return insertElement(st, newVideoElement(uuid(), it.asset, it.name, st.canvasSize(), dims), "Add Video");
  return insertElement(st, newImageElement(uuid(), it.asset, it.name, st.canvasSize(), dims), "Add Image");
}
