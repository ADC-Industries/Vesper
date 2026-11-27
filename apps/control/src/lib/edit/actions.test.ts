import { describe, expect, it } from "vitest";
import type { SlideAction } from "$protocol";
import { actionSummary, addActionOp, moveActionOp, moveActionTo, newAction, removeActionOp, setActionOp } from "./actions";

const a = (id: string, kind: SlideAction["kind"]): SlideAction => ({ id, kind });
const list = [a("1", { type: "timer", timer: { id: "t" } }), a("2", { type: "prop", prop: { id: "p" }, show: false }), a("3", { type: "look", look: { id: "l", name: "Old" } })];

describe("action summaries", () => {
  it("names targets and operations", () => {
    const names = { timers: new Map([["t", "Countdown"]]), props: new Map([["p", "Logo"]]) };
    expect(actionSummary(list[0]!, names)).toBe("Start: Countdown");
    expect(actionSummary(list[1]!, names)).toBe("Hide: Logo");
    expect(actionSummary(list[2]!, names)).toBe("Old (missing)");
    expect(actionSummary(a("x", { type: "timer", timer: { id: "t" }, operation: { type: "set", seconds: 90 } }), names)).toBe("Set to 1:30: Countdown");
    expect(actionSummary(a("x", { type: "clearLayer", targets: [{ layer: "props" }, {}] }))).toBe("Props, All");
    expect(actionSummary(a("x", { type: "unsupported", sourceType: "ACTION_TYPE_X" }))).toBe("ACTION_TYPE_X (not supported)");
  });
});

describe("action ops", () => {
  it("adds at the end, removes and sets", () => {
    const n = newAction("timer", "n", { timer: { id: "t", name: "T" } })!;
    expect(n.kind).toEqual({ type: "timer", timer: { id: "t", name: "T" }, operation: { type: "start" } });
    expect(newAction("prop", "n", {})).toBeNull();
    expect(addActionOp("s", list, n)).toEqual({ type: "addSlideAction", slide: "s", action: n, after: "3" });
    expect(addActionOp("s", [], n)).toEqual({ type: "addSlideAction", slide: "s", action: n });
    expect(removeActionOp("s", "2")).toEqual({ type: "removeSlideAction", slide: "s", action: "2" });
    expect(setActionOp("s", n)).toEqual({ type: "setSlideAction", slide: "s", action: n });
  });
  it("moves up and down", () => {
    expect(moveActionOp("s", list, 1, -1)).toEqual({ type: "moveSlideAction", slide: "s", action: "2" });
    expect(moveActionOp("s", list, 0, 1)).toEqual({ type: "moveSlideAction", slide: "s", action: "1", after: "2" });
    expect(moveActionOp("s", list, 2, 1)).toBeNull();
    expect(moveActionTo("s", list, 0, 2)).toEqual({ type: "moveSlideAction", slide: "s", action: "1", after: "3" });
    expect(moveActionTo("s", list, 1, 1)).toBeNull();
  });
});
