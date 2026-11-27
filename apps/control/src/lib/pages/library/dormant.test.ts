import { describe, expect, it } from "vitest";
import type { DormantUpload } from "$protocol";
import { dormantCard, offerText, progressText } from "./dormant";

const base: DormantUpload = {
  state: "idle",
  source: "6a1d1c8e-0000-4000-8000-0000000000c1",
  name: "Youth",
  songs: 12,
  documents: { done: 0, total: 40 },
  media: { done: 0, total: 0 },
  bytes: { done: 0, total: 0 },
};

describe("dormant upload words", () => {
  it("offers the collection with its songs", () => {
    expect(offerText(base)).toBe("This box's own collection “Youth” (12 songs) isn't in the site's Codex.");
    expect(offerText({ name: "One", songs: 1 })).toContain("(1 song)");
    expect(dormantCard(base)?.tone).toBe("info");
    expect(dormantCard(null)).toBeNull();
  });

  it("counts documents first, then files and bytes", () => {
    const docs = progressText({ ...base, state: "copying", documents: { done: 10, total: 40 }, media: { done: 0, total: 3 } });
    expect(docs.text).toBe("Copying documents: 10 of 40");
    expect(docs.fraction).toBeCloseTo(0.075);
    const media = progressText({
      ...base,
      state: "copying",
      documents: { done: 40, total: 40 },
      media: { done: 1, total: 3 },
      bytes: { done: 5 * 1024 * 1024, total: 20 * 1024 * 1024 },
    });
    expect(media.text).toBe("Copying media: 1 of 3 files · 5.0 MB of 20 MB");
    expect(media.fraction).toBeCloseTo(0.475);
  });

  it("says where it went, or why it stopped", () => {
    expect(dormantCard({ ...base, state: "done", collectionName: "Youth (from BOX-B)" })?.detail).toContain("as “Youth (from BOX-B)”");
    expect(dormantCard({ ...base, state: "failed", error: "presentation x was written by a newer server" })?.detail).toContain("newer server");
    expect(dormantCard({ ...base, state: "cancelled" })?.detail).toContain("Upload again");
  });
});
