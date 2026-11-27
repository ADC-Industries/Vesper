// Canvas maths for the Edit view: the view transform (zoom and pan), rulers, hit testing,
// moving and resizing with snapping, alignment and distribution. Everything here works in
// model space (canvas px, e.g. 1920 × 1080) unless a name says "screen". Pure functions, so
// they are tested without a browser.

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Size {
  width: number;
  height: number;
}

/** Screen = model × zoom + pan (screen px relative to the viewport's top-left corner). */
export interface View {
  zoom: number;
  panX: number;
  panY: number;
}

export const ZOOM_MIN = 0.25;
export const ZOOM_MAX = 4;
/** Zoom steps offered by the zoom menu and used by Ctrl+wheel / Ctrl+plus. */
export const ZOOM_STEPS = [0.25, 0.33, 0.5, 0.67, 0.75, 1, 1.25, 1.5, 2, 3, 4];

export function clampZoom(z: number): number {
  return Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, z));
}

/** Zoom that fits the whole canvas inside the viewport with `margin` px all round. */
export function fitZoom(canvas: Size, viewport: Size, margin = 40): number {
  const w = Math.max(1, viewport.width - 2 * margin);
  const h = Math.max(1, viewport.height - 2 * margin);
  const z = Math.min(w / Math.max(1, canvas.width), h / Math.max(1, canvas.height));
  // Fit may go below the manual minimum on a small window; never above the maximum.
  return Math.max(0.05, Math.min(ZOOM_MAX, z));
}

/** A view showing the canvas centred at `zoom`. */
export function centredView(canvas: Size, viewport: Size, zoom: number): View {
  return {
    zoom,
    panX: (viewport.width - canvas.width * zoom) / 2,
    panY: (viewport.height - canvas.height * zoom) / 2,
  };
}

export function fitView(canvas: Size, viewport: Size, margin = 40): View {
  return centredView(canvas, viewport, fitZoom(canvas, viewport, margin));
}

/** Fit to width: the canvas spans the viewport's width (minus margins), top-aligned. */
export function fitWidthView(canvas: Size, viewport: Size, margin = 20): View {
  const zoom = Math.max(0.05, Math.min(ZOOM_MAX, (viewport.width - 2 * margin) / Math.max(1, canvas.width)));
  const v = centredView(canvas, viewport, zoom);
  return canvas.height * zoom + 2 * margin > viewport.height ? { ...v, panY: margin } : v;
}

export function toModel(v: View, sx: number, sy: number): { x: number; y: number } {
  return { x: (sx - v.panX) / v.zoom, y: (sy - v.panY) / v.zoom };
}

export function toScreen(v: View, mx: number, my: number): { x: number; y: number } {
  return { x: mx * v.zoom + v.panX, y: my * v.zoom + v.panY };
}

export function rectToScreen(v: View, r: Rect): Rect {
  return { x: r.x * v.zoom + v.panX, y: r.y * v.zoom + v.panY, width: r.width * v.zoom, height: r.height * v.zoom };
}

/** Changes the zoom keeping the model point under screen point (sx, sy) where it is. */
export function zoomAt(v: View, zoom: number, sx: number, sy: number): View {
  const z = clampZoom(zoom);
  const m = toModel(v, sx, sy);
  return { zoom: z, panX: sx - m.x * z, panY: sy - m.y * z };
}

/** The next zoom step up (dir 1) or down (dir -1) from `z`. */
export function stepZoom(z: number, dir: 1 | -1): number {
  if (dir > 0) return ZOOM_STEPS.find((s) => s > z + 1e-6) ?? ZOOM_MAX;
  return [...ZOOM_STEPS].reverse().find((s) => s < z - 1e-6) ?? ZOOM_MIN;
}

// ---- rulers ------------------------------------------------------------------------------------

export interface Tick {
  /** Screen px along the ruler. */
  pos: number;
  /** Model value (canvas px). */
  value: number;
  major: boolean;
}

const RULER_STEPS = [1, 2, 5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 5000];

/**
 * Ticks for a ruler `length` screen px long whose model origin sits at screen `pan`. Major
 * ticks (labelled) are at least `minLabelPx` apart; minor ticks divide them in ten or five,
 * at least 5 px apart.
 */
export function rulerTicks(zoom: number, pan: number, length: number, minLabelPx = 60): Tick[] {
  const major = RULER_STEPS.find((s) => s * zoom >= minLabelPx) ?? RULER_STEPS[RULER_STEPS.length - 1]!;
  let minor = major / 10;
  if (minor * zoom < 5) minor = major / 5;
  if (minor * zoom < 5) minor = major / 2;
  if (minor * zoom < 5) minor = major;
  const from = Math.floor(-pan / zoom / minor) * minor;
  const to = (length - pan) / zoom;
  const out: Tick[] = [];
  for (let v = from, i = 0; v <= to && i < 5000; v += minor, i++) {
    const value = Math.round(v * 1000) / 1000;
    const isMajor = Math.abs(value / major - Math.round(value / major)) < 1e-6;
    out.push({ pos: value * zoom + pan, value, major: isMajor });
  }
  return out;
}

