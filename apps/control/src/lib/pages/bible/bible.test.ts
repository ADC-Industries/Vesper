import { describe, expect, it } from "vitest";
import {
  clickVerse,
  parseRecent,
  parseSplit,
  previewParams,
  pushRecent,
  referenceFromSelection,
  selectAll,
  slideRequest,
  parseSecondLayout,
  verseKey,
} from "./bible";

const chapter = (c: number, n: number) => Array.from({ length: n }, (_, i) => ({ chapter: c, verse: i + 1 }));

describe("reference from a selection", () => {
  it("joins runs with an en dash and commas, in order", () => {
    expect(referenceFromSelection("Psalms", ["23:6", "23:2", "23:1", "23:3", "23:4"])).toBe("Psalm 23:1–4, 6");
    expect(referenceFromSelection("John", ["3:16"])).toBe("John 3:16");
    expect(referenceFromSelection("John", ["3:16", "3:17", "3:18"])).toBe("John 3:16–18");
    expect(referenceFromSelection("Genesis", ["1:1", "1:3", "1:5"])).toBe("Genesis 1:1, 3, 5");
  });

  it("writes a new chapter as chapter:verse", () => {
    expect(referenceFromSelection("John", ["3:35", "3:36", "4:1", "4:2"])).toBe("John 3:35–36, 4:1–2");
  });

  it("is empty for no selection and ignores bad keys", () => {
    expect(referenceFromSelection("John", [])).toBe("");
    expect(referenceFromSelection("John", ["x", "0:1"])).toBe("");
  });
});

describe("verse selection", () => {
  const verses = chapter(23, 6);

  it("toggles a verse and moves the anchor", () => {
    let r = clickVerse(verses, new Set(), null, 1, false);
    expect([...r.selected]).toEqual(["23:2"]);
    expect(r.anchor).toBe(1);
    r = clickVerse(verses, r.selected, r.anchor, 1, false);
    expect(r.selected.size).toBe(0);
  });

  it("selects a range with shift, both directions, keeping the rest", () => {
    let r = clickVerse(verses, new Set(["23:6"]), 1, 3, true);
    expect([...r.selected].sort()).toEqual(["23:2", "23:3", "23:4", "23:6"]);
    expect(r.anchor).toBe(1);
    r = clickVerse(verses, new Set(), 4, 2, true);
    expect([...r.selected].sort()).toEqual(["23:3", "23:4", "23:5"]);
  });

  it("shift without an anchor toggles", () => {
    const r = clickVerse(verses, new Set(), null, 2, true);
    expect([...r.selected]).toEqual(["23:3"]);
  });

  it("selects all", () => {
    expect(selectAll(verses).size).toBe(6);
    expect(verseKey({ chapter: 3, verse: 16 })).toBe("3:16");
  });
});

describe("slide options", () => {
  it("builds the request and the preview parameters", () => {
    const req = slideRequest("TST", "Psalm 23:1–4", { split: "2", showReference: true, verseNumbers: false, theme: "th" });
    expect(req).toEqual({ translation: "TST", reference: "Psalm 23:1–4", verseNumbers: false, referenceOn: "each", versesPerSlide: 2, theme: "th" });
    const auto = slideRequest("TST", "John 3:16", { split: "auto", showReference: false, verseNumbers: true, theme: null });
    expect(auto).toEqual({ translation: "TST", reference: "John 3:16", verseNumbers: true, referenceOn: "none" });
    expect(previewParams(req, 1, 320)).toEqual({
      translation: "TST",
      reference: "Psalm 23:1–4",
      verseNumbers: "false",
      referenceOn: "each",
      versesPerSlide: 2,
      theme: "th",
      slide: 1,
      w: 320,
    });
  });

  it("asks for a second translation (parallel Bible) unless it is the first", () => {
    const base = { split: "auto" as const, showReference: true, verseNumbers: false, theme: null };
    const req = slideRequest("TST", "John 3:16", { ...base, second: "TS2", secondLayout: "stacked" });
    expect(req.secondTranslation).toBe("TS2");
    expect(req.secondLayout).toBe("stacked");
    expect(previewParams(req, 0, 160)).toMatchObject({ secondTranslation: "TS2", secondLayout: "stacked" });
    // Side by side is the server's default: not sent.
    const side = slideRequest("TST", "John 3:16", { ...base, second: "TS2", secondLayout: "sideBySide" });
    expect(side.secondLayout).toBeUndefined();
    expect(slideRequest("TST", "John 3:16", { ...base, second: "TST" }).secondTranslation).toBeUndefined();
    expect(parseSecondLayout("stacked")).toBe("stacked");
    expect(parseSecondLayout(null)).toBe("sideBySide");
  });

  it("reads the split preference", () => {
    expect(parseSplit("2")).toBe("2");
    expect(parseSplit(null)).toBe("auto");
    expect(parseSplit("7")).toBe("auto");
  });
});

describe("recent references", () => {
  it("keeps the newest first without duplicates, at most eight", () => {
    let list = pushRecent([], { translation: "TST", reference: "John 3:16" });
    list = pushRecent(list, { translation: "TST", reference: "Psalm 23" });
    list = pushRecent(list, { translation: "TST", reference: "john 3:16" });
    expect(list.map((r) => r.reference)).toEqual(["john 3:16", "Psalm 23"]);
    list = pushRecent(list, { translation: "SYN", reference: "Psalm 23" });
    expect(list).toHaveLength(3);
    for (let i = 0; i < 10; i++) list = pushRecent(list, { translation: "TST", reference: `Genesis ${i + 1}` });
    expect(list).toHaveLength(8);
    expect(list[0]!.reference).toBe("Genesis 10");
  });

  it("reads the saved list, ignoring junk", () => {
    expect(parseRecent(null)).toEqual([]);
    expect(parseRecent("nope")).toEqual([]);
    expect(parseRecent('[{"translation":"TST","reference":"John 1"},{"x":1},3]')).toEqual([{ translation: "TST", reference: "John 1" }]);
  });
});
