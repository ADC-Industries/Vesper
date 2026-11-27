// Edit ops the Edit view sends for inspector changes, canvas gestures and slide list actions.
// Pure: each function returns the ops (the caller sends them as one batch = one undo step).

import type {
  CharStyle,
  Color,
  Element,
  ElementPatch,
  HotKey,
  Paragraph,
  ParagraphStyle,
  Presentation,
  PresentationOp,
  RichText,
  Slide,
  SongInfo,
  TextBox,
  TextRange,
  Transition,
  TransitionKind,
} from "$protocol";
import type { Rect } from "./geometry";
import { roundRect } from "./geometry";
import { cpLength, plainText } from "./text";

/** Model defaults (omitted from JSON while they hold): what the inspector shows for a missing field. */
export const DEFAULT_OUTLINE = { enabled: false, width: 3, color: { r: 0, g: 0, b: 0 } } as const;
export const DEFAULT_STROKE = { enabled: false, width: 3, color: { r: 1, g: 1, b: 1 } } as const;
export const DEFAULT_SHADOW = { enabled: false, angle: 315, offset: 5, radius: 5, color: { r: 0, g: 0, b: 0 }, opacity: 0.75 } as const;
export const DEFAULT_FEATHER = { enabled: false, style: "inside", radius: 0.05 } as const;
/** The server's default scrolling text (off; left at 120 px/s, automatic start, looping 120 px apart). */
export const DEFAULT_SCROLL = { enabled: false, direction: "left", speed: 120, start: "auto", repeat: true, gap: 120 } as const;

/** `setElement` for each element whose patch is not empty. */
export function setElements(slide: string, patches: { id: string; patch: ElementPatch }[]): PresentationOp[] {
  return patches.filter((p) => Object.keys(p.patch).length).map((p) => ({ type: "setElement", slide, element: p.id, patch: p.patch }));
}

/** New bounds for elements (rounded to 1/100 px); unchanged ones are skipped. */
export function boundsOps(slide: string, moved: { id: string; rect: Rect; was?: Rect }[]): PresentationOp[] {
  return setElements(
    slide,
    moved
      .map(({ id, rect, was }) => {
        const r = roundRect(rect);
        if (was) {
          const w = roundRect(was);
          if (w.x === r.x && w.y === r.y && w.width === r.width && w.height === r.height) return { id, patch: {} };
        }
        return { id, patch: { bounds: r } };
      }),
  );
}

/** A geometry field typed in the inspector; with the aspect lock, W and H change together. */
export function geometryPatch(e: Element, field: "x" | "y" | "width" | "height" | "rotation", value: number, aspectLocked: boolean): ElementPatch {
  if (!Number.isFinite(value)) return {};
  if (field === "rotation") return { rotation: ((value % 360) + 360) % 360 };
  const b = { x: e.bounds?.x ?? 0, y: e.bounds?.y ?? 0, width: e.bounds?.width ?? 0, height: e.bounds?.height ?? 0 };
  const next = { ...b, [field]: field === "width" || field === "height" ? Math.max(1, value) : value };
  if (aspectLocked && b.width > 0 && b.height > 0) {
    if (field === "width") next.height = (next.width * b.height) / b.width;
    if (field === "height") next.width = (next.height * b.width) / b.height;
  }
  return { bounds: roundRect(next) };
}

/**
 * Character / paragraph style for a text element. With a range (text selected in the
 * in-place editor) only those characters (and the paragraphs they touch). Without one, the
 * whole box: its defaults, and every run (so runs with their own colour or size follow too).
 */
export function textStyleOps(
  slide: string,
  e: Element,
  style: { character?: CharStyle; paragraph?: ParagraphStyle; clear?: string[] },
  range: TextRange | null,
): PresentationOp[] {
  const base = { type: "setTextStyle" as const, slide, element: e.id, ...style };
  if (range) return [{ ...base, range }];
  const tb = e.kind?.type === "text" ? e.kind.text : undefined;
  const len = cpLength(plainText(tb));
  const ops: PresentationOp[] = [base];
  if (len > 0) ops.push({ ...base, range: { start: 0, end: len } });
  return ops;
}

/** Box-level text settings (fit, margins, outline, text shadow, line mask, vertical align). */
export function textBoxOp(
  slide: string,
  element: string,
  fields: Omit<Extract<PresentationOp, { type: "setTextBox" }>, "type" | "slide" | "element">,
): PresentationOp {
  return { type: "setTextBox", slide, element, ...fields };
}

