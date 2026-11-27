// Slide elements in the Edit view: new elements from the insert toolbar, the app's element
// clipboard (copy / paste / duplicate with fresh IDs), names and kinds for the Objects list.

import type { CharStyle, Color, Element, ElementPatch, Fill, FillKind, MediaAssetId, Shape, Slide, TextBox } from "$protocol";
import type { Rect, Size } from "./geometry";
import { dataLinkLabel, type DataLinkChoice } from "../pages/stage/stage";

export type ShapeChoice = "rectangle" | "roundedRectangle" | "ellipse" | "triangle" | "line";

export const SHAPE_CHOICES: { id: ShapeChoice; label: string }[] = [
  { id: "rectangle", label: "Rectangle" },
  { id: "roundedRectangle", label: "Rounded Rectangle" },
  { id: "ellipse", label: "Ellipse" },
  { id: "triangle", label: "Triangle" },
  { id: "line", label: "Line" },
];

/** What the Objects list shows as an element's kind (and picks its icon by). */
export type ElementType = "text" | "image" | "line" | "ellipse" | "triangle" | "shape" | "other";

/** A fill is drawn only when it says so (H09-1): the server leaves `enabled: false` out of a
 * switched-off fill, as its model defaults a missing `enabled` to off. */
export function fillDrawn(f: Fill | undefined): boolean {
  return f?.enabled === true;
}

export function elementType(e: Element): ElementType {
  const k = e.kind;
  if (k?.type === "text") return "text";
  if (fillDrawn(e.fill) && e.fill?.kind?.type === "media") return "image";
  if (k?.type !== "shape") return "other";
  switch (k.shape?.type) {
    case "ellipse":
      return "ellipse";
    case "isoscelesTriangle":
    case "rightTriangle":
      return "triangle";
    case "path":
      return (k.shape.path.points?.length ?? 0) <= 2 && !k.shape.path.closed ? "line" : "shape";
    default:
      return "shape";
  }
}

/** Name for the Objects list: the element's own, else its data link, else its kind. */
export function elementName(e: Element): string {
  if (e.name?.trim()) return e.name;
  if (e.dataLink) return dataLinkLabel(e.dataLink);
  switch (elementType(e)) {
    case "text":
      return "Text";
    case "image":
      return "Image";
    case "line":
      return "Line";
    case "ellipse":
      return "Ellipse";
    case "triangle":
      return "Triangle";
    case "shape":
      return "Shape";
    default:
      return "Object";
  }
}

export function boundsOf(e: Element): Rect {
  const b = e.bounds ?? {};
  return { x: b.x ?? 0, y: b.y ?? 0, width: b.width ?? 0, height: b.height ?? 0 };
}

/** A rect of `w` × `h` centred on the canvas. */
function centred(canvas: Size, w: number, h: number): Rect {
  return { x: Math.round((canvas.width - w) / 2), y: Math.round((canvas.height - h) / 2), width: Math.round(w), height: Math.round(h) };
}

const WHITE: Color = { r: 1, g: 1, b: 1 };

/** The character style new text boxes start with: the slide's first text box's, else plain white. */
export function prevailingStyle(slide: Slide | null, fallbackFamily: string): CharStyle {
  for (const e of slide?.elements ?? []) {
    if (e.kind?.type === "text" && e.kind.text?.defaultStyle?.fontFamily) {
      const s = e.kind.text.defaultStyle;
      return { fontFamily: s.fontFamily, fontWeight: s.fontWeight ?? 400, size: s.size ?? 72, color: s.color ?? WHITE };
    }
  }
  return { fontFamily: fallbackFamily, fontWeight: 400, size: 72, color: WHITE };
}

export function newTextElement(id: string, canvas: Size, style: CharStyle, text = "Text"): Element {
  const text_: TextBox = {
    paragraphs: [{ runs: [{ text }] }],
    defaultStyle: style,
    defaultParagraph: { alignment: "center" },
    verticalAlign: "middle",
  };
  return {
    id,
    name: "Text",
    bounds: centred(canvas, canvas.width * 0.6, canvas.height * 0.3),
    kind: { type: "text", shape: { type: "rectangle" }, text: text_ },
  };
}

