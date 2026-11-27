import { describe, expect, it } from "vitest";
import type { Element, Presentation, Slide } from "$protocol";
import { groupOps, groupChoices, mergeOps, primaryText, primaryTextIndex, reflowBlocks, slideHash, splitOps, splitPoint } from "./reflow";

function textEl(id: string, lines: string[], w = 100, h = 100, extra: Partial<Element> = {}): Element {
  return {
    id,
    bounds: { x: 0, y: 0, width: w, height: h },
    kind: { type: "text", text: { paragraphs: lines.map((l) => (l ? { runs: [{ text: l }] } : {})) } },
    ...extra,
  } as Element;
}

function slide(id: string, els: Element[]): Slide {
  return { id, elements: els } as Slide;
}

const pres: Presentation = {
  id: "p",
  name: "Song",
  sections: [
    { id: "v", name: "Verse 1", color: { r: 0, g: 0, b: 1 }, slides: [slide("s1", [textEl("e1", ["a", "b"])]), slide("s2", [textEl("e2", ["c"])])] },
    { id: "c", name: "Chorus", slides: [slide("s3", [textEl("e3", ["d", "", "e"])]), slide("s4", [])] },
  ],
} as unknown as Presentation;

describe("primary text element", () => {
  it("prefers a text placeholder slot, else the largest text box without a data link", () => {
    const small = textEl("small", ["x"], 10, 10);
    const big = textEl("big", ["y"], 500, 300);
    const linked = textEl("linked", ["clock"], 1000, 1000, { dataLink: { type: "clock" } } as Partial<Element>);
    expect(primaryTextIndex(slide("a", [small, big, linked]))).toBe(1);
    const slot = textEl("slot", ["z"], 5, 5, { placeholder: { slot: "text1", accepts: "text" } } as Partial<Element>);
    expect(primaryTextIndex(slide("b", [big, slot]))).toBe(1);
    expect(primaryTextIndex(slide("c", []))).toBe(-1);
    expect(primaryText(slide("d", [small, big]))).toBe("y");
  });
});

describe("reflow blocks", () => {
  it("lists one block per slide with its group and text", () => {
    const b = reflowBlocks(pres);
    expect(b.map((x) => x.text)).toEqual(["a\nb", "c", "d\n\ne", ""]);
    expect(b.map((x) => [x.index, x.sectionName, x.sectionStart, x.sectionSize])).toEqual([
      [0, "Verse 1", true, 2],
      [1, "Verse 1", false, 2],
      [2, "Chorus", true, 2],
      [3, "Chorus", false, 2],
    ]);
    expect(b[0]!.color).toBe("rgba(0, 0, 255, 1)");
    expect(b[3]!.elementId).toBeNull();
  });
});

describe("text ↔ ops", () => {
  it("finds the split point at the caret", () => {
    expect(splitPoint("ab\ncd", 3)).toEqual({ text: "ab\ncd", atParagraph: 1 }); // start of line 2
    expect(splitPoint("ab\ncd", 2)).toEqual({ text: "ab\ncd", atParagraph: 1 }); // end of line 1
    expect(splitPoint("ab\ncd", 1)).toEqual({ text: "a\nb\ncd", atParagraph: 1 }); // inside a line
    expect(splitPoint("ab\ncd", 5)).toEqual({ text: "ab\ncd", atParagraph: 2 }); // at the end
    expect(splitPoint("ab", 0)).toEqual({ text: "ab", atParagraph: 0 });
  });

  it("saves the text and splits in one batch", () => {
    expect(splitOps("s1", "a\nb", "a\nb", 2)).toEqual([{ type: "splitSlide", slide: "s1", atParagraph: 1 }]);
    expect(splitOps("s1", "ab", "a", 1)).toEqual([
      { type: "setSlideText", slide: "s1", text: "a\nb" },
      { type: "splitSlide", slide: "s1", atParagraph: 1 },
    ]);
    expect(mergeOps("s1", "s2")).toEqual([{ type: "mergeSlides", first: "s1", second: "s2" }]);
  });

  it("assigns groups: renames a section at its first slide, splits it elsewhere", () => {
    const chorus = { name: "Chorus", color: { r: 1, g: 0, b: 0, a: 1 } };
    expect(groupOps(pres, "s1", chorus)).toEqual([
      { type: "setSection", section: "v", name: "Chorus", color: chorus.color, globalGroup: null },
    ]);
    let n = 0;
    const ops = groupOps(pres, "s2", { name: "Bridge", color: null, globalGroup: { id: "g1", name: "Bridge" } }, () => `new${++n}`);
    expect(ops).toEqual([
      { type: "addSection", after: "v", section: { id: "new1", name: "Bridge", globalGroup: { id: "g1", name: "Bridge" } } },
      { type: "moveSlide", slide: "s2", toSection: "new1" },
    ]);
    const tail = groupOps(pres, "s3", chorus, () => "x");
    expect(tail[0]!.type).toBe("setSection");
    expect(groupOps(pres, "nope", chorus)).toEqual([]);
  });

  it("offers section names first, then global groups not already used", () => {
    const g = groupChoices(pres, [
      { name: "chorus", color: null },
      { name: "Bridge", color: null },
    ]);
    expect(g.sections.map((s) => s.name)).toEqual(["Verse 1", "Chorus"]);
    expect(g.global.map((s) => s.name)).toEqual(["Bridge"]);
  });

  it("slide hashes change with the content", () => {
    const a = slide("s", [textEl("e", ["one"])]);
    const b = slide("s", [textEl("e", ["two"])]);
    expect(slideHash(a)).not.toBe(slideHash(b));
    expect(slideHash(a)).toBe(slideHash(structuredClone(a)));
  });
});