/** Ruler label: "1,100", "-200". */
export function rulerLabel(v: number): string {
  const n = Math.round(v);
  const s = Math.abs(n)
    .toString()
    .replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  return n < 0 ? `-${s}` : s;
}

// ---- rects ---------------------------------------------------------------------------------------

export function union(rs: Rect[]): Rect | null {
  if (!rs.length) return null;
  let x0 = Infinity,
    y0 = Infinity,
    x1 = -Infinity,
    y1 = -Infinity;
  for (const r of rs) {
    x0 = Math.min(x0, r.x);
    y0 = Math.min(y0, r.y);
    x1 = Math.max(x1, r.x + r.width);
    y1 = Math.max(y1, r.y + r.height);
  }
  return { x: x0, y: y0, width: x1 - x0, height: y1 - y0 };
}

export function intersects(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/** A rect from two corner points (any order). */
export function rectFromPoints(ax: number, ay: number, bx: number, by: number): Rect {
  return { x: Math.min(ax, bx), y: Math.min(ay, by), width: Math.abs(bx - ax), height: Math.abs(by - ay) };
}

/** Axis-aligned bounds of `r` rotated by `deg` (clockwise) about its centre. */
export function rotatedBounds(r: Rect, deg: number): Rect {
  if (!deg || !(deg % 360)) return r;
  const t = (deg * Math.PI) / 180;
  const c = Math.abs(Math.cos(t)),
    s = Math.abs(Math.sin(t));
  const w = r.width * c + r.height * s;
  const h = r.width * s + r.height * c;
  const cx = r.x + r.width / 2,
    cy = r.y + r.height / 2;
  return { x: cx - w / 2, y: cy - h / 2, width: w, height: h };
}

/** True when model point (px, py) is inside `r` rotated by `deg` about its centre. */
export function containsPoint(r: Rect, deg: number, px: number, py: number): boolean {
  const cx = r.x + r.width / 2,
    cy = r.y + r.height / 2;
  const t = (-(deg || 0) * Math.PI) / 180;
  const dx = px - cx,
    dy = py - cy;
  const lx = dx * Math.cos(t) - dy * Math.sin(t);
  const ly = dx * Math.sin(t) + dy * Math.cos(t);
  return Math.abs(lx) <= r.width / 2 && Math.abs(ly) <= r.height / 2;
}

/** Rounds geometry to 1/100 px (what the inspector shows and the ops send). */
export function roundRect(r: Rect): Rect {
  const q = (v: number) => Math.round(v * 100) / 100;
  return { x: q(r.x), y: q(r.y), width: q(r.width), height: q(r.height) };
}

// ---- snapping ------------------------------------------------------------------------------------

export interface SnapTargets {
  /** Vertical lines (x positions). */
  x: number[];
  /** Horizontal lines (y positions). */
  y: number[];
}

/** A guide line drawn while snapping. */
export interface Guide {
  axis: "x" | "y";
  /** Model coordinate of the line. */
  pos: number;
}

export interface SnapOptions {
  /** Snap distance, model px (screen px / zoom). */
  threshold: number;
  /** Grid spacing in model px, or 0 for none. */
  grid: number;
}

/** Canvas edges and centre plus the edges and centres of `others`. */
export function snapTargets(canvas: Size, others: Rect[]): SnapTargets {
  const x = [0, canvas.width / 2, canvas.width];
  const y = [0, canvas.height / 2, canvas.height];
  for (const r of others) {
    x.push(r.x, r.x + r.width / 2, r.x + r.width);
    y.push(r.y, r.y + r.height / 2, r.y + r.height);
  }
  return { x, y };
}

/**
 * The best snap of any of `candidates` (positions of the moving thing on one axis) to
 * `targets`: the offset to add, and the target lines hit, or null when none is within the
 * threshold. Grid lines count as targets too, but without a guide.
 */
export function snapAxis(candidates: number[], targets: number[], opt: SnapOptions): { delta: number; lines: number[]; grid: boolean } | null {
  let best: { delta: number; lines: number[]; grid: boolean } | null = null;
  for (const c of candidates) {
    for (const t of targets) {
      const d = t - c;
      if (Math.abs(d) > opt.threshold) continue;
      if (!best || Math.abs(d) < Math.abs(best.delta) - 1e-9) best = { delta: d, lines: [t], grid: false };
      else if (Math.abs(d - best.delta) < 1e-9 && !best.lines.includes(t)) best.lines.push(t);
    }
  }
  if (best) {
    // Every line the snapped candidates now touch gets a guide.
    const lines = new Set<number>();
    for (const c of candidates) for (const t of targets) if (Math.abs(c + best.delta - t) < 1e-6) lines.add(t);
    best.lines = [...lines];
    return best;
  }
  if (opt.grid > 0) {
    for (const c of candidates) {
      const g = Math.round(c / opt.grid) * opt.grid;
      const d = g - c;
      if (Math.abs(d) <= opt.threshold && (!best || Math.abs(d) < Math.abs(best.delta))) best = { delta: d, lines: [], grid: true };
    }
  }
  return best;
}

/** Snaps a moving rect (its edges and centre); returns the snapped rect and the guides. */
export function snapMove(r: Rect, targets: SnapTargets, opt: SnapOptions): { rect: Rect; guides: Guide[] } {
  const sx = snapAxis([r.x, r.x + r.width / 2, r.x + r.width], targets.x, opt);
  const sy = snapAxis([r.y, r.y + r.height / 2, r.y + r.height], targets.y, opt);
  const guides: Guide[] = [];
  for (const p of sx?.lines ?? []) guides.push({ axis: "x", pos: p });
  for (const p of sy?.lines ?? []) guides.push({ axis: "y", pos: p });
  return { rect: { ...r, x: r.x + (sx?.delta ?? 0), y: r.y + (sy?.delta ?? 0) }, guides };
}

/** Moves by (dx, dy); Shift keeps only the larger axis. */
export function constrainAxis(dx: number, dy: number): { dx: number; dy: number } {
  return Math.abs(dx) >= Math.abs(dy) ? { dx, dy: 0 } : { dx: 0, dy };
}

// ---- resizing --------------------------------------------------------------------------------------

export type Handle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";
export const HANDLES: Handle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];