/** A scrolling text ticker across the lower part of the canvas (moving left, looping). */
export function newScrollingTextElement(id: string, canvas: Size, style: CharStyle): Element {
  const h = Math.round(canvas.height * 0.1);
  return {
    ...newTextElement(id, canvas, style, "Scrolling text"),
    name: "Scrolling text",
    bounds: { x: 0, y: canvas.height - h * 1.5, width: canvas.width, height: h },
    scroll: { enabled: true },
  };
}

/**
 * A text element showing live data (`choice.link`); its text is the design-time preview. A
 * picture link (slide image, song artwork) gets a frame (16:9, or `choice.aspect`) with a thin
 * outline and no text.
 */
export function newDataLinkElement(id: string, choice: DataLinkChoice, canvas: Size, style: CharStyle): Element {
  const picture = !!choice.picture;
  const w = picture ? canvas.width * 0.4 : canvas.width * 0.6;
  const h = picture ? w / (choice.aspect ?? 16 / 9) : canvas.height * 0.2;
  const text: TextBox = {
    paragraphs: [{ runs: [{ text: picture ? "" : choice.preview }] }],
    defaultStyle: style,
    defaultParagraph: { alignment: "center" },
    verticalAlign: "middle",
    fit: "shrinkToFit",
  };
  return {
    id,
    name: dataLinkLabel(choice.link),
    bounds: centred(canvas, w, h),
    ...(picture ? { aspectLocked: true, stroke: { enabled: true, width: 2, color: { r: 0.5, g: 0.5, b: 0.5 } } } : {}),
    kind: { type: "text", shape: { type: "rectangle" }, text },
    dataLink: choice.link,
  };
}

export function newShapeElement(id: string, choice: ShapeChoice, canvas: Size): Element {
  const s = Math.min(canvas.width, canvas.height) * 0.4;
  const fill = { enabled: true, kind: { type: "color" as const, color: { r: 0.8, g: 0.8, b: 0.8 } } };
  const label = SHAPE_CHOICES.find((c) => c.id === choice)?.label ?? "Shape";
  if (choice === "line") {
    const shape: Shape = {
      type: "path",
      path: { closed: false, points: [{ point: { x: 0, y: 0.5 } }, { point: { x: 1, y: 0.5 } }] },
    };
    return {
      id,
      name: label,
      bounds: centred(canvas, canvas.width * 0.4, 20),
      kind: { type: "shape", shape },
      fill: { enabled: false },
      stroke: { enabled: true, width: 6, color: WHITE },
    };
  }
  const shape: Shape =
    choice === "roundedRectangle"
      ? { type: "roundedRectangle", roundness: 0.15 }
      : choice === "ellipse"
        ? { type: "ellipse" }
        : choice === "triangle"
          ? { type: "isoscelesTriangle" }
          : { type: "rectangle" };
  const w = choice === "rectangle" || choice === "roundedRectangle" ? s * 1.6 : s;
  return { id, name: label, bounds: centred(canvas, w, s), kind: { type: "shape", shape: shape }, fill };
}

/** An image element: a rectangle filled with the asset, sized to fit half the canvas. */
export function newImageElement(id: string, asset: MediaAssetId, name: string, canvas: Size, dims: Size | null): Element {
  let w = canvas.width * 0.5,
    h = canvas.height * 0.5;
  if (dims && dims.width > 0 && dims.height > 0) {
    const k = Math.min(w / dims.width, h / dims.height);
    w = dims.width * k;
    h = dims.height * k;
  }
  return {
    id,
    name: name || "Image",
    bounds: centred(canvas, w, h),
    aspectLocked: true,
    kind: { type: "shape", shape: { type: "rectangle" } },
    fill: { enabled: true, kind: { type: "media", asset, display: { scale: "fit" } } },
  };
}

