import { describe, expect, it } from "vitest";
import {
  align,
  centredView,
  constrainAxis,
  containsPoint,
  distribute,
  fitView,
  fitWidthView,
  fitZoom,
  nudge,
  resizeRect,
  rotatedBounds,
  rulerLabel,
  rulerTicks,
  snapAxis,
  snapMove,
  snapTargets,
  stepZoom,
  toModel,
  toScreen,
  zoomAt,
} from "./geometry";

const canvas = { width: 1920, height: 1080 };
const opt = { threshold: 6, grid: 0 };

describe("view transform", () => {
  it("fits the canvas with margins, centred", () => {
    const z = fitZoom(canvas, { width: 1000, height: 1000 }, 40);
    expect(z).toBeCloseTo(920 / 1920);
    const v = fitView(canvas, { width: 1000, height: 1000 });
    expect(v.panX).toBeCloseTo((1000 - 1920 * z) / 2);
    expect(v.panY).toBeCloseTo((1000 - 1080 * z) / 2);
  });
  it("maps screen and model both ways", () => {
    const v = { zoom: 0.5, panX: 100, panY: 50 };
    expect(toScreen(v, 960, 540)).toEqual({ x: 580, y: 320 });
    expect(toModel(v, 580, 320)).toEqual({ x: 960, y: 540 });
  });
  it("zooms about the pointer", () => {
    const v = centredView(canvas, { width: 1200, height: 800 }, 0.5);
    const before = toModel(v, 300, 200);
    const z = zoomAt(v, 2, 300, 200);
    expect(z.zoom).toBe(2);
    const after = toModel(z, 300, 200);
    expect(after.x).toBeCloseTo(before.x);
    expect(after.y).toBeCloseTo(before.y);
  });
  it("clamps manual zoom to 25..400 %", () => {
    expect(zoomAt({ zoom: 1, panX: 0, panY: 0 }, 10, 0, 0).zoom).toBe(4);
    expect(zoomAt({ zoom: 1, panX: 0, panY: 0 }, 0.01, 0, 0).zoom).toBe(0.25);
    expect(stepZoom(1, 1)).toBe(1.25);
    expect(stepZoom(1, -1)).toBe(0.75);
    expect(stepZoom(4, 1)).toBe(4);
  });
  it("fit to width spans the width", () => {
    const v = fitWidthView(canvas, { width: 1000, height: 300 }, 20);
    expect(v.zoom * 1920).toBeCloseTo(960);
    expect(v.panX).toBeCloseTo(20);
    expect(v.panY).toBe(20);
  });
});

describe("rulers", () => {
  it("labels majors at least 60 px apart, in canvas px", () => {
    const t = rulerTicks(0.5, 100, 1000);
    const majors = t.filter((x) => x.major);
    expect(majors[1]!.pos - majors[0]!.pos).toBeGreaterThanOrEqual(60);
    // The canvas origin sits at the pan offset.
    expect(t.find((x) => x.value === 0)?.pos).toBe(100);
    expect(majors.every((m) => m.value % 200 === 0)).toBe(true);
    // Negative values left of the canvas.
    expect(t[0]!.value).toBeLessThan(0);
  });
  it("formats like the reference", () => {
    expect(rulerLabel(1100)).toBe("1,100");
    expect(rulerLabel(-200)).toBe("-200");
    expect(rulerLabel(0)).toBe("0");
  });
});

describe("snapping", () => {
  const targets = snapTargets(canvas, [{ x: 100, y: 100, width: 200, height: 100 }]);
  it("snaps an element's centre to the canvas centre with guides", () => {
    const r = { x: 958 - 200, y: 543 - 100, width: 400, height: 200 };
    const s = snapMove(r, targets, opt);
    expect(s.rect.x + s.rect.width / 2).toBe(960);
    expect(s.rect.y + s.rect.height / 2).toBe(540);
    expect(s.guides).toContainEqual({ axis: "x", pos: 960 });
    expect(s.guides).toContainEqual({ axis: "y", pos: 540 });
  });
  it("snaps edges to other elements' edges", () => {
    const s = snapMove({ x: 303, y: 400, width: 50, height: 50 }, targets, opt);
    expect(s.rect.x).toBe(300);
    expect(s.guides).toEqual([{ axis: "x", pos: 300 }]);
  });
  it("does not snap beyond the threshold", () => {
    const s = snapMove({ x: 320, y: 400, width: 50, height: 50 }, targets, opt);
    expect(s.rect).toEqual({ x: 320, y: 400, width: 50, height: 50 });
    expect(s.guides).toEqual([]);
  });
  it("falls back to the grid, without guides", () => {
    const s = snapAxis([447], [0, 960, 1920], { threshold: 6, grid: 50 });
    expect(s).toEqual({ delta: 3, lines: [], grid: true });
  });
  it("Shift constrains to the larger axis", () => {
    expect(constrainAxis(10, 3)).toEqual({ dx: 10, dy: 0 });
    expect(constrainAxis(-2, -9)).toEqual({ dx: 0, dy: -9 });
  });
  it("nudges 1 px, 10 px with Shift", () => {
    expect(nudge("ArrowLeft", false)).toEqual({ dx: -1, dy: 0 });
    expect(nudge("ArrowDown", true)).toEqual({ dx: 0, dy: 10 });
    expect(nudge("a", false)).toBeNull();
  });
});

