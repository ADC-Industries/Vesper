import { describe, expect, it } from "vitest";
import { compareValues, menuMove, menuTypeahead, nextSort, segmentedKey, sortRows, splitterKey } from "./logic";

describe("splitterKey", () => {
  it("moves by 10 px, 50 with Shift, along its own axis", () => {
    expect(splitterKey("ArrowRight", false, 200, { dir: "col" })).toBe(210);
    expect(splitterKey("ArrowLeft", true, 200, { dir: "col" })).toBe(150);
    expect(splitterKey("ArrowDown", false, 200, { dir: "row" })).toBe(210);
    expect(splitterKey("ArrowUp", false, 200, { dir: "row" })).toBe(190);
  });
  it("ignores the other axis and other keys", () => {
    expect(splitterKey("ArrowUp", false, 200, { dir: "col" })).toBeNull();
    expect(splitterKey("ArrowLeft", false, 200, { dir: "row" })).toBeNull();
    expect(splitterKey("a", false, 200, { dir: "row" })).toBeNull();
  });
  it("follows the scale (a panel that grows the other way) and clamps", () => {
    expect(splitterKey("ArrowRight", false, 200, { dir: "col", scale: -1 })).toBe(190);
    expect(splitterKey("ArrowRight", false, 395, { dir: "col", max: 400 })).toBe(400);
    expect(splitterKey("ArrowLeft", false, 105, { dir: "col", min: 100 })).toBe(100);
  });
  it("jumps with Home / End only when bounds are given", () => {
    expect(splitterKey("Home", false, 200, { dir: "col", min: 120, max: 500 })).toBe(120);
    expect(splitterKey("End", false, 200, { dir: "col", min: 120, max: 500 })).toBe(500);
    expect(splitterKey("Home", false, 200, { dir: "col" })).toBeNull();
  });
});

describe("menuMove", () => {
  const items = [{}, { separator: true }, { disabled: true }, {}, {}];
  it("skips separators and disabled entries, and wraps", () => {
    expect(menuMove(items, 0, "ArrowDown")).toBe(3);
    expect(menuMove(items, 3, "ArrowUp")).toBe(0);
    expect(menuMove(items, 4, "ArrowDown")).toBe(0);
    expect(menuMove(items, 0, "ArrowUp")).toBe(4);
  });
  it("starts at the first or last entry when nothing has the focus", () => {
    expect(menuMove(items, -1, "ArrowDown")).toBe(0);
    expect(menuMove(items, -1, "ArrowUp")).toBe(4);
    expect(menuMove(items, 3, "Home")).toBe(0);
    expect(menuMove(items, 0, "End")).toBe(4);
  });
  it("stays put when nothing else is focusable, and ignores other keys", () => {
    expect(menuMove([{}, { disabled: true }], 0, "ArrowDown")).toBe(0);
    expect(menuMove(items, 0, "Enter")).toBeNull();
    expect(menuMove([], 0, "ArrowDown")).toBeNull();
  });
  it("type-ahead finds the next entry starting with the letter", () => {
    const labels = ["Copy", null, "Cut", "Clear", "Paste"];
    expect(menuTypeahead(labels, items, 0, "c")).toBe(3); // "Cut" is disabled
    expect(menuTypeahead(labels, items, 3, "C")).toBe(0);
    expect(menuTypeahead(labels, items, 0, "x")).toBeNull();
  });
});

describe("segmentedKey", () => {
  const opts = [{ value: "show" }, { value: "edit", disabled: true }, { value: "reflow" }];
  it("moves with arrows, skipping disabled options and wrapping", () => {
    expect(segmentedKey(opts, "show", "ArrowRight")).toBe("reflow");
    expect(segmentedKey(opts, "reflow", "ArrowRight")).toBe("show");
    expect(segmentedKey(opts, "show", "ArrowLeft")).toBe("reflow");
    expect(segmentedKey(opts, "reflow", "ArrowUp")).toBe("show");
  });
  it("Home and End, other keys ignored", () => {
    expect(segmentedKey(opts, "reflow", "Home")).toBe("show");
    expect(segmentedKey(opts, "show", "End")).toBe("reflow");
    expect(segmentedKey(opts, "show", "Enter")).toBeNull();
    expect(segmentedKey([{ value: 1, disabled: true }], 1, "ArrowRight")).toBeNull();
  });
});

describe("DataTable sorting", () => {
  const rows = [
    { t: "Morning Song 10", n: 3 },
    { t: "morning song 2", n: null },
    { t: "Evening", n: 10 },
    { t: "", n: 1 },
  ];
  it("sorts text by locale with numbers in order, empties last both ways", () => {
    expect(sortRows(rows, (r) => r.t, "asc").map((r) => r.t)).toEqual(["Evening", "morning song 2", "Morning Song 10", ""]);
    expect(sortRows(rows, (r) => r.t, "desc").map((r) => r.t)).toEqual(["Morning Song 10", "morning song 2", "Evening", ""]);
  });
  it("sorts numbers numerically and keeps ties stable", () => {
    expect(sortRows(rows, (r) => r.n, "asc").map((r) => r.n)).toEqual([1, 3, 10, null]);
    const ties = [{ k: 1, i: "a" }, { k: 0, i: "b" }, { k: 1, i: "c" }];
    expect(sortRows(ties, (r) => r.k, "desc").map((r) => r.i)).toEqual(["a", "c", "b"]);
  });
  it("a header click flips the same column and starts others ascending", () => {
    expect(nextSort(null, "title")).toEqual({ key: "title", dir: "asc" });
    expect(nextSort({ key: "title", dir: "asc" }, "title")).toEqual({ key: "title", dir: "desc" });
    expect(nextSort({ key: "title", dir: "desc" }, "year")).toEqual({ key: "year", dir: "asc" });
  });
  it("compares dates and booleans", () => {
    expect(compareValues(new Date(1), new Date(2))).toBeLessThan(0);
    expect(compareValues(true, false)).toBeGreaterThan(0);
  });
});
