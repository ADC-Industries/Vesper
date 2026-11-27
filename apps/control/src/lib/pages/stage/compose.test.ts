import { describe, expect, it } from "vitest";
import type { Slide } from "$protocol";
import { composition, compositionText, currentTextSize, layoutClock24, layoutTimer, newStageElement, relinkOps, textSizeOps, toggleOps } from "./compose";

const slide = (): Slide => ({ id: "s", elements: [] });

describe("stage layout composition", () => {
  it("adds and removes data-link elements", () => {
    const s = slide();
    const add = toggleOps(s, "clock", true, { use24Hour: true });
    expect(add).toHaveLength(1);
    const op = add[0]!;
    expect(op.type).toBe("addElement");
    if (op.type !== "addElement") return;
    expect(op.element.dataLink).toEqual({ type: "clock", format: { use24Hour: true } });
    s.elements = [op.element];
    expect(composition(s).clock).toBe(true);
    expect(layoutClock24(s)).toBe(true);
    expect(toggleOps(s, "clock", true)).toEqual([]);
    expect(toggleOps(s, "clock", false)).toEqual([{ type: "deleteElement", slide: "s", element: op.element.id }]);
  });

  it("narrows the current slide when the next one joins it, and widens it back", () => {
    const s = slide();
    const cur = newStageElement("current", s);
    expect(cur.bounds?.width).toBe(1840);
    s.elements = [cur];
    const ops = toggleOps(s, "next", true);
    expect(ops.map((o) => o.type)).toEqual(["addElement", "setElement"]);
    const add = ops[0]!;
    if (add.type !== "addElement") return;
    s.elements = [{ ...cur, bounds: { x: 40, y: 160, width: 1220, height: 700 } }, add.element];
    expect(compositionText(s)).toBe("Current · Next");
    expect(toggleOps(s, "next", false).map((o) => o.type)).toEqual(["deleteElement", "setElement"]);
  });

  it("scales new elements to the layout's canvas", () => {
    const e = newStageElement("timer", slide(), { width: 1280, height: 720, timer: { id: "t1", name: "Sermon" } });
    expect(e.bounds).toEqual({ x: 853, y: 13, width: 400, height: 80 });
    expect(e.dataLink).toEqual({ type: "timer", timer: { id: "t1", name: "Sermon" } });
  });

  it("relinks timers and sets text sizes", () => {
    const s = slide();
    const t = newStageElement("timer", s);
    const c = newStageElement("current", s);
    s.elements = [c, t];
    expect(layoutTimer(s)).toBe("");
    const ops = relinkOps(s, "timer", () => ({ type: "timer", timer: { id: "t2" } }));
    expect(ops.map((o) => o.type)).toEqual(["deleteElement", "addElement"]);
    expect(ops[1]).toMatchObject({ index: 1, element: { dataLink: { type: "timer", timer: { id: "t2" } } } });
    expect(currentTextSize(s)).toBe(110);
    expect(textSizeOps(s, 90)).toEqual([{ type: "setTextStyle", slide: "s", element: c.id, character: { size: 90 } }]);
    expect(compositionText({ id: "x" })).toBe("Empty");
  });
});
