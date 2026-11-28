import { describe, expect, it } from "vitest";
import type { Presentation, SongSelectParsed } from "$protocol";
import { importLabel, importPlan, importSummary, itemFor, looksLikeSongSelect, replaceOps, sectionNames, toBase64 } from "./songselect";

// Made-up songs only.
const LYRICS = "Lanterns Over Kettle Bay\n\nVerse 1\nLanterns over Kettle Bay\n\nAlex Example | Jamie Sample\nCCLI Song # 1234567\n© 2019 Example Songs\nFor use solely with the SongSelect® Terms of Use. All rights reserved. www.ccli.com\nCCLI License # 7654321\n";

const pres = (id: string, sections: string[]): Presentation =>
  ({ schemaVersion: 2, id, name: `Song ${id}`, canvasSize: { width: 1920, height: 1080 }, sections: sections.map((s, i) => ({ id: `${id}-s${i}`, name: s, slides: [] })) }) as unknown as Presentation;

const parsed = (id: string, dup = false): SongSelectParsed =>
  ({ format: "lyrics", presentation: pres(id, ["Verse 1", "Chorus"]), sections: 2, slides: 2, chordLines: 0, ...(dup ? { duplicates: [{ id: "old", name: "Old Copy" }] } : {}) }) as SongSelectParsed;

describe("SongSelect import", () => {
  it("recognises SongSelect lyrics", () => {
    expect(looksLikeSongSelect(LYRICS)).toBe(true);
    expect(looksLikeSongSelect("Verse 1\nOnly words here\n")).toBe(false);
    expect(looksLikeSongSelect("CCLI Song No. 1234567")).toBe(true);
    expect(looksLikeSongSelect("Line about the CCLI Song # 12 inside a sentence")).toBe(false);
  });

  it("encodes bytes as base64", () => {
    expect(toBase64(new TextEncoder().encode("Café"))).toBe("Q2Fmw6k=");
    expect(toBase64(new Uint8Array(70000).fill(65)).length).toBe(93336);
  });

  it("skips duplicates unless told otherwise", () => {
    const a = itemFor("a", "a.txt", parsed("a"), null);
    const b = itemFor("b", "b.txt", parsed("b", true), null);
    const bad = itemFor("c", "c.txt", null, "no lyrics found in the song");
    expect(b.action).toBe("skip");
    expect(importPlan([a, b, bad]).map((p) => p.kind)).toEqual(["create", "skip"]);
    expect(importLabel([a, b, bad])).toBe("Import 1 song");
    expect(importPlan([a, { ...b, action: "replace" }])[1]).toMatchObject({ kind: "replace", target: "old" });
    expect(importLabel([a, { ...b, action: "keep" }])).toBe("Import 2 songs");
    expect(importLabel([])).toBe("Import");
    expect(sectionNames(a.parsed!.presentation)).toBe("Verse 1, Chorus");
  });

  it("replaces a song's words and metadata in place", () => {
    const old = pres("old", ["Verse 1", "Verse 2"]);
    const next = { ...pres("new", ["Verse 1", "Chorus"]), songInfo: { ccliNumber: "1234567", display: true } } as Presentation;
    const ops = replaceOps(old, next);
    expect(ops.map((o) => o.type)).toEqual(["deleteSection", "deleteSection", "addSection", "addSection", "setArrangements", "setPresentation"]);
    expect(ops[3]).toMatchObject({ type: "addSection", after: "new-s0" });
    expect(ops[2]).not.toHaveProperty("after");
    expect(ops[5]).toMatchObject({ songInfo: { ccliNumber: "1234567" } });
  });

  it("sums up an import", () => {
    expect(importSummary(2, 1, 1, 0)).toBe("SongSelect: 2 songs imported, 1 song replaced, 1 duplicate skipped.");
    expect(importSummary(0, 0, 0, 0)).toBe("SongSelect: nothing imported.");
  });
});
