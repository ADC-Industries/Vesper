// The Stage page's checkboxes (plan 4.6, question 10): a stage layout is a one-slide document;
// "On this layout" adds and removes data-link elements (clock, timer, current / next slide,
// notes, stage message) with edit ops, and the element settings change them (which timer, the
// clock's 12 / 24 h, the current slide's text size). Anything else is "Design freely…" in the
// slide editor. Pure functions: they read the layout and return the ops to send.

import type { CharStyle, Color, DataLink, Element, PresentationOp, Slide, TextBox } from "$protocol";
import { uuid as newId } from "../../edit/ids";

export type StageElementKind = "clock" | "timer" | "current" | "next" | "notes" | "message";

export const STAGE_ELEMENTS: readonly { kind: StageElementKind; label: string }[] = [
  { kind: "clock", label: "Clock" },
  { kind: "timer", label: "Timer" },
  { kind: "current", label: "Current slide" },
  { kind: "next", label: "Next slide" },
  { kind: "notes", label: "Slide notes" },
  { kind: "message", label: "Stage message" },
];

/** Which checkbox an element belongs to (by its data link), or null. */
export function stageKindOf(e: Element): StageElementKind | null {
  const l = e.dataLink;
  switch (l?.type) {
    case "clock":
      return "clock";
    case "timer":
      return "timer";
    case "slideText":
      return l.which === "next" ? "next" : "current";
    case "slideNotes":
      return "notes";
    case "message":
      return "message";
    default:
      return null;
  }
}

export function elementsOf(slide: Slide | undefined, kind: StageElementKind): Element[] {
  return (slide?.elements ?? []).filter((e) => stageKindOf(e) === kind);
}

/** Which checkboxes are on for a layout. */
export function composition(slide: Slide | undefined): Record<StageElementKind, boolean> {
  const out = { clock: false, timer: false, current: false, next: false, notes: false, message: false };
  for (const e of slide?.elements ?? []) {
    const k = stageKindOf(e);
    if (k) out[k] = true;
  }
  return out;
}

/** "Current · Next · Clock · Timer" for a layout card. */
export function compositionText(slide: Slide | undefined): string {
  const c = composition(slide);
  const chords = (slide?.elements ?? []).some((e) => e.dataLink?.type === "chordChart");
  const parts = [chords && "Chords", c.current && "Current", c.next && "Next", c.notes && "Notes", c.clock && "Clock", c.timer && "Timer", c.message && "Message"].filter(Boolean);
  if (parts.length) return parts.join(" · ");
  return (slide?.elements?.length ?? 0) ? "Designed freely" : "Empty";
}

type Box = { x: number; y: number; width: number; height: number };

/** Where a new element goes on a 1920 × 1080 canvas (scaled to the layout's size). */
const SLOTS: Record<StageElementKind, Box & { size: number; align: "left" | "center" | "right" }> = {
  clock: { x: 40, y: 20, width: 600, height: 120, size: 84, align: "left" },
  timer: { x: 1280, y: 20, width: 600, height: 120, size: 84, align: "right" },
  current: { x: 40, y: 160, width: 1840, height: 700, size: 110, align: "center" },
  next: { x: 1300, y: 160, width: 580, height: 700, size: 72, align: "center" },
  notes: { x: 40, y: 870, width: 1840, height: 90, size: 48, align: "left" },
  message: { x: 40, y: 970, width: 1840, height: 96, size: 60, align: "center" },
};
/** The current slide's box while the next slide is beside it. */
const CURRENT_WITH_NEXT: Box = { x: 40, y: 160, width: 1220, height: 700 };

const PREVIEW: Record<StageElementKind, string> = {
  clock: "12:00 PM",
  timer: "00:00",
  current: "Current Slide",
  next: "Next Slide",
  notes: "Slide Notes",
  message: "Stage Message",
};

const WHITE: Color = { r: 1, g: 1, b: 1 };
const AMBER: Color = { r: 0.96, g: 0.77, b: 0.42 };
const BLUE: Color = { r: 0.75, g: 0.84, b: 1 };

export interface ComposeOptions {
  /** The layout's canvas (default 1920 × 1080). */
  width?: number;
  height?: number;
  /** The timer a new timer element shows (null = the most recently started). */
  timer?: { id: string; name?: string } | null;
  use24Hour?: boolean;
}

function scaled(b: Box, w: number, h: number): Box {
  const sx = w / 1920;
  const sy = h / 1080;
  return { x: Math.round(b.x * sx), y: Math.round(b.y * sy), width: Math.round(b.width * sx), height: Math.round(b.height * sy) };
}

const sameBox = (a: Element["bounds"], b: Box) => !!a && (a.x ?? 0) === b.x && (a.y ?? 0) === b.y && a.width === b.width && a.height === b.height;

/** The font the layout already uses (its first text box), else a common sans. */
function fontOf(slide: Slide | undefined): Pick<CharStyle, "fontFamily" | "fontWeight"> {
  for (const e of slide?.elements ?? []) {
    const s = e.kind?.type === "text" ? e.kind.text?.defaultStyle : undefined;
    if (s?.fontFamily) return { fontFamily: s.fontFamily, fontWeight: s.fontWeight ?? 700 };
  }
  return { fontFamily: "DejaVu Sans", fontWeight: 700 };
}