/** Unit position (0..1, 0..1) of a handle on its rect. */
export function handleUnit(h: Handle): { u: number; v: number } {
  const u = h.includes("w") ? 0 : h.includes("e") ? 1 : 0.5;
  const v = h.includes("n") ? 0 : h.includes("s") ? 1 : 0.5;
  return { u, v };
}

export interface ResizeOptions {
  /** Keep the aspect ratio of `start` (Shift, or the element's aspect lock). */
  keepAspect: boolean;
  /** Resize about the centre (Alt). */
  fromCenter: boolean;
  /** Rotation of the element, degrees clockwise (the handle drags in its own frame). */
  rotation?: number;
  /** Smallest width / height. */
  min?: number;
  /** Snapping (only for unrotated elements). */
  snap?: { targets: SnapTargets; opt: SnapOptions } | null;
}

/**
 * Resizes `start` by dragging handle `h` by (dx, dy) model px. The opposite handle (or the
 * centre with `fromCenter`) stays where it is on screen, also for rotated elements.
 */
export function resizeRect(start: Rect, h: Handle, dx: number, dy: number, o: ResizeOptions): { rect: Rect; guides: Guide[] } {
  const rot = ((o.rotation ?? 0) * Math.PI) / 180;
  const cos = Math.cos(rot),
    sin = Math.sin(rot);
  // The drag in the element's own (unrotated) frame.
  let lx = dx * cos + dy * sin;
  let ly = -dx * sin + dy * cos;
  const { u, v } = handleUnit(h);
  const sxDir = u === 1 ? 1 : u === 0 ? -1 : 0;
  const syDir = v === 1 ? 1 : v === 0 ? -1 : 0;
  const k = o.fromCenter ? 2 : 1;
  const guides: Guide[] = [];

  // Snap the moving edges (unrotated only): adjust the drag so the edge lands on a target.
  if (o.snap && !o.rotation) {
    if (sxDir) {
      const edge = sxDir > 0 ? start.x + start.width + lx : start.x + lx;
      const s = snapAxis([edge], o.snap.targets.x, o.snap.opt);
      if (s) {
        lx += s.delta;
        for (const p of s.lines) guides.push({ axis: "x", pos: p });
      }
    }
    if (syDir) {
      const edge = syDir > 0 ? start.y + start.height + ly : start.y + ly;
      const s = snapAxis([edge], o.snap.targets.y, o.snap.opt);
      if (s) {
        ly += s.delta;
        for (const p of s.lines) guides.push({ axis: "y", pos: p });
      }
    }
  }

  const min = o.min ?? 1;
  let w = start.width + sxDir * lx * k;
  let hgt = start.height + syDir * ly * k;
  if (o.keepAspect && start.width > 0 && start.height > 0) {
    const ratio = start.width / start.height;
    if (sxDir && syDir) {
      // Corner: follow whichever side changed more (relatively).
      if (Math.abs(w / start.width - 1) >= Math.abs(hgt / start.height - 1)) hgt = w / ratio;
      else w = hgt * ratio;
    } else if (sxDir) hgt = w / ratio;
    else w = hgt * ratio;
    // Aspect wins over snapping on the other axis: keep only guides that still touch.
    for (let i = guides.length - 1; i >= 0; i--) if ((sxDir && syDir) || (sxDir ? guides[i]!.axis === "y" : guides[i]!.axis === "x")) guides.splice(i, 1);
  }
  if (w < min) w = min;
  if (hgt < min) hgt = min;

  // The anchor: the opposite handle, or the centre.
  const au = o.fromCenter ? 0.5 : 1 - u;
  const av = o.fromCenter ? 0.5 : 1 - v;
  const world = (r: { cx: number; cy: number; w: number; h: number }, pu: number, pv: number) => {
    const ox = (pu - 0.5) * r.w,
      oy = (pv - 0.5) * r.h;
    return { x: r.cx + ox * cos - oy * sin, y: r.cy + ox * sin + oy * cos };
  };
  const s0 = { cx: start.x + start.width / 2, cy: start.y + start.height / 2, w: start.width, h: start.height };
  const anchor = world(s0, au, av);
  const ox = (au - 0.5) * w,
    oy = (av - 0.5) * hgt;
  const cx = anchor.x - (ox * cos - oy * sin);
  const cy = anchor.y - (ox * sin + oy * cos);
  return { rect: { x: cx - w / 2, y: cy - hgt / 2, width: w, height: hgt }, guides };
}

