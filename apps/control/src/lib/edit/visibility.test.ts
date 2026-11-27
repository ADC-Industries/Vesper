import { describe, expect, it } from "vitest";
import type { VisibilityCondition } from "$protocol";
import { CONDITION_KINDS, activeOf, conditionLabel, newCondition, removeCondition, replaceCondition, timerShowLink, withActive, withCondition } from "./visibility";

describe("visibility conditions", () => {
  it("makes a new condition of every offered kind", () => {
    for (const k of CONDITION_KINDS) {
      const c = newCondition(k.kind, [{ id: "other" }], [{ id: "in1", name: "Wide shot" }]);
      expect(c?.type).toBe(k.kind);
    }
    expect(newCondition("elementHasText", [{ id: "e2" }])).toEqual({ type: "elementHasText", element: "e2" });
    // Nothing to name: no "object has text" condition.
    expect(newCondition("elementHasText", [])).toBeNull();
    // "Video input live" names the show's first input; none: no such condition.
    expect(newCondition("videoInput", [], [{ id: "in1", name: "Wide shot" }, { id: "in2" }])).toEqual({ type: "videoInput", input: { id: "in1", name: "Wide shot" } });
    expect(newCondition("videoInput", [{ id: "e2" }])).toBeNull();
  });

  it("edits the whole list", () => {
    const a: VisibilityCondition = { type: "stageMessage" };
    const b: VisibilityCondition = { type: "layer", layer: "props", active: false };
    const list = withCondition(withCondition(undefined, a), b);
    expect(list).toEqual([a, b]);
    expect(replaceCondition(list, 0, { type: "message" })).toEqual([{ type: "message" }, b]);
    expect(removeCondition(list, 0)).toEqual([b]);
    expect(removeCondition(undefined, 0)).toEqual([]);
  });

  it("writes active only when false, as the server does", () => {
    const c: VisibilityCondition = { type: "stageMessage" };
    expect(activeOf(c)).toBe("yes");
    const off = withActive(c, "no");
    expect(off).toEqual({ type: "stageMessage", active: false });
    expect(activeOf(off)).toBe("no");
    expect(withActive(off, "yes")).toEqual({ type: "stageMessage" });
  });

  it("labels conditions", () => {
    expect(conditionLabel({ type: "elementHasText", element: "e1", hasText: false }, { element: () => "Point 1" })).toBe("Point 1 is empty");
    expect(conditionLabel({ type: "message" })).toBe("Any message showing");
    expect(conditionLabel({ type: "message", message: { id: "m1" }, active: false }, { message: () => "Welcome" })).toBe("Welcome not showing");
    expect(conditionLabel({ type: "layer", layer: "props", active: false })).toBe("Props layer clear");
    expect(conditionLabel({ type: "playlistItem", which: "next" })).toBe("Next item exists");
    expect(conditionLabel({ type: "videoCountdown", state: "looping" })).toBe("Video loops");
    expect(conditionLabel({ type: "timer", state: "isRunning" })).toBe("Most recent timer is running");
    expect(conditionLabel({ type: "videoInput", input: { id: "in1", name: "Old name" }, active: false })).toBe("Old name not live");
    expect(conditionLabel({ type: "videoInput", input: { id: "in1", name: "Old name" } }, { input: () => "Wide shot" })).toBe("Wide shot live");
    expect(conditionLabel({ type: "unsupported", sourceType: "visibilityLink" })).toContain("not evaluated");
  });

  it("sets a timer link's time", () => {
    const link = { type: "timer" as const, timer: { id: "t1" } };
    expect(timerShowLink(link, "elapsed")).toEqual({ type: "timer", timer: { id: "t1" }, show: "elapsed" });
    expect(timerShowLink({ ...link, show: "remaining" }, "value")).toEqual(link);
  });
});
