import { describe, expect, it } from "vitest";
import type { CodexStatus, Presentation, ShowState } from "$protocol";
import {
  overrideLabel,
  slideOverrides,
  songThemeState,
  compareTitles,
  connectionCard,
  daysLeft,
  formatBytes,
  formatDuration,
  liveSummary,
  matchesFilter,
  matchReportLines,
  mediaKind,
  railIndex,
  railLetter,
  railTarget,
  relinkReportLines,
  titleKey,
} from "./logic";

describe("titles and the A–Z rail", () => {
  it("sorts by title, ignoring case, accents and leading quotes", () => {
    const names = ["“Open Gate”", "copper Sky", "Ébène Road", "Psalm 10", "Psalm 9", "12 Stones"];
    expect([...names].sort(compareTitles)).toEqual(["12 Stones", "copper Sky", "Ébène Road", "“Open Gate”", "Psalm 9", "Psalm 10"]);
    expect(titleKey("  ‘Árbol’")).toBe("arbol’");
  });

  it("files titles under a letter, digits and others under #", () => {
    expect(railLetter("Éclair")).toBe("E");
    expect(railLetter("“Open Gate”")).toBe("O");
    expect(railLetter("12 Stones")).toBe("#");
    expect(railLetter("Ωmega")).toBe("#");
  });

  it("jumps to a letter's first row, or the next letter that has rows", () => {
    const rows = ["12 Stones", "Amber Hill", "Apple Lane", "Copper Sky", "Willow Steps"];
    const idx = railIndex(rows, (r) => r);
    expect([...idx.entries()]).toEqual([
      ["#", 0],
      ["A", 1],
      ["C", 3],
      ["W", 4],
    ]);
    expect(railTarget(idx, "A")).toBe(1);
    expect(railTarget(idx, "B")).toBe(3);
    expect(railTarget(idx, "X")).toBeNull();
    // Sorted Z to A: the rail looks the other way.
    const desc = [...rows].reverse();
    const di = railIndex(desc, (r) => r);
    expect(railTarget(di, "B", true)).toBe(di.get("A")!);
  });

  it("filters by the starts of words", () => {
    expect(matchesFilter("Lantern Road", "lan ro")).toBe(true);
    expect(matchesFilter("Lantern Road", "antern")).toBe(false);
    expect(matchesFilter("Éclair Song", "ecl")).toBe(true);
    expect(matchesFilter("Anything", "  ")).toBe(true);
  });
});

describe("what is live", () => {
  const base = { layers: { videoInput: null, media: null, foreground: null, slide: null, announcements: null, props: [], messages: [], audio: null } };
  const st = (layers: object) => ({ ...base, layers: { ...base.layers, ...layers } }) as unknown as ShowState;

  it("names the live layers", () => {
    expect(liveSummary(null)).toBeNull();
    expect(liveSummary(st({}))).toBeNull();
    expect(liveSummary(st({ slide: { presentationName: "Copper Sky" } }))).toBe("a slide of “Copper Sky”");
    expect(liveSummary(st({ slide: { presentationName: "" }, props: [{}], audio: {} }))).toBe("a slide, a prop and audio");
    expect(liveSummary(st({ media: {}, messages: [{}, {}] }))).toBe("background media and 2 messages");
  });
});

