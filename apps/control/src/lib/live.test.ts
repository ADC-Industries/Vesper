import { describe, expect, it } from "vitest";
import type { SlidePosition } from "$protocol";
import { LatestLive } from "./live";

const pos = (presentation: string, slide: string): SlidePosition =>
  ({ presentation, presentationName: presentation, slide, slideIndex: 0, section: "s", sectionName: "S", playlist: null }) as SlidePosition;
const st = (slide: SlidePosition | null, announcements: SlidePosition | null, cursor: SlidePosition | null = null) => ({ layers: { slide, announcements }, cursor });

describe("the slide triggered last (X3-8)", () => {
  it("follows whichever layer changed last: an announcement, then a song slide again", () => {
    const l = new LatestLive();
    const song = pos("song", "s9");
    expect(l.update(st(song, null))).toEqual(song);
    const a3 = pos("ann", "a3");
    expect(l.update(st(song, a3))).toEqual(a3);
    // Another state (meters, timers): unchanged layers keep the choice.
    expect(l.update(st(song, a3))).toEqual(a3);
    const a4 = pos("ann", "a4");
    expect(l.update(st(song, a4))).toEqual(a4);
    const s1 = pos("song2", "s1");
    expect(l.update(st(s1, a4))).toEqual(s1);
    // The announcements layer cleared, or the slide layer: the other one.
    expect(l.update(st(null, a4))).toEqual(a4);
    expect(l.update(st(s1, null))).toEqual(s1);
    expect(l.update(st(null, null))).toBeNull();
  });

  it("at first sight of both layers, the one the cursor continues", () => {
    const song = pos("song", "s9");
    const ann = pos("ann", "a3");
    expect(new LatestLive().update(st(song, ann, pos("ann", "a4")))).toEqual(ann);
    expect(new LatestLive().update(st(song, ann, pos("song", "s10")))).toEqual(song);
    expect(new LatestLive().update(st(null, ann))).toEqual(ann);
  });
});
