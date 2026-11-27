import { describe, expect, it } from "vitest";
import type { TextBox } from "$protocol";
import { cpLength, cpToUtf16, editedFromTextBox, plainText, rangeFromUtf16, rangeText, rebuildTextBox, styleAt, utf16ToCp } from "./text";

const RED = { r: 1, g: 0, b: 0 };
const box: TextBox = {
  defaultStyle: { fontFamily: "Carlito", size: 100 },
  paragraphs: [
    { runs: [{ text: "Morning " }, { text: "light", style: { color: RED } }] },
    { style: { alignment: "left" }, runs: [{ text: "how 😀 sweet" }] },
  ],
};

describe("code points", () => {
  it("counts the paragraph break as one and astral characters as one", () => {
    const t = plainText(box);
    expect(t).toBe("Morning light\nhow 😀 sweet");
    expect(cpLength(t)).toBe(25);
    expect(t.length).toBe(26);
  });
  it("converts DOM (UTF-16) offsets to code points and back", () => {
    const t = plainText(box);
    const sweet = t.indexOf("sweet");
    expect(utf16ToCp(t, sweet)).toBe(sweet - 1);
    expect(cpToUtf16(t, utf16ToCp(t, sweet))).toBe(sweet);
    // Inside the surrogate pair: rounds up past the emoji.
    const emoji = t.indexOf("😀");
    expect(utf16ToCp(t, emoji + 1)).toBe(utf16ToCp(t, emoji) + 1);
  });
  it("makes a range from a selection in either direction", () => {
    const t = plainText(box);
    const s = t.indexOf("sweet");
    const r = rangeFromUtf16(t, s + 5, s);
    expect(r).toEqual({ start: 20, end: 25 });
    expect(rangeText(t, r!)).toBe("sweet");
    expect(rangeFromUtf16(t, 3, 3)).toBeNull();
  });
  it("reads the style at a code point through the cascade", () => {
    expect(styleAt(box, 8).color).toEqual(RED);
    expect(styleAt(box, 8).fontFamily).toBe("Carlito");
    expect(styleAt(box, 0).color).toEqual({ r: 1, g: 1, b: 1 });
  });
});

describe("rebuilding the text box", () => {
  it("round-trips an unedited box", () => {
    expect(rebuildTextBox(box, editedFromTextBox(box))).toEqual(box);
  });
  it("keeps run styles of edited text and copies paragraph style to new paragraphs", () => {
    const ed = editedFromTextBox(box);
    ed[0]!.pieces[1]!.text = "grace!!";
    ed.push({ from: null, pieces: [{ text: "new line", run: null }] });
    const tb = rebuildTextBox(box, ed);
    expect(tb.paragraphs![0]!.runs![1]).toEqual({ text: "grace!!", style: { color: RED } });
    expect(tb.paragraphs![2]).toEqual({ style: { alignment: "left" }, runs: [{ text: "new line" }] });
    expect(tb.defaultStyle).toEqual(box.defaultStyle);
  });
  it("merges pieces of the same style and drops empty runs", () => {
    const tb = rebuildTextBox(box, [{ from: 0, pieces: [{ text: "a", run: [0, 0] }, { text: "", run: [0, 1] }, { text: "b", run: [0, 0] }] }]);
    expect(tb.paragraphs).toEqual([{ runs: [{ text: "ab" }] }]);
  });
});
