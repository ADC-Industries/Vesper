// A screen's colour correction and corner pin (crates/protocol/README.md, "Screen
// configuration"): the settings with their ranges, defaults filled in, the corner pin's quad,
// dragging a corner, and a convexity check that mirrors the server's.

import type { ColorCorrection, CornerOffset, CornerPin, Screen } from "$protocol";

export type ColorKey = keyof ColorCorrection;

export interface ColorField {
  key: ColorKey;
  label: string;
  min: number;
  max: number;
  step: number;
  def: number;
}

/** The colour settings in the order the tab shows them (ranges as the server checks them). */
export const COLOR_FIELDS: ColorField[] = [
  { key: "brightness", label: "Brightness", min: -1, max: 1, step: 0.01, def: 0 },
  { key: "contrast", label: "Contrast", min: 0, max: 2, step: 0.01, def: 1 },
  { key: "gamma", label: "Gamma", min: 0.1, max: 4, step: 0.01, def: 1 },
  { key: "saturation", label: "Saturation", min: 0, max: 2, step: 0.01, def: 1 },
  { key: "red", label: "Red", min: 0, max: 2, step: 0.01, def: 1 },
  { key: "green", label: "Green", min: 0, max: 2, step: 0.01, def: 1 },
  { key: "blue", label: "Blue", min: 0, max: 2, step: 0.01, def: 1 },
  { key: "blackLevel", label: "Black level", min: -0.5, max: 0.5, step: 0.01, def: 0 },
];

export const IDENTITY_COLOR: ColorCorrection = Object.fromEntries(COLOR_FIELDS.map((f) => [f.key, f.def])) as unknown as ColorCorrection;

/** The screen's colour correction with every default filled in. */
export function colorOf(s: Pick<Screen, "color"> | null | undefined): ColorCorrection {
  return { ...IDENTITY_COLOR, ...(s?.color ?? {}) };
}

export function isIdentityColor(c: ColorCorrection): boolean {
  return COLOR_FIELDS.every((f) => c[f.key] === f.def);
}

/** `v` clamped to the field's range and rounded to its step (NaN → the default). */
export function clampField(f: ColorField, v: number): number {
  if (!Number.isFinite(v)) return f.def;
  const r = Math.round(v / f.step) * f.step;
  return Math.min(f.max, Math.max(f.min, Number(r.toFixed(4))));
}

/** A number field's text as a number: NaN for an empty (being retyped) or invalid field, which
 * `Number("")` would read as 0 (contrast 0 flattens the screen live). */
export function fieldNumber(raw: string): number {
  return raw.trim() === "" ? NaN : Number(raw);
}

export type Corner = "topLeft" | "topRight" | "bottomRight" | "bottomLeft";
export const CORNERS: { key: Corner; label: string }[] = [
  { key: "topLeft", label: "Top left" },
  { key: "topRight", label: "Top right" },
  { key: "bottomLeft", label: "Bottom left" },
  { key: "bottomRight", label: "Bottom right" },
];

const zero = (): CornerOffset => ({ x: 0, y: 0 });

/** The screen's corner pin with every default filled in. */
export function pinOf(s: Pick<Screen, "cornerPin"> | null | undefined): CornerPin {
  const p = s?.cornerPin;
  return {
    enabled: p?.enabled ?? false,
    topLeft: { ...zero(), ...(p?.topLeft ?? {}) },
    topRight: { ...zero(), ...(p?.topRight ?? {}) },
    bottomRight: { ...zero(), ...(p?.bottomRight ?? {}) },
    bottomLeft: { ...zero(), ...(p?.bottomLeft ?? {}) },
  };
}

/** Horizontal / vertical direction that points away from the centre, per corner. */
const OUT: Record<Corner, [number, number]> = {
  topLeft: [-1, -1],
  topRight: [1, -1],
  bottomRight: [1, 1],
  bottomLeft: [-1, 1],
};

/** Where a corner of a `w` × `h` screen lands (px, x right, y down). */
export function cornerPoint(p: CornerPin, c: Corner, w: number, h: number): [number, number] {
  const [ox, oy] = OUT[c];
  const base: [number, number] = [ox < 0 ? 0 : w, oy < 0 ? 0 : h];
  return [base[0] + ox * p[c].x, base[1] + oy * p[c].y];
}

export type Point = [number, number];
export type Quad = [Point, Point, Point, Point];

/** The quad clockwise from the top left. */
export function quadOf(p: CornerPin, w: number, h: number): Quad {
  return [cornerPoint(p, "topLeft", w, h), cornerPoint(p, "topRight", w, h), cornerPoint(p, "bottomRight", w, h), cornerPoint(p, "bottomLeft", w, h)];
}

/** The offset that puts corner `c` at the point (x, y) (whole pixels). */
export function offsetFor(c: Corner, x: number, y: number, w: number, h: number): CornerOffset {
  const [ox, oy] = OUT[c];
  const bx = ox < 0 ? 0 : w;
  const by = oy < 0 ? 0 : h;
  return { x: Math.round((x - bx) * ox), y: Math.round((y - by) * oy) };
}

/** The corners make a convex, clockwise quad (what the server accepts while enabled). */
export function isConvex(q: Quad): boolean {
  for (let i = 0; i < 4; i++) {
    const [a, b, c] = [q[i]!, q[(i + 1) % 4]!, q[(i + 2) % 4]!];
    const cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
    if (cross <= 0) return false;
  }
  return true;
}

/** Calls `fn` at most every `ms` with the latest arguments (the last call always runs). */
export function throttle<A extends unknown[]>(fn: (...a: A) => void, ms: number): ((...a: A) => void) & { flush: () => void } {
  let last = 0;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: A | null = null;
  const run = () => {
    timer = null;
    last = Date.now();
    const a = pending;
    pending = null;
    if (a) fn(...a);
  };
  const t = (...a: A) => {
    pending = a;
    const wait = ms - (Date.now() - last);
    if (wait <= 0 && !timer) run();
    else if (!timer) timer = setTimeout(run, Math.max(0, wait));
  };
  t.flush = () => {
    if (timer) clearTimeout(timer);
    run();
  };
  return t;
}
