import { describe, expect, it } from "vitest";
import {
  chordOnLine,
  chordViewText,
  inlineChords,
  keyDistance,
  keyLabel,
  parseChord,
  playedKey,
  remapChords,
  setParagraphChords,
  showChord,
  transposeChord,
  withChord,
} from "./chords";
import { rebuildTextBox, editedFromTextBox } from "./text";
import { compositionText } from "../pages/stage/compose";
import { DATA_LINKS, dataLinkLabel } from "../pages/stage/stage";

describe("chords", () => {
  it("parses chords and refuses words", () => {
    expect(parseChord("G/B")?.bass?.pc).toBe(11);
    expect(parseChord("C6/9")?.suffix).toBe("6/9");
    expect(parseChord("Day")).toBeNull();
    expect(parseChord("N.C.")).toBeNull();
    expect(parseChord("C#m7b5")).not.toBeNull();
  });

  it("transposes slash chords, minors, sus/add/7ths and enharmonics", () => {
    expect(transposeChord("G/B", 2)).toBe("A/C#");
    expect(transposeChord("Em7", 2)).toBe("F#m7");
    expect(transposeChord("Csus4", 2)).toBe("Dsus4");
    expect(transposeChord("Cadd9", 2)).toBe("Dadd9");
    expect(transposeChord("Bb", 2)).toBe("C");
    expect(transposeChord("Bb", 3)).toBe("Db");
    expect(transposeChord("C", 1, "flats")).toBe("Db");
    expect(transposeChord("Gb", 0, "sharps")).toBe("F#");
    expect(transposeChord("B", 1)).toBe("C");
    expect(transposeChord("N.C.", 3)).toBe("N.C.");
    // By the key: G down to F spells flats.
    expect(showChord("C", -2, { key: "G" })).toBe("Bb");
    expect(showChord("E7", 2, { key: "C" })).toBe("F#7");
  });

  it("Nashville numbers, keys and key lines", () => {
    expect(showChord("D/F#", 3, { key: "G", nashville: true })).toBe("5/7");
    expect(showChord("Em7", 0, { key: "G", nashville: true })).toBe("6m7");
    expect(playedKey("G", 2)).toBe("A");
    expect(playedKey("", 2)).toBe("");
    expect(keyDistance("G", "A")).toBe(2);
    expect(keyDistance("C", "Bb")).toBe(-2);
    expect(keyLabel({ capo: 2 }, "G", 2)).toBe("Key A · Capo 2 (G shapes)");
    expect(keyLabel({}, "G", 2)).toBe("Key A");
    expect(keyLabel({ capo: 3 }, null, 0)).toBe("Capo 3");
    expect(chordViewText({ capo: 2, accidentals: "flats" })).toBe("Capo 2 · flats");
    expect(chordViewText(undefined)).toBe("Concert pitch");
  });

  it("chords follow edited words", () => {
    const marks = [
      { at: 0, chord: "G" },
      { at: 7, chord: "C" },
    ];
    expect(remapChords("Sing a song", "Sing a happy song", marks)).toEqual([
      { at: 0, chord: "G" },
      { at: 13, chord: "C" },
    ]);
    expect(remapChords("Sing a song", "Sing it", [{ at: 7, chord: "C" }])).toEqual([{ at: 6, chord: "C" }]);
  });

  it("edits a line's chords and writes them inline", () => {
    let c = withChord([{ at: 4, chord: "D" }], 0, "G");
    expect(c).toEqual([
      { at: 0, chord: "G" },
      { at: 4, chord: "D" },
    ]);
    c = withChord(c, 4, "");
    expect(c).toEqual([{ at: 0, chord: "G" }]);
    expect(inlineChords("Hold on", [{ at: 5, chord: "D" }, { at: 7, chord: "G" }])).toBe("Hold [D]on[G]");
    const tb = setParagraphChords({ paragraphs: [{ runs: [{ text: "a" }] }] }, 0, [{ at: 0, chord: "E" }]);
    expect(tb.paragraphs?.[0]?.chords).toEqual([{ at: 0, chord: "E" }]);
    expect(setParagraphChords(tb, 0, []).paragraphs?.[0]?.chords).toBeUndefined();
  });

  it("the in-place editor keeps a line's chords on its words", () => {
    const tb = { paragraphs: [{ runs: [{ text: "Sing a song" }], chords: [{ at: 7, chord: "C" }] }, { runs: [{ text: "two" }] }] };
    const edited = editedFromTextBox(tb);
    edited[0] = { from: 0, pieces: [{ text: "Sing a new song", run: [0, 0] }] };
    const out = rebuildTextBox(tb, edited);
    expect(out.paragraphs?.[0]?.chords).toEqual([{ at: 11, chord: "C" }]);
    expect(out.paragraphs?.[1]?.chords).toBeUndefined();
  });

  it("Enter inside a line shares its chords out between the halves (H06-4)", () => {
    const tb = {
      paragraphs: [
        {
          runs: [{ text: "Sing a song of joy" }],
          chords: [
            { at: 0, chord: "G" },
            { at: 7, chord: "C" },
            { at: 15, chord: "D" },
          ],
        },
      ],
    };
    // contenteditable clones the line's div, origin included: both halves say `from: 0`.
    const edited = [
      { from: 0, pieces: [{ text: "Sing a ", run: [0, 0] as [number, number] }] },
      { from: 0, pieces: [{ text: "song of joy", run: [0, 0] as [number, number] }] },
    ];
    const out = rebuildTextBox(tb, edited);
    expect(out.paragraphs?.[0]?.chords).toEqual([{ at: 0, chord: "G" }]);
    expect(out.paragraphs?.[1]?.chords).toEqual([
      { at: 0, chord: "C" },
      { at: 8, chord: "D" },
    ]);
    // Enter at the end of the line: the new empty line has none.
    const end = rebuildTextBox(tb, [
      { from: 0, pieces: [{ text: "Sing a song of joy", run: [0, 0] as [number, number] }] },
      { from: 0, pieces: [] },
    ]);
    expect(end.paragraphs?.[0]?.chords).toEqual(tb.paragraphs[0]!.chords);
    expect(end.paragraphs?.[1]?.chords).toBeUndefined();
  });

  it("a chord goes over the character the chart showed, or nowhere (H11-7)", () => {
    const tb = { paragraphs: [{ runs: [{ text: "Sing a song" }] }] };
    expect(chordOnLine(tb, 0, "Sing a song", 0, "G")?.paragraphs?.[0]?.chords).toEqual([{ at: 0, chord: "G" }]);
    // The chart was built before "Oh, " was typed in the in-place editor.
    const typed = { paragraphs: [{ runs: [{ text: "Oh, Sing a song" }] }] };
    expect(chordOnLine(typed, 0, "Sing a song", 0, "G")).toBeNull();
    expect(chordOnLine(typed, 1, "Sing a song", 0, "G")).toBeNull();
  });

  it("stage layouts offer and name chord charts", () => {
    expect(DATA_LINKS.map((d) => d.id)).toEqual(expect.arrayContaining(["chordChart", "nextChordChart", "songKey"]));
    expect(dataLinkLabel({ type: "chordChart", which: "next" })).toBe("Next Chord Chart");
    expect(dataLinkLabel({ type: "songKey" })).toBe("Song Key");
    const slide = { id: "s", elements: [{ id: "e", bounds: { x: 0, y: 0, width: 1, height: 1 }, dataLink: { type: "chordChart" as const } }] };
    expect(compositionText(slide)).toBe("Chords");
  });
});
