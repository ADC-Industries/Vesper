import { describe, expect, it } from "vitest";
import { shown, type Sent } from "./optimistic";

describe("optimistic values (D10-2)", () => {
  it("builds on what was just sent until its save settled", () => {
    const slide = { id: "s1", builds: [{ delay: 0 }] };
    // The Delay field committed 2: the next click builds on it, not on the stale prop.
    const sent: Sent<{ delay: number }[]> = { key: slide.id, value: [{ delay: 2 }] };
    expect(shown(sent, slide.id, slide.builds)).toEqual([{ delay: 2 }]);
    // Settled (the caller clears it): the server's value wins (also after a failure).
    expect(shown(null, slide.id, slide.builds)).toBe(slide.builds);
  });

  it("keeps a later change when an earlier save's refetch lands first (E10-5)", () => {
    // Change 1 (delay) and change 2 (moved, built on 1) are sent; run 1's refetch brings a new
    // slide object with change 1 only. Change 2 is still what's shown, so a third change made
    // now builds on it instead of undoing the move.
    const sent2: Sent<string[]> = { key: "s1", value: ["delay", "moved"] };
    const afterRun1 = { id: "s1", builds: ["delay"] };
    expect(shown(sent2, afterRun1.id, afterRun1.builds)).toEqual(["delay", "moved"]);
  });

  it("never shows a value sent for another slide, also when neither has one (E10-6)", () => {
    // Push picked for slide 1 (on Default); slide 2 (also on Default) is selected before the
    // save is back: its grid shows its own (none), not slide 1's Push.
    const sent: Sent<string | undefined> = { key: "slide-1", value: "push" };
    expect(shown(sent, "slide-2", undefined)).toBeUndefined();
    expect(shown(sent, "slide-1", undefined)).toBe("push");
  });
});
