import { describe, expect, it } from "vitest";
import { createShowStore, jsonEqual, select } from "../state";
import { showState } from "./fixtures";

describe("show state store", () => {
  it("applies newer revisions and ignores older ones", () => {
    const s = createShowStore();
    const seen: number[] = [];
    s.subscribe((v) => v && seen.push(v.revision));
    expect(s.apply(10, showState())).toBe(true);
    expect(s.apply(12, showState())).toBe(true);
    expect(s.apply(11, showState())).toBe(false);
    expect(s.apply(12, showState())).toBe(false);
    expect(s.revision).toBe(12);
    expect(seen).toEqual([10, 12]);
  });

  it("accepts any revision after resync (server restarted)", () => {
    const s = createShowStore();
    s.apply(500, showState());
    s.resync();
    expect(s.apply(3, showState())).toBe(true);
    expect(s.revision).toBe(3);
    // ...and orders again from there.
    expect(s.apply(2, showState())).toBe(false);
  });

  it("waitFor resolves once the revision arrives", async () => {
    const s = createShowStore();
    s.apply(1, showState());
    let done = false;
    const p = s.waitFor(3).then(() => (done = true));
    s.apply(2, showState());
    await Promise.resolve();
    expect(done).toBe(false);
    s.apply(3, showState());
    await p;
    expect(done).toBe(true);
  });

  it("select only notifies when the selected slice changes", () => {
    const s = createShowStore();
    const look = select(s, (st) => st?.activeLook ?? null);
    const layers = select(s, (st) => st?.layers ?? null, jsonEqual);
    const looks: (string | null)[] = [];
    let layerCalls = 0;
    look.subscribe((v) => looks.push(v));
    layers.subscribe(() => layerCalls++);
    s.apply(1, showState({ activeLook: "a" }));
    s.apply(2, showState({ activeLook: "a" })); // e.g. only fps counters changed
    s.apply(3, showState({ activeLook: "b" }));
    expect(looks).toEqual([null, "a", "b"]);
    expect(layerCalls).toBe(2); // initial null + first state; identical layers after that
  });
});