/**
 * Saving the in-place editor: a no-op `setSlideText` with `expectText` first (the batch fails
 * with a conflict when someone else changed the text meanwhile), then the new text box.
 * `force` (Keep mine) leaves out the precondition.
 */
export function saveTextOps(slide: string, element: string, before: string, textBox: TextBox, force = false): PresentationOp[] {
  const ops: PresentationOp[] = [];
  if (!force) ops.push({ type: "setSlideText", slide, element, text: before, expectText: before });
  ops.push({ type: "replaceTextBox", slide, element, textBox });
  return ops;
}

// ---- presentation and slide ------------------------------------------------------------------------

export function allSlides(p: Presentation | null): Slide[] {
  return (p?.sections ?? []).flatMap((s) => s.slides ?? []);
}

/**
 * The Presentation tab's background: every slide's background colour (`null` = off). The
 * model keeps the background per slide; this sets them all at once.
 */
export function presentationBackgroundOps(p: Presentation, color: Color | null): PresentationOp[] {
  return allSlides(p)
    .filter((s) => JSON.stringify(s.background ?? null) !== JSON.stringify(color))
    .map((s) => ({ type: "setSlide", slide: s.id, background: color }));
}

/** The common background of every slide: a colour, null (none), or "mixed". */
export function presentationBackground(p: Presentation | null): Color | null | "mixed" {
  const slides = allSlides(p);
  if (!slides.length) return null;
  const first = JSON.stringify(slides[0]!.background ?? null);
  if (slides.some((s) => JSON.stringify(s.background ?? null) !== first)) return "mixed";
  return slides[0]!.background ?? null;
}

/** One Copyright field. An empty text clears the field; all fields empty clears `songInfo`. */
export function songInfoOp(p: Presentation, field: keyof SongInfo, value: string | number | boolean | null): PresentationOp {
  const next: SongInfo = { ...(p.songInfo ?? {}) };
  if (value === null || value === "") delete next[field];
  else (next as Record<string, unknown>)[field] = value;
  const empty = Object.keys(next).length === 0;
  return { type: "setPresentation", songInfo: empty ? null : next };
}

/** Plain notes text → RichText (one unstyled run per line). */
export function notesFromText(text: string): RichText {
  const lines = text.split("\n");
  if (lines.length === 1 && lines[0] === "") return {};
  return { paragraphs: lines.map((l): Paragraph => (l ? { runs: [{ text: l }] } : {})) };
}

export function notesText(n: RichText | undefined): string {
  return (n?.paragraphs ?? []).map((p) => (p.runs ?? []).map((r) => r.text ?? "").join("")).join("\n");
}

/** Transition choices the inspector edits; anything else is shown read-only. */
export type TransitionChoice = "default" | "cut" | "dissolve" | "push" | "wipe" | "zoom" | "slide";

export function transitionChoice(t: Transition | undefined): TransitionChoice | "other" {
  if (!t || !t.kind) return t ? "cut" : "default";
  switch (t.kind.type) {
    case "cut":
    case "dissolve":
    case "push":
    case "wipe":
    case "zoom":
    case "slide":
      return t.kind.type;
    default:
      return "other";
  }
}

const DIRECTION_LABEL: Record<string, string> = {
  left: "left",
  right: "right",
  up: "up",
  down: "down",
  upLeft: "up-left",
  upRight: "up-right",
  downLeft: "down-left",
  downRight: "down-right",
  center: "from the centre",
};

export function transitionLabel(t: Transition | undefined): string {
  if (!t) return "None";
  const k = t.kind;
  const d = t.duration !== undefined ? ` (${t.duration} s)` : "";
  switch (k?.type) {
    case undefined:
    case "cut":
      return "Cut";
    case "dissolve":
      return `Dissolve${d}`;
    case "vendor":
      return `${k.name || "Vendor effect"}${d}`;
    case "unsupported":
      return `${k.sourceType || "Unsupported"}${d}`;
    case "fadeThroughColor":
      return `Fade through colour${d}`;
    case "push":
      return `Push ${DIRECTION_LABEL[k.direction] ?? k.direction}${d}`;
    case "wipe":
      return `Wipe ${DIRECTION_LABEL[k.direction] ?? k.direction}${d}`;
    case "slide":
      return `Slide ${DIRECTION_LABEL[k.direction] ?? k.direction}${d}`;
    case "zoom":
      return `Zoom ${k.direction === "out" ? "out" : "in"}${d}`;
    default:
      return `Unknown${d}`;
  }
}

