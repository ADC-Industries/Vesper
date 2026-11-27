// The mask editor's pure logic (docs/LOOKS-MASKS-PLAN.md §6): a mask is a slide of shapes
// drawn over a screen (overlay) or the only place the screen shows through (keyhole). Shapes
// are ordinary slide elements: rectangles, ellipses and regular polygons with a colour fill.
import type { Color, Element, Mask, Rect, Shape } from "$protocol";

export type MaskShape = "rectangle" | "ellipse" | "polygon";

export const SHAPES: { id: MaskShape; label: string }[] = [
  { id: "rectangle", label: "Rectangle" },
  { id: "ellipse", label: "Ellipse" },
  { id: "polygon", label: "Polygon" },
];

const newId = () =>
  typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : "xxxxxxxx-xxxx-4xxx-8xxx-xxxxxxxxxxxx".replace(/x/g, () => Math.floor(Math.random() * 16).toString(16));

/** The canvas size of a mask (1920 × 1080 when not stored). */
export function canvasOf(m: Mask): { width: number; height: number } {
  return { width: m.size?.width ?? 1920, height: m.size?.height ?? 1080 };
}

/** A new shape element, centred, half the canvas wide (`polygon`: a hexagon). */
export function newShape(m: Mask, shape: MaskShape, n: number): Element {
  const { width, height } = canvasOf(m);
  const w = Math.round(width / 2);
  const h = Math.round(height / 2);
  const s: Shape = shape === "polygon" ? { type: "polygon", sides: 6 } : { type: shape };
  return {
    id: newId(),
    name: `${SHAPES.find((x) => x.id === shape)?.label ?? "Shape"} ${n}`,
    bounds: { x: Math.round((width - w) / 2), y: Math.round((height - h) / 2), width: w, height: h },
    kind: { type: "shape", shape: s },
    fill: { enabled: true, kind: { type: "color", color: { r: 0, g: 0, b: 0 } } },
  };
}

/** The shape kind of an element (null: not one the editor draws). */
export function shapeOf(e: Element): MaskShape | null {
  const k = e.kind;
  const t = k && k.type !== "unknown" ? k.shape?.type : undefined;
  if (t === "rectangle" || t === "ellipse" || t === "polygon") return t;
  return null;
}

/** The fill colour of an element (black when it has none). */
export function fillOf(e: Element): Color {
  const k = e.fill?.kind;
  return k?.type === "color" ? k.color : { r: 0, g: 0, b: 0 };
}

/** `m` with its slide's elements replaced by `f(elements)`. Never mutates `m`. */
export function withElements(m: Mask, f: (els: Element[]) => Element[]): Mask {
  return { ...m, slide: { ...m.slide, elements: f(m.slide.elements ?? []) } };
}

/** `m` with element `id` changed by `f`. */
export function withElement(m: Mask, id: string, f: (e: Element) => Element): Mask {
  return withElements(m, (els) => els.map((e) => (e.id === id ? f(e) : e)));
}

/** An element's bounds with one field set (sizes at least 1 px). */
export function setBound(e: Element, key: keyof Rect, v: number): Element {
  const b = { x: 0, y: 0, width: 0, height: 0, ...(e.bounds ?? {}) };
  const val = key === "width" || key === "height" ? Math.max(1, v) : v;
  return { ...e, bounds: { ...b, [key]: val } };
}

/** The SVG points of a regular polygon of `sides` in a box (first point at the top). */
export function polygonPoints(b: Rect, sides: number): string {
  const n = Math.max(3, Math.round(sides));
  const cx = (b.x ?? 0) + (b.width ?? 0) / 2;
  const cy = (b.y ?? 0) + (b.height ?? 0) / 2;
  const rx = (b.width ?? 0) / 2;
  const ry = (b.height ?? 0) / 2;
  return Array.from({ length: n }, (_, i) => {
    const t = -Math.PI / 2 + (2 * Math.PI * i) / n;
    return `${(cx + rx * Math.cos(t)).toFixed(1)},${(cy + ry * Math.sin(t)).toFixed(1)}`;
  }).join(" ");
}

/** A free "Mask N" name. */
export function freeMaskName(masks: { name?: string }[]): string {
  const taken = new Set(masks.map((m) => (m.name ?? "").trim().toLowerCase()));
  for (let n = 1; ; n++) if (!taken.has(`mask ${n}`)) return `Mask ${n}`;
}

/** Who uses a mask, from a refused delete's `current` (`{ looks, screens }`). */
export interface MaskUsers {
  looks: number;
  /** Stage screens' names (or "a stage screen" when unknown). */
  stages: string[];
}

/**
 * The users in a refused mask delete's `current`: looks and stage screens both count (D08-5).
 * `null` when nothing uses it (the refusal was about something else, e.g. a stale revision).
 */
export function maskUsers(current: unknown, screenName: (id: string) => string | undefined): MaskUsers | null {
  const c = (current ?? {}) as { looks?: unknown; screens?: unknown };
  const looks = Array.isArray(c.looks) ? c.looks.length : 0;
  const stages = Array.isArray(c.screens) ? c.screens.map((id) => screenName(String(id)) || "a stage screen") : [];
  return looks || stages.length ? { looks, stages } : null;
}

/** The delete prompt for a mask in use: which looks and stage screens lose it. */
export function maskUsersText(u: MaskUsers): string {
  const parts: string[] = [];
  if (u.looks) parts.push(u.looks === 1 ? "a look" : `${u.looks} looks`);
  if (u.stages.length === 1) parts.push(`the stage screen “${u.stages[0]}”`);
  else if (u.stages.length) parts.push(`${u.stages.length} stage screens (${u.stages.map((s) => `“${s}”`).join(", ")})`);
  const who = parts.join(" and ");
  const plural = u.looks + u.stages.length > 1;
  return `${who.charAt(0).toUpperCase()}${who.slice(1)} ${plural ? "use" : "uses"} this mask. Delete it anyway? ${plural ? "They" : "It"} will show no mask from then on.`;
}