describe("resizing", () => {
  const start = { x: 100, y: 100, width: 200, height: 100 };
  const base = { keepAspect: false, fromCenter: false };
  it("drags the south-east corner", () => {
    expect(resizeRect(start, "se", 50, 20, base).rect).toEqual({ x: 100, y: 100, width: 250, height: 120 });
  });
  it("drags the west edge, keeping the east edge", () => {
    expect(resizeRect(start, "w", -30, 99, base).rect).toEqual({ x: 70, y: 100, width: 230, height: 100 });
  });
  it("keeps the aspect ratio with Shift", () => {
    const r = resizeRect(start, "se", 100, 0, { ...base, keepAspect: true }).rect;
    expect(r.width / r.height).toBeCloseTo(2);
    expect(r.width).toBe(300);
    expect(r.x).toBe(100);
  });
  it("resizes about the centre with Alt", () => {
    const r = resizeRect(start, "e", 10, 0, { ...base, fromCenter: true }).rect;
    expect(r).toEqual({ x: 90, y: 100, width: 220, height: 100 });
  });
  it("snaps the moving edge to a target", () => {
    const targets = snapTargets(canvas, []);
    const r = resizeRect(start, "e", 655, 0, { ...base, snap: { targets, opt } });
    expect(r.rect.x + r.rect.width).toBe(960);
    expect(r.guides).toEqual([{ axis: "x", pos: 960 }]);
  });
  it("keeps the opposite corner fixed for rotated elements", () => {
    const rot = 90;
    const r = resizeRect(start, "se", 0, 40, { ...base, rotation: rot }).rect;
    // Rotated 90° clockwise, local +x points down: the drag widens the box.
    expect(r.width).toBeCloseTo(240);
    expect(r.height).toBeCloseTo(100);
    // The NW handle (anchor) stays put in world space.
    const world = (q: typeof r, u: number, v: number) => {
      const t = (rot * Math.PI) / 180;
      const ox = (u - 0.5) * q.width,
        oy = (v - 0.5) * q.height;
      return { x: q.x + q.width / 2 + ox * Math.cos(t) - oy * Math.sin(t), y: q.y + q.height / 2 + ox * Math.sin(t) + oy * Math.cos(t) };
    };
    expect(world(r, 0, 0).x).toBeCloseTo(world(start, 0, 0).x);
    expect(world(r, 0, 0).y).toBeCloseTo(world(start, 0, 0).y);
  });
  it("never goes below the minimum size", () => {
    expect(resizeRect(start, "e", -500, 0, base).rect.width).toBe(1);
  });
});

describe("hit testing", () => {
  it("handles rotation", () => {
    const r = { x: 0, y: 0, width: 200, height: 20 };
    expect(containsPoint(r, 0, 190, 10)).toBe(true);
    expect(containsPoint(r, 90, 190, 10)).toBe(false);
    expect(containsPoint(r, 90, 100, 90)).toBe(true);
    const b = rotatedBounds(r, 90);
    expect(b.width).toBeCloseTo(20);
    expect(b.height).toBeCloseTo(200);
  });
});

describe("align and distribute", () => {
  const items = [
    { id: "a", rect: { x: 0, y: 0, width: 100, height: 50 } },
    { id: "b", rect: { x: 300, y: 100, width: 50, height: 50 } },
    { id: "c", rect: { x: 900, y: 40, width: 100, height: 20 } },
  ];
  it("aligns to the selection bounds", () => {
    expect(align(items, "right").map((i) => i.rect.x + i.rect.width)).toEqual([1000, 1000, 1000]);
    expect(align(items, "centre").map((i) => i.rect.x + i.rect.width / 2)).toEqual([500, 500, 500]);
    expect(align(items, "bottom").map((i) => i.rect.y + i.rect.height)).toEqual([150, 150, 150]);
    expect(align(items, "middle").map((i) => i.rect.y + i.rect.height / 2)).toEqual([75, 75, 75]);
  });
  it("distributes with equal gaps, ends fixed", () => {
    const d = distribute(items, "horizontal");
    const byId = Object.fromEntries(d.map((i) => [i.id, i.rect]));
    expect(byId.a!.x).toBe(0);
    expect(byId.c!.x).toBe(900);
    const gap1 = byId.b!.x - (byId.a!.x + 100);
    const gap2 = byId.c!.x - (byId.b!.x + 50);
    expect(gap1).toBeCloseTo(gap2);
    expect(distribute(items.slice(0, 2), "vertical")).toEqual(items.slice(0, 2));
  });
});