export function linkFor(kind: StageElementKind, opts: ComposeOptions = {}): DataLink {
  switch (kind) {
    case "clock":
      return { type: "clock", format: { use24Hour: !!opts.use24Hour } };
    case "timer":
      return opts.timer ? { type: "timer", timer: { id: opts.timer.id, ...(opts.timer.name ? { name: opts.timer.name } : {}) } } : { type: "timer" };
    case "current":
      return { type: "slideText" };
    case "next":
      return { type: "slideText", which: "next" };
    case "notes":
      return { type: "slideNotes" };
    case "message":
      return { type: "message", flash: true };
  }
}

/** A new element for a checkbox, in its slot. */
export function newStageElement(kind: StageElementKind, slide: Slide | undefined, opts: ComposeOptions = {}): Element {
  const w = opts.width ?? 1920;
  const h = opts.height ?? 1080;
  const slot = SLOTS[kind];
  const c = composition(slide);
  const box = kind === "current" && c.next ? CURRENT_WITH_NEXT : slot;
  const color = kind === "next" ? AMBER : kind === "notes" ? BLUE : WHITE;
  const text: TextBox = {
    paragraphs: [{ runs: [{ text: PREVIEW[kind] }] }],
    defaultStyle: { ...fontOf(slide), size: Math.round((slot.size * h) / 1080), color },
    defaultParagraph: { alignment: slot.align },
    verticalAlign: kind === "current" || kind === "next" ? "top" : "middle",
    fit: "shrinkToFit",
  };
  return {
    id: newId(),
    name: STAGE_ELEMENTS.find((x) => x.kind === kind)!.label,
    bounds: scaled(box, w, h),
    kind: { type: "text", shape: { type: "rectangle" }, text },
    dataLink: linkFor(kind, opts),
  };
}

/**
 * The ops that turn a checkbox on or off. On adds the element in its slot (and narrows a
 * full-width current slide that the page placed when the next slide joins it); off deletes every
 * element of that kind (and widens the current slide back).
 */
export function toggleOps(slide: Slide, kind: StageElementKind, on: boolean, opts: ComposeOptions = {}): PresentationOp[] {
  const w = opts.width ?? 1920;
  const h = opts.height ?? 1080;
  const ops: PresentationOp[] = [];
  if (on) {
    if (elementsOf(slide, kind).length) return [];
    ops.push({ type: "addElement", slide: slide.id, element: newStageElement(kind, slide, opts) });
  } else {
    for (const e of elementsOf(slide, kind)) ops.push({ type: "deleteElement", slide: slide.id, element: e.id });
  }
  if (kind === "next") {
    const [from, to] = on ? [SLOTS.current, CURRENT_WITH_NEXT] : [CURRENT_WITH_NEXT, SLOTS.current];
    for (const e of elementsOf(slide, "current"))
      if (sameBox(e.bounds, scaled(from, w, h))) ops.push({ type: "setElement", slide: slide.id, element: e.id, patch: { bounds: scaled(to, w, h) } });
  }
  return ops;
}

/** Replaces the data link of every element of a kind (delete and re-add at the same place). */
export function relinkOps(slide: Slide, kind: StageElementKind, link: (old: DataLink) => DataLink): PresentationOp[] {
  const ops: PresentationOp[] = [];
  const list = slide.elements ?? [];
  list.forEach((e, index) => {
    if (stageKindOf(e) !== kind || !e.dataLink) return;
    const next = link(e.dataLink);
    if (JSON.stringify(next) === JSON.stringify(e.dataLink)) return;
    ops.push({ type: "deleteElement", slide: slide.id, element: e.id });
    ops.push({ type: "addElement", slide: slide.id, element: { ...e, id: newId(), dataLink: next }, index });
  });
  return ops;
}

/** The timer the layout's timer element shows: its ID, "" for the most recently started, null without one. */
export function layoutTimer(slide: Slide | undefined): string | null {
  const e = elementsOf(slide, "timer")[0];
  if (e?.dataLink?.type !== "timer") return null;
  return e.dataLink.timer?.id ?? "";
}

/** Whether the layout's clock shows 24-hour time (null without a clock). */
export function layoutClock24(slide: Slide | undefined): boolean | null {
  const e = elementsOf(slide, "clock")[0];
  if (e?.dataLink?.type !== "clock") return null;
  return !!e.dataLink.format?.use24Hour;
}

/** The current slide's text size (px on the layout's canvas), null without one. */
export function currentTextSize(slide: Slide | undefined): number | null {
  const e = elementsOf(slide, "current")[0];
  return e?.kind?.type === "text" ? (e.kind.text?.defaultStyle?.size ?? 72) : null;
}

/** Sets the current slide's text size; the next slide follows at two thirds. */
export function textSizeOps(slide: Slide, size: number): PresentationOp[] {
  const ops: PresentationOp[] = [];
  for (const e of elementsOf(slide, "current")) ops.push({ type: "setTextStyle", slide: slide.id, element: e.id, character: { size } });
  for (const e of elementsOf(slide, "next")) ops.push({ type: "setTextStyle", slide: slide.id, element: e.id, character: { size: Math.round(size * 0.68) } });
  return ops;
}
