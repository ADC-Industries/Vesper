import { describe, expect, it } from "vitest";
import { MAX_TOASTS, toastLifetime, visibleToasts } from "./toasts";

const n = (k: number) => Array.from({ length: k }, (_, i) => ({ id: i + 1 }));

describe("toasts", () => {
  it("shows up to three, then two and 'N more' for the older ones", () => {
    expect(visibleToasts(n(3))).toEqual({ shown: n(3), folded: 0 });
    const five = visibleToasts(n(5));
    expect(five.shown.map((t) => t.id)).toEqual([4, 5]);
    expect(five.folded).toBe(3);
    expect(five.shown.length + (five.folded ? 1 : 0)).toBeLessThanOrEqual(MAX_TOASTS);
  });

  it("keeps errors until dismissed; the rest go after 2.6 s", () => {
    expect(toastLifetime("error")).toBeNull();
    expect(toastLifetime("warning")).toBe(2600);
    expect(toastLifetime("info")).toBe(2600);
  });
});