describe("the connection card", () => {
  const codex = (o: Partial<CodexStatus>) =>
    ({ id: "c", role: "host", collection: "a", collectionName: "Main Hall", collections: 2, documents: 9, trash: 0, deletedCollections: 0, mediaLibrary: { available: true, served: true }, ...o }) as CodexStatus;
  const sync = { reachable: true, seq: 4, codexSeq: 4, behind: 0, media: { pinned: 0, cached: 0, copying: 0, missing: 0, failed: 0, bytes: 0, capBytes: 0 } };

  it("says where the Codex is and whether editing works", () => {
    expect(connectionCard(codex({}), true, "Booth")).toMatchObject({ status: "ok", title: "Connected", readOnly: false });
    expect(connectionCard(codex({ role: "client", sync }), true, "Booth")).toMatchObject({ status: "ok", readOnly: false });
    const away = connectionCard(codex({ role: "client", sync: { ...sync, reachable: false } }), true, "Booth");
    expect(away).toMatchObject({ status: "warn", title: "Codex unreachable", readOnly: true });
    expect(away.detail).toMatch(/^Editing needs the site's server\. The show keeps playing/);
    expect(connectionCard(codex({ role: "dormant" }), true, "Booth").readOnly).toBe(true);
    expect(connectionCard(codex({}), false, "Booth")).toMatchObject({ status: "error", title: "Not connected" });
    expect(connectionCard(null, true, "Booth").status).toBe("off");
  });
});

describe("formatters", () => {
  it("formats sizes, durations and days", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(48_000_000)).toBe("48 MB");
    expect(formatBytes(1_250_000_000)).toBe("1.3 GB");
    expect(formatDuration(30)).toBe("0:30");
    expect(formatDuration(3723)).toBe("1:02:03");
    expect(daysLeft("2026-11-30T00:00:00Z", Date.parse("2026-11-23T00:00:00Z"))).toBe(7);
    expect(daysLeft("2020-01-01T00:00:00Z")).toBe(0);
  });
});

describe("relink reports", () => {
  it("say what was found", () => {
    expect(relinkReportLines({ checked: 0, relinked: [], ambiguous: [], notFound: [], statusChanged: 0 })).toEqual(["No files are missing."]);
    const loc = { type: "unresolved" as const };
    expect(
      relinkReportLines({ checked: 3, relinked: [{ asset: "a", name: "A", method: "contentHash", location: loc }], ambiguous: [], notFound: ["b", "c"], statusChanged: 1 }),
    ).toEqual(["1 file was found and linked again.", "2 files weren't found. Try “Match a folder on this computer”."]);
    expect(matchReportLines({ matches: [], ambiguous: ["x"], unmatched: ["y"] })).toEqual([
      "None of the missing files are in that folder.",
      "1 file matches more than one file there.",
      "1 file is still missing.",
    ]);
  });
});

describe("media kinds", () => {
  it("reads a missing kind as an image, as the server leaves it out", () => {
    expect(mediaKind({})).toBe("image");
    expect(mediaKind({ kind: "video" })).toBe("video");
    expect(mediaKind({ kind: "audio" })).toBe("audio");
  });
});

describe("linked themes", () => {
  const origin = { theme: { id: "t1", name: "Evening" }, slide: "tpl" };
  const doc = {
    id: "p",
    name: "Song",
    templateOrigin: { theme: { id: "old", name: "Imported Look" }, slide: "x" },
    sections: [
      {
        id: "s",
        name: "Verse",
        slides: [
          { id: "a", templateOrigin: origin, themeLink: {}, elements: [{ id: "e1", fromTheme: { element: "te" } }] },
          { id: "b", templateOrigin: origin, themeLink: { overrides: ["background", "removed:te2"] }, elements: [{ id: "e2", fromTheme: { element: "te", overrides: ["bounds"] } }] },
          { id: "c", elements: [] },
        ],
      },
    ],
  } as unknown as Presentation;

  it("tells which theme a song follows and which slides override it", () => {
    const st = songThemeState(doc);
    expect(st.theme).toBe("t1");
    expect(st.linked).toBe(2);
    expect(st.total).toBe(3);
    expect(st.overridden).toEqual(["b"]);
    expect(st.madeFrom).toBe("Imported Look");
  });

  it("names the overrides of a slide and of one object", () => {
    const slides = doc.sections![0]!.slides!;
    const b = slides[1]!;
    expect(slideOverrides(b)).toEqual(["background", "bounds", "removed"]);
    expect(slideOverrides(b, "e2")).toEqual(["bounds"]);
    expect(slideOverrides(slides[2]!)).toEqual([]);
    expect(overrideLabel("bounds")).toBe("Position and size");
    expect(overrideLabel("text.defaultStyle")).toBe("Font and colour");
    expect(overrideLabel("somethingNew")).toBe("somethingNew");
  });

  it("an unlinked song has no theme, only where it was copied from", () => {
    const plain = { id: "p", name: "x", sections: [{ id: "s", slides: [{ id: "a", templateOrigin: origin }] }] } as unknown as Presentation;
    expect(songThemeState(plain)).toMatchObject({ theme: null, linked: 0, madeFrom: "Evening" });
  });
});