/** A video element: plays the video muted in its box, looping, from when the slide goes live. */
export function newVideoElement(id: string, asset: MediaAssetId, name: string, canvas: Size, dims: Size | null): Element {
  const e = newImageElement(id, asset, name || "Video", canvas, dims);
  return { ...e, fill: { enabled: true, kind: { type: "media", asset, display: { scale: "fit" }, playback: { repeat: { type: "loop" } } } } };
}

/**
 * A media fill with another image or video (Fill ▸ Choose…, D10-4): the current fill's display
 * settings are kept, except the ones in the old file's own pixels and time (its crop and
 * corrective rotation); a video keeps its playback (loop, end, fades) without the old clip's in
 * and out points, or loops when it had none, and keeps its sound (mute, level: E10-2); an image
 * has no playback or sound.
 */
export function mediaFillWith(current: Fill | undefined, asset: MediaAssetId, kind: "image" | "video" | string): Fill {
  const cur = current?.kind?.type === "media" ? current.kind : null;
  const { crop: _crop, rotation: _rotation, ...display } = cur?.display ?? { scale: "fit" };
  if (kind === "video") {
    const { inPoint: _in, outPoint: _out, ...playback } = cur?.playback ?? { repeat: { type: "loop" } };
    return { enabled: true, kind: { type: "media", asset, display, playback, ...(cur?.audio ? { audio: cur.audio } : {}) } };
  }
  return { enabled: true, kind: { type: "media", asset, display } };
}

/**
 * One setting of a media or live-source fill changed on `e`'s own fill (Loop, Start, Mute,
 * Level, the source, scaling): with several objects selected each keeps its own clip or source,
 * and an object whose fill is of another kind is left alone (E10-3). Each keeps its own Fill
 * switch too: a fill switched off stays off (F11-2).
 */
export function fillKindPatch<T extends "media" | "liveSource">(e: Element, type: T, change: (k: Extract<FillKind, { type: T }>) => Extract<FillKind, { type: T }>): ElementPatch {
  const k = e.fill?.kind;
  if (k?.type !== type) return {};
  return { fill: { ...e.fill, kind: change(k as Extract<FillKind, { type: T }>) } };
}

// ---- clipboard -------------------------------------------------------------------------------------

/** What Copy puts on the app's clipboard. */
export interface ElementClipboard {
  kind: "elements";
  /** Slide they were copied from (paste onto it offsets the copies). */
  slide: string;
  /** Bottom to top. */
  elements: Element[];
}

/** Copies the given elements of a slide, in z-order. */
export function copyElements(slide: Slide, ids: Iterable<string>): ElementClipboard {
  const want = new Set(ids);
  return {
    kind: "elements",
    slide: slide.id,
    elements: structuredClone((slide.elements ?? []).filter((e) => want.has(e.id))),
  };
}

/** Offset of pasted / duplicated copies, canvas px. */
export const PASTE_OFFSET = 20;

/**
 * Copies of clipboard elements with fresh IDs. Pasted onto the slide they came from (or
 * duplicated), they are offset so they do not hide the originals.
 */
export function pasteElements(clip: ElementClipboard, targetSlide: Slide, newId: () => string): Element[] {
  const present = new Set((targetSlide.elements ?? []).map((e) => e.id));
  const offset = clip.slide === targetSlide.id || clip.elements.some((e) => present.has(e.id)) ? PASTE_OFFSET : 0;
  return clip.elements.map((e) => {
    const copy: Element = structuredClone(e);
    copy.id = newId();
    if (offset && copy.bounds) copy.bounds = { ...copy.bounds, x: (copy.bounds.x ?? 0) + offset, y: (copy.bounds.y ?? 0) + offset };
    // A copy is user content, not a template slot.
    delete copy.placeholder;
    return copy;
  });
}

/** Ops-ready: z index of an element (0 = bottom), or -1. */
export function zIndex(slide: Slide, id: string): number {
  return (slide.elements ?? []).findIndex((e) => e.id === id);
}