/** Scales `r` (a member of a group) from group bounds `from` to `to`. */
export function scaleInto(r: Rect, from: Rect, to: Rect): Rect {
  const sx = from.width ? to.width / from.width : 1;
  const sy = from.height ? to.height / from.height : 1;
  return { x: to.x + (r.x - from.x) * sx, y: to.y + (r.y - from.y) * sy, width: r.width * sx, height: r.height * sy };
}

// ---- align and distribute ----------------------------------------------------------------------------

export type AlignMode = "left" | "centre" | "right" | "top" | "middle" | "bottom";
export type DistributeMode = "horizontal" | "vertical";

export interface Placed {
  id: string;
  rect: Rect;
}

/** Aligns rects to the edges / centre of their common bounds (or of `to`). Returns moved rects. */
export function align(items: Placed[], mode: AlignMode, to?: Rect): Placed[] {
  const b = to ?? union(items.map((i) => i.rect));
  if (!b) return [];
  return items.map(({ id, rect: r }) => {
    let { x, y } = r;
    switch (mode) {
      case "left":
        x = b.x;
        break;
      case "centre":
        x = b.x + (b.width - r.width) / 2;
        break;
      case "right":
        x = b.x + b.width - r.width;
        break;
      case "top":
        y = b.y;
        break;
      case "middle":
        y = b.y + (b.height - r.height) / 2;
        break;
      case "bottom":
        y = b.y + b.height - r.height;
        break;
    }
    return { id, rect: { ...r, x, y } };
  });
}

/**
 * Spaces rects evenly between the first and the last (by position) so the gaps between them
 * are equal. Needs three or more; fewer come back unchanged.
 */
export function distribute(items: Placed[], mode: DistributeMode): Placed[] {
  if (items.length < 3) return items.map((i) => ({ ...i }));
  const hor = mode === "horizontal";
  const pos = (r: Rect) => (hor ? r.x : r.y);
  const len = (r: Rect) => (hor ? r.width : r.height);
  const sorted = [...items].sort((a, b) => pos(a.rect) + len(a.rect) / 2 - (pos(b.rect) + len(b.rect) / 2));
  const first = sorted[0]!.rect;
  const last = sorted[sorted.length - 1]!.rect;
  const span = pos(last) + len(last) - pos(first);
  const total = sorted.reduce((s, i) => s + len(i.rect), 0);
  const gap = (span - total) / (sorted.length - 1);
  let at = pos(first);
  const out: Placed[] = [];
  for (const it of sorted) {
    const r = hor ? { ...it.rect, x: at } : { ...it.rect, y: at };
    out.push({ id: it.id, rect: r });
    at += len(it.rect) + gap;
  }
  return out;
}

/** Arrow-key nudge: 1 px, 10 px with Shift. */
export function nudge(key: string, shift: boolean): { dx: number; dy: number } | null {
  const d = shift ? 10 : 1;
  switch (key) {
    case "ArrowLeft":
      return { dx: -d, dy: 0 };
    case "ArrowRight":
      return { dx: d, dy: 0 };
    case "ArrowUp":
      return { dx: 0, dy: -d };
    case "ArrowDown":
      return { dx: 0, dy: d };
  }
  return null;
}
