import { describe, expect, it } from "vitest";
import { COLOR_FIELDS, IDENTITY_COLOR, clampField, colorOf, cornerPoint, fieldNumber, isConvex, isIdentityColor, offsetFor, pinOf, quadOf } from "./adjust";

describe("colour correction", () => {
  it("fills in defaults and knows the identity", () => {
    expect(colorOf(null)).toEqual(IDENTITY_COLOR);
    expect(isIdentityColor(colorOf({ color: undefined }))).toBe(true);
    const c = colorOf({ color: { ...IDENTITY_COLOR, gamma: 1.4 } });
    expect(c.gamma).toBe(1.4);
    expect(isIdentityColor(c)).toBe(false);
  });
  it("clamps and rounds to the field", () => {
    const gamma = COLOR_FIELDS.find((f) => f.key === "gamma")!;
    expect(clampField(gamma, 0)).toBe(0.1);
    expect(clampField(gamma, 9)).toBe(4);
    expect(clampField(gamma, 1.234)).toBe(1.23);
    expect(clampField(gamma, Number.NaN)).toBe(1);
  });
  it("reads an emptied number field as no number, not 0 (A16-3)", () => {
    expect(fieldNumber("")).toBeNaN();
    expect(fieldNumber("  ")).toBeNaN();
    expect(fieldNumber("abc")).toBeNaN();
    expect(fieldNumber("0")).toBe(0);
    expect(fieldNumber("1.25")).toBe(1.25);
    expect(fieldNumber("-3")).toBe(-3);
  });
});

describe("corner pin", () => {
  const W = 1920;
  const H = 1080;
  it("positive offsets move corners outwards", () => {
    const p = pinOf({ cornerPin: { enabled: true, topLeft: { x: 10, y: 20 }, topRight: { x: 10, y: 20 }, bottomRight: { x: 10, y: 20 }, bottomLeft: { x: 10, y: 20 } } });
    expect(quadOf(p, W, H)).toEqual([
      [-10, -20],
      [1930, -20],
      [1930, 1100],
      [-10, 1100],
    ]);
  });
  it("dragging a corner gives the offset back", () => {
    const p = pinOf(null);
    for (const c of ["topLeft", "topRight", "bottomRight", "bottomLeft"] as const) {
      const o = offsetFor(c, 100, 900, W, H);
      const q = cornerPoint({ ...p, [c]: o }, c, W, H);
      expect(q).toEqual([100, 900]);
    }
    expect(offsetFor("topLeft", 50, 30, W, H)).toEqual({ x: -50, y: -30 });
  });
  it("checks convexity like the server", () => {
    expect(isConvex(quadOf(pinOf(null), W, H))).toBe(true);
    const crossed = pinOf({ cornerPin: { enabled: true, topLeft: { x: -W, y: 0 }, topRight: { x: -W, y: 0 }, bottomRight: { x: 0, y: 0 }, bottomLeft: { x: 0, y: 0 } } });
    expect(isConvex(quadOf(crossed, W, H))).toBe(false);
  });
});
