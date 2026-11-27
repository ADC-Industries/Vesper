import { describe, expect, it } from "vitest";
import type { Build, Slide } from "$protocol";
import { buildSteps, effectOf, hiddenAt, moveBuild, newBuild, stepCount, withEffect } from "./builds";

const el = (id: string) => ({ id });
const slide = (builds: Build[]): Pick<Slide, "builds" | "elements"> => ({ elements: ["a", "b", "c", "d"].map(el), builds });
const dis = (d: number) => ({ kind: { type: "dissolve" as const }, duration: d });

describe("object builds", () => {
  it("places builds in steps and time as the server does", () => {
    const s = slide([
      { element: "a", start: "withSlide", delay: 0.5, transition: dis(1) },
      { element: "b", transition: dis(1) },
      { element: "c", start: "withPrevious", delay: 0.25, transition: dis(1) },
      { element: "d", start: "afterPrevious", delay: 0.5, transition: dis(2) },
      { element: "gone", transition: dis(1) },
    ]);
    expect(buildSteps(s).map((t) => [t.index, t.step, t.start, t.duration])).toEqual([
      [0, 0, 0.5, 1],
      [1, 1, 0, 1],
      [2, 1, 0.25, 1],
      [3, 1, 1.75, 2],
    ]);
    expect(stepCount(s)).toBe(1);
  });

  it("hides what has not built in, and what has built out", () => {
    const s = slide([newBuild("a", "in"), newBuild("b", "out"), { element: "c", start: "withSlide" }]);
    expect(hiddenAt(s, 0).sort()).toEqual(["a"]);
    expect(hiddenAt(s, 1)).toEqual([]);
    expect(hiddenAt(s, 2)).toEqual(["b"]);
  });

  it("switches effects keeping the duration and direction", () => {
    const push = withEffect(dis(0.8), "push");
    expect(push).toEqual({ kind: { type: "push", direction: "left" }, duration: 0.8, easing: "easeInOut" });
    expect(effectOf(push)).toBe("push");
    expect(withEffect({ kind: { type: "wipe", direction: "center" }, duration: 1 }, "slide").kind).toEqual({ type: "slide", direction: "left" });
    expect(withEffect(undefined, "dissolve")).toEqual(dis(0.5));
    expect(effectOf(undefined)).toBe("cut");
    expect(effectOf({ kind: { type: "fadeThroughColor", color: { r: 0, g: 0, b: 0 } }, duration: 1 })).toBe("dissolve");
  });

  it("moves builds within the order", () => {
    const list = [newBuild("a", "in"), newBuild("b", "in")];
    expect(moveBuild(list, 1, -1).map((b) => b.element)).toEqual(["b", "a"]);
    expect(moveBuild(list, 0, -1)).toBe(list);
  });
});
