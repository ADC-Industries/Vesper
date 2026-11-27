import { describe, expect, it } from "vitest";
import type { Presentation, Slide, SlideAction } from "$protocol";
import { canContinueAbove, continueGroupOps, deleteSlideOps, entryFocusable, mediaCues, removeCueOps, slideBadges, slideMenuEntries } from "./slideMenu";

const cue = (id: string, name?: string): SlideAction => ({ id, ...(name ? { name } : {}), kind: { type: "media", asset: "a1" } }) as SlideAction;
const slide = (id: string, extra: Partial<Slide> = {}): Slide => ({ id, elements: [], ...extra }) as Slide;

const pres = {
  id: "p",
  name: "Song",
  sections: [
    { id: "intro", name: "", slides: [slide("s0", { actions: [cue("c1", "dawn-loop.mp4"), { id: "t", kind: { type: "clearGroup", group: { id: "g" } } } as SlideAction] })] },
    { id: "v", name: "Verse 1", hotkey: { code: "KeyV" }, slides: [slide("s1"), slide("s2", { hotkey: { code: "Digit2", shift: true } })] },
    { id: "c", name: "Chorus", slides: [slide("s3"), slide("s4")] },
    { id: "e", name: "Ending", slides: [slide("s5")] },
  ],
} as unknown as Presentation;

describe("continue the group above", () => {
  it("moves the whole group after the last slide of the group above and drops its section", () => {
    expect(canContinueAbove(pres, "s3")).toBe(true);
    expect(continueGroupOps(pres, "s3")).toEqual([
      { type: "moveSlide", slide: "s3", toSection: "v", after: "s2" },
      { type: "moveSlide", slide: "s4", toSection: "v", after: "s3" },
      { type: "deleteSection", section: "c" },
    ]);
  });

  it("does nothing for a slide that already continues a group, or the first group", () => {
    expect(canContinueAbove(pres, "s4")).toBe(false);
    expect(continueGroupOps(pres, "s4")).toEqual([]);
    expect(canContinueAbove(pres, "s0")).toBe(false);
    expect(continueGroupOps(pres, "s0")).toEqual([]);
    expect(continueGroupOps(pres, "nope")).toEqual([]);
  });

  it("moves into an empty group above without an `after`", () => {
    const p = { id: "p", sections: [{ id: "a", slides: [] }, { id: "b", slides: [slide("x")] }] } as unknown as Presentation;
    expect(continueGroupOps(p, "x")).toEqual([
      { type: "moveSlide", slide: "x", toSection: "a" },
      { type: "deleteSection", section: "b" },
    ]);
  });
});

describe("delete, media cues and badges", () => {
  it("deletes a slide, and its group when it was the group's only slide", () => {
    expect(deleteSlideOps(pres, "s3")).toEqual([{ type: "deleteSlide", slide: "s3" }]);
    expect(deleteSlideOps(pres, "s5")).toEqual([
      { type: "deleteSlide", slide: "s5" },
      { type: "deleteSection", section: "e" },
    ]);
    expect(deleteSlideOps(pres, "nope")).toEqual([]);
  });

  it("removes only the media actions", () => {
    const s0 = pres.sections![0]!.slides![0]!;
    expect(mediaCues(s0).map((a) => a.id)).toEqual(["c1"]);
    expect(removeCueOps(s0)).toEqual([{ type: "removeSlideAction", slide: "s0", action: "c1" }]);
    expect(removeCueOps(slide("bare"))).toEqual([]);
    expect(removeCueOps(undefined)).toEqual([]);
  });

  it("shows the slide's hotkey, else its group's on the group's first slide, and the cue name", () => {
    const [intro, v] = pres.sections!;
    expect(slideBadges(intro!.slides![0], intro, true)).toEqual({ hotkey: "", cue: "dawn-loop.mp4", cueCount: 1 });
    expect(slideBadges(v!.slides![0], v, true).hotkey).toBe("V");
    expect(slideBadges(v!.slides![1], v, false).hotkey).toBe("Shift+2");
    expect(slideBadges(slide("x", { hotkey: undefined }), v, false).hotkey).toBe("");
    expect(slideBadges(slide("y", { actions: [cue("c")] }), v, false).cue).toBe("Media");
  });
});

describe("the menu entries", () => {
  const choices = {
    sections: [
      { name: "Verse 1", color: null },
      { name: "Chorus", color: null },
    ],
    global: [{ name: "Bridge", color: null }],
  };

  it("lists the groups with the current one checked, then the actions", () => {
    const e = slideMenuEntries({ choices, startsGroup: "chorus", isFirst: false, canContinue: true, hasCue: false });
    expect(e.map((x) => x.label ?? (x.separator ? "—" : ""))).toEqual([
      "Start a group here",
      "Verse 1",
      "Chorus",
      "—",
      "Bridge",
      "New group…",
      "—",
      "Continue the group above",
      "Insert slide break at cursor",
      "Merge with slide above",
      "Delete slide",
    ]);
    expect(e.filter((x) => x.checked).map((x) => x.label)).toEqual(["Chorus"]);
    expect(e.find((x) => x.id === "delete")?.danger).toBe(true);
    expect(e.filter(entryFocusable).length).toBe(8);
  });

  it("disables what doesn't apply and offers the cue removal only with a cue", () => {
    const e = slideMenuEntries({ choices: { sections: [], global: choices.global }, startsGroup: null, isFirst: true, canContinue: false, hasCue: true });
    expect(e.find((x) => x.id === "continue")?.disabled).toBe(true);
    expect(e.find((x) => x.id === "merge")?.disabled).toBe(true);
    expect(e.some((x) => x.id === "removeCue")).toBe(true);
    expect(e.some((x) => x.id === "sep:global")).toBe(false);
    expect(e.some((x) => x.checked)).toBe(false);
  });

  it("offers translating a slide when the box can", () => {
    const off = slideMenuEntries({ choices: { sections: [], global: [] }, startsGroup: null, isFirst: true, canContinue: false, hasCue: false });
    expect(off.some((e) => e.action === "translate")).toBe(false);
    const on = slideMenuEntries({ choices: { sections: [], global: [] }, startsGroup: null, isFirst: true, canContinue: false, hasCue: false, translate: true });
    const t = on.find((e) => e.action === "translate");
    expect(t?.label).toBe("Translate this slide…");
    // Before "Delete slide", which stays last.
    expect(on[on.length - 1]?.action).toBe("delete");
  });
});