/** The transition to store for a choice (`null` = the default: no override). A moving kind
 * keeps the direction and easing of `prev` when it has them (else left / ease in-out). */
export function transitionFor(choice: TransitionChoice, duration: number, prev?: Transition): Transition | null {
  if (choice === "default") return null;
  if (choice === "cut") return { kind: { type: "cut" } };
  const dur = Math.max(0, Math.round(duration * 100) / 100);
  if (choice === "dissolve") {
    const t: Transition = { kind: { type: "dissolve" }, duration: dur };
    if (prev?.kind?.type === "dissolve" && prev.easing && prev.easing !== "linear") t.easing = prev.easing;
    return t;
  }
  const pk = prev?.kind;
  const easing = prev?.easing ?? "easeInOut";
  if (choice === "zoom") {
    const out = pk?.type === "zoom" && pk.direction === "out";
    return { kind: out ? { type: "zoom", direction: "out" } : { type: "zoom" }, duration: dur, easing };
  }
  const prevDir = pk && (pk.type === "push" || pk.type === "wipe" || pk.type === "slide") ? pk.direction : "left";
  // Only a wipe opens from the centre.
  const direction = choice !== "wipe" && prevDir === "center" ? "left" : prevDir;
  const kind: TransitionKind = choice === "push" ? { type: "push", direction } : choice === "wipe" ? { type: "wipe", direction } : { type: "slide", direction };
  return { kind, duration: dur, easing };
}

/** A hotkey from a key press, or null for keys that cannot be one (modifiers alone, Esc). */
export function hotkeyFrom(e: { code: string; key: string; shiftKey: boolean; ctrlKey: boolean; altKey: boolean; metaKey: boolean }): HotKey | null {
  if (!e.code || ["ShiftLeft", "ShiftRight", "ControlLeft", "ControlRight", "AltLeft", "AltRight", "MetaLeft", "MetaRight", "Escape", "Tab"].includes(e.code)) return null;
  const k: HotKey = { code: e.code };
  if (e.shiftKey) k.shift = true;
  if (e.ctrlKey) k.ctrl = true;
  if (e.altKey) k.alt = true;
  if (e.metaKey) k.meta = true;
  return k;
}

export function hotkeyLabel(k: HotKey | undefined): string {
  if (!k?.code) return "";
  const key = k.code.replace(/^Key/, "").replace(/^Digit/, "");
  return [k.ctrl && "Ctrl", k.alt && "Alt", k.shift && "Shift", k.meta && "Meta", key].filter(Boolean).join("+");
}

/** Insert a slide after `after` (or at the end of the presentation's last section). */
export function addSlideOps(p: Presentation, after: string | null, newId: () => string): PresentationOp[] {
  const sections = p.sections ?? [];
  if (after) {
    const sec = sections.find((s) => (s.slides ?? []).some((sl) => sl.id === after));
    if (sec) return [{ type: "addSlide", section: sec.id, after }];
  }
  const last = sections[sections.length - 1];
  if (!last) {
    const id = newId();
    return [
      { type: "addSection", section: { id, name: "Group" } },
      { type: "addSlide", section: id },
    ];
  }
  const lastSlide = (last.slides ?? [])[(last.slides ?? []).length - 1];
  return [{ type: "addSlide", section: last.id, ...(lastSlide ? { after: lastSlide.id } : {}) }];
}

/** The z-index the Objects list's drop position means (the list shows top first). */
export function reorderIndex(count: number, listIndex: number): number {
  return Math.max(0, Math.min(count - 1, count - 1 - listIndex));
}

/** Font weight names for the weight picker. */
export const WEIGHT_NAMES: Record<number, string> = {
  100: "Thin",
  200: "Extra Light",
  300: "Light",
  400: "Regular",
  500: "Medium",
  600: "Semibold",
  700: "Bold",
  800: "Extra Bold",
  900: "Black",
};

export function weightName(w: number): string {
  return WEIGHT_NAMES[w] ?? WEIGHT_NAMES[Math.round(w / 100) * 100] ?? String(w);
}
