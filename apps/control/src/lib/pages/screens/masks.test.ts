import { describe, expect, it } from "vitest";
import type { Mask } from "$protocol";
import { canvasOf, fillOf, freeMaskName, maskUsers, maskUsersText, newShape, polygonPoints, setBound, shapeOf, withElement, withElements } from "./masks";

const mask = (over: Partial<Mask> = {}): Mask => ({ schemaVersion: 2, id: "m1", name: "Window", slide: { id: "s1" }, ...over }) as Mask;

describe("mask shapes", () => {
  it("adds centred shapes of each kind on the mask's canvas", () => {
    const m = mask({ size: { width: 1000, height: 500 } });
    expect(canvasOf(mask())).toEqual({ width: 1920, height: 1080 });
    const r = newShape(m, "rectangle", 1);
    expect(r.bounds).toEqual({ x: 250, y: 125, width: 500, height: 250 });
    expect(shapeOf(r)).toBe("rectangle");
    expect(fillOf(r)).toEqual({ r: 0, g: 0, b: 0 });
    expect(shapeOf(newShape(m, "ellipse", 2))).toBe("ellipse");
    const p = newShape(m, "polygon", 3);
    expect(p.kind).toEqual({ type: "shape", shape: { type: "polygon", sides: 6 } });
    expect(p.name).toBe("Polygon 3");
    expect(shapeOf({ id: "t", kind: { type: "text" } })).toBeNull();
  });

  it("edits elements without touching the original", () => {
    const m = mask();
    const e = newShape(m, "rectangle", 1);
    const m2 = withElements(m, (els) => [...els, e]);
    expect(m.slide.elements).toBeUndefined();
    const m3 = withElement(m2, e.id, (x) => setBound(x, "width", 0));
    expect(m3.slide.elements?.[0]?.bounds?.width).toBe(1);
    expect(m2.slide.elements?.[0]?.bounds?.width).toBe(960);
    expect(withElement(m3, e.id, (x) => setBound(x, "x", -20)).slide.elements?.[0]?.bounds?.x).toBe(-20);
  });

  it("draws regular polygons from the top", () => {
    const pts = polygonPoints({ x: 0, y: 0, width: 100, height: 100 }, 4).split(" ");
    expect(pts).toHaveLength(4);
    expect(pts[0]).toBe("50.0,0.0");
    expect(polygonPoints({ x: 0, y: 0, width: 10, height: 10 }, 1).split(" ")).toHaveLength(3);
  });

  it("finds a free name", () => {
    expect(freeMaskName([{ name: "Mask 1" }, { name: "mask 2" }])).toBe("Mask 3");
  });
});

describe("mask delete prompt (D08-5)", () => {
  const names: Record<string, string> = { s1: "Confidence" };
  const name = (id: string) => names[id];
  it("counts stage screens, not just looks", () => {
    const u = maskUsers({ looks: [], screens: ["s1"] }, name);
    expect(u).toEqual({ looks: 0, stages: ["Confidence"] });
    expect(maskUsersText(u!)).toBe("The stage screen “Confidence” uses this mask. Delete it anyway? It will show no mask from then on.");
  });
  it("names looks and stage screens together", () => {
    const u = maskUsers({ looks: ["a", "b"], screens: ["s1", "zz"] }, name)!;
    expect(maskUsersText(u)).toBe(
      "2 looks and 2 stage screens (“Confidence”, “a stage screen”) use this mask. Delete it anyway? They will show no mask from then on.",
    );
    expect(maskUsersText(maskUsers({ looks: ["a"] }, name)!)).toBe("A look uses this mask. Delete it anyway? It will show no mask from then on.");
  });
  it("is null when nothing uses it", () => {
    expect(maskUsers({ looks: [], screens: [] }, name)).toBeNull();
    expect(maskUsers(undefined, name)).toBeNull();
  });
});
