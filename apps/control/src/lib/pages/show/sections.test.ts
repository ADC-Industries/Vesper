import { describe, expect, it } from "vitest";
import type { PlaylistItemDetail, Presentation, SlideAction } from "$protocol";
import {
  arrangeSlides,
  atOccurrence,
  clampThumb,
  occurrenceMarks,
  playOrder,
  cuesBySlide,
  thumbVersions,
  inSection,
  librarySection,
  liveSectionKey,
  perRow,
  placeholderHeight,
  playlistCounts,
  playlistSections,
  presentationKind,
  sectionMeta,
  slideContextKey,
  summaryMeta,
  slideMarks,
  treeMove,
  type Position,
  type ShowSection,
  type TreeNodeLike,
} from "./sections";

const pres = (id: string, name: string, slideCount: number, item: string, arrangement?: string): PlaylistItemDetail => ({
  id: item,
  displayName: name,
  enabled: true,
  kind: { type: "presentation", presentation: { id, name }, ...(arrangement ? { arrangement } : {}) },
  presentation: { id, name, library: null, slideCount, destination: "slide" } as PlaylistItemDetail["presentation"],
});

const ITEMS: PlaylistItemDetail[] = [
  { id: "h1", displayName: "Worship", enabled: true, kind: { type: "header", name: "Worship", color: { r: 1, g: 0, b: 0 } }, presentation: null },
  pres("pA", "Song A", 4, "i1"),
  pres("pB", "Song B", 3, "i2", "arr-short"),
  { id: "m1", displayName: "Loop", enabled: true, kind: { type: "media", asset: "a1" }, presentation: null },
  { id: "x1", displayName: "Gone", enabled: true, kind: { type: "presentation", presentation: { id: "pX" } }, presentation: null },
  pres("pA", "Song A (again)", 4, "i3"),
];

const sections = playlistSections(ITEMS, () => "#f00");
const pos = (presentation: string, slide: string, item?: string): Position => ({
  presentation,
  slide,
  playlist: item ? { playlist: "pl", item } : null,
});

describe("playlist sections", () => {
  it("makes one section per item, in order", () => {
    expect(sections.map((s) => s.type)).toEqual(["header", "presentation", "presentation", "item", "missing", "presentation"]);
    expect(sections[0]).toMatchObject({ type: "header", name: "Worship", color: "#f00" });
    expect(sections[2]).toMatchObject({ presentationId: "pB", arrangement: "arr-short", slideCount: 3 });
    expect(sections[3]).toMatchObject({ kind: "media" });
    expect(new Set(sections.map((s) => s.key)).size).toBe(sections.length);
  });

  it("counts items and slides for the toolbar", () => {
    expect(playlistCounts(sections)).toEqual({ items: 5, slides: 11 });
  });

  it("a library item is its own section", () => {
    const s = librarySection({ id: "pZ", name: "Z", slideCount: 2 });
    expect(s).toMatchObject({ type: "presentation", itemId: null, presentationId: "pZ" });
    expect(inSection(s, pos("pZ", "s1", "i9"))).toBe(true);
  });
});

describe("live and next across sections", () => {
  const a1 = sections[1] as ShowSection;
  const a2 = sections[5] as ShowSection;
  const b = sections[2] as ShowSection;

  it("matches the playlist item, so a presentation in the playlist twice lights up once", () => {
    const live = pos("pA", "s2", "i3");
    expect(inSection(a1, live)).toBe(false);
    expect(inSection(a2, live)).toBe(true);
    expect(liveSectionKey(sections, live)).toBe("item:i3");
  });

  it("a slide triggered outside the playlist matches by presentation", () => {
    expect(liveSectionKey(sections, pos("pA", "s1"))).toBe("item:i1");
    expect(liveSectionKey(sections, null)).toBeNull();
  });

  it("marks NEXT in the following section when the cursor crosses the boundary", () => {
    const live = pos("pA", "s4", "i1");
    const cursor = pos("pB", "s1", "i2");
    expect(slideMarks(a1, "s4", true, live, cursor)).toEqual({ live: true, next: false });
    expect(slideMarks(b, "s1", true, live, cursor)).toEqual({ live: false, next: true });
    expect(slideMarks(a1, "s1", true, live, cursor)).toEqual({ live: false, next: false });
  });

  it("never marks a disabled or live slide as next", () => {
    const p = pos("pB", "s1", "i2");
    expect(slideMarks(b, "s1", false, null, p).next).toBe(false);
    expect(slideMarks(b, "s1", true, p, p)).toEqual({ live: true, next: false });
  });

  it("headers and media items are never live", () => {
    expect(inSection(sections[0]!, pos("pA", "s1"))).toBe(false);
    expect(inSection(sections[3]!, pos("pA", "s1"))).toBe(false);
  });
});

describe("sizes", () => {
  it("clamps and steps the pixel size", () => {
    expect(clampThumb(50)).toBe(120);
    expect(clampThumb(999)).toBe(420);
    expect(clampThumb(194)).toBe(190);
    expect(clampThumb(NaN)).toBe(190);
  });

  it("fits tiles per row", () => {
    expect(perRow(1000, 190, 14)).toBe(4);
    expect(perRow(100, 190, 14)).toBe(1);
    expect(perRow(0, 190, 14)).toBe(1);
  });

  it("estimates placeholder heights per view", () => {
    expect(placeholderHeight("grid", 0, 1000, 190)).toBe(0);
    const one = placeholderHeight("grid", 4, 1000, 190);
    const two = placeholderHeight("grid", 5, 1000, 190);
    expect(two).toBeGreaterThan(one * 1.9);
    expect(placeholderHeight("list", 10, 1000, 190)).toBe(310);
  });
});

describe("documents", () => {
  const doc: Presentation = {
    schemaVersion: 1,
    id: "p",
    name: "P",
    songInfo: { authors: "A. Writer", copyrightYear: 2011, ccliNumber: "123" },
    sections: [
      {
        id: "s",
        slides: [
          { id: "a", actions: [{ id: "c1", name: "Sunrise", kind: { type: "media", asset: "m1" } }, { id: "c2", kind: { type: "clearLayer" } }] },
          { id: "b", actions: [{ id: "c3", enabled: false, kind: { type: "media", asset: "m2" } }] },
        ],
      },
    ],
  } as Presentation;

  it("finds media cues per slide, skipping disabled ones and other actions", () => {
    const m = cuesBySlide(doc);
    expect(m.get("a")).toEqual([{ action: "c1", name: "Sunrise", asset: "m1" }]);
    expect(m.has("b")).toBe(false);
  });

  it("versions every thumbnail by the slide, every slide's cues and the presentation", () => {
    const v = thumbVersions(doc);
    expect(v.get("a")).not.toBe(v.get("b"));
    // A new cue on slide a changes slide b's picture too (its background comes from a).
    const slideA = (p: Presentation) => p.sections!.flatMap((x) => x.slides!).find((x) => x.id === "a")!;
    const cued = structuredClone(doc);
    slideA(cued).actions!.push({ id: "c4", kind: { type: "media", asset: "m3" } } as SlideAction);
    expect(thumbVersions(cued).get("b")).not.toBe(v.get("b"));
    // Text on slide a only changes slide a.
    const typed = structuredClone(doc);
    slideA(typed).elements = [];
    expect(thumbVersions(typed).get("a")).not.toBe(v.get("a"));
    expect(thumbVersions(typed).get("b")).toBe(v.get("b"));
    // The edit time alone changes nothing.
    expect(thumbVersions({ ...doc, modifiedAt: "2026-01-01T00:00:00Z" } as Presentation)).toEqual(v);
  });

  it("reads the kind and the header line", () => {
    expect(presentationKind(doc)).toBe("song");
    expect(presentationKind({ ...doc, songInfo: undefined })).toBe("text");
    expect(sectionMeta(doc)).toBe("A. Writer, 2011 · CCLI 123");
    expect(sectionMeta({ ...doc, songInfo: undefined, canvasSize: { width: 1440, height: 1080 } })).toBe("1440×1080");
  });
});

describe("moving playlists in the tree", () => {
  const tree: TreeNodeLike[] = [
    { id: "F", kind: "folder", children: [{ id: "p1", kind: "playlist", children: [] }, { id: "G", kind: "folder", children: [] }] },
    { id: "p2", kind: "playlist", children: [] },
    { id: "p3", kind: "playlist", children: [] },
  ];

  it("drops into a folder at its end", () => {
    expect(treeMove(tree, "p2", { into: "F" })).toEqual({ parent: "F", after: "G" });
    expect(treeMove(tree, "p3", { into: "G" })).toEqual({ parent: "G", after: null });
  });

  it("drops before a node", () => {
    expect(treeMove(tree, "p3", { before: "p2" })).toEqual({ parent: null, after: "F" });
    expect(treeMove(tree, "p2", { before: "p1" })).toEqual({ parent: "F", after: null });
    expect(treeMove(tree, "p1", { before: "p3" })).toEqual({ parent: null, after: "p2" });
  });

  it("moves to the top level", () => {
    expect(treeMove(tree, "p1", { root: true })).toEqual({ parent: null, after: "p3" });
    expect(treeMove(tree, "p3", { root: true })).toBeNull();
  });

  it("refuses a folder into itself and no-op moves", () => {
    expect(treeMove(tree, "F", { into: "G" })).toBeNull();
    expect(treeMove(tree, "F", { into: "F" })).toBeNull();
    expect(treeMove(tree, "p3", { before: "p3" })).toBeNull();
    expect(treeMove(tree, "p2", { before: "p3" })).toBeNull();
  });
});

describe("keys", () => {
  const k = (key: string, mods: Partial<KeyboardEvent> = {}) => ({ key, shiftKey: false, ctrlKey: false, altKey: false, metaKey: false, ...mods });
  it("opens the slide menu on the menu key and Shift+F10 only", () => {
    expect(slideContextKey(k("ContextMenu"))).toBe(true);
    expect(slideContextKey(k("F10", { shiftKey: true }))).toBe(true);
    expect(slideContextKey(k("F10"))).toBe(false);
    expect(slideContextKey(k("F10", { shiftKey: true, ctrlKey: true }))).toBe(false);
  });
});

describe("summaries (kind, authors, year, CCLI)", () => {
  it("carry the server's kind and a second line into the sections", () => {
    const it = pres("pS", "Song S", 2, "iS");
    it.presentation = { ...it.presentation!, kind: "media", authors: "A. Writer", copyrightYear: 2011, ccliNumber: "123" };
    const [sec] = playlistSections([it], () => null);
    expect(sec).toMatchObject({ kind: "media", meta: "A. Writer, 2011 · CCLI 123" });
    expect(librarySection({ id: "p", name: "P", slideCount: 1 })).toMatchObject({ kind: "text", meta: "" });
  });

  it("leave out what the server does not know", () => {
    expect(summaryMeta({})).toBe("");
    expect(summaryMeta({ copyrightYear: 1999 })).toBe("1999");
    expect(summaryMeta({ ccliNumber: "77" })).toBe("CCLI 77");
    expect(summaryMeta(null)).toBe("");
  });
});

describe("play order (a song with a repeated chorus)", () => {
  // V1 [v1a v1b], C [c1 c2], B [b1], V2 [v2a]; "Short" = V1 C V2 C (the bridge left out),
  // "Long" = C V1.
  const sec = (id: string, slides: string[]) => ({ id, name: id, slides: slides.map((s) => ({ id: s })) });
  const song = {
    sections: [sec("V1", ["v1a", "v1b"]), sec("C", ["c1", "c2"]), sec("B", ["b1"]), sec("V2", ["v2a"])],
    arrangements: [
      { id: "short", name: "Short", sections: ["V1", "C", "V2", "C"] },
      { id: "long", name: "Long", sections: ["C", "V1", "gone"] },
      { id: "empty", name: "Empty", sections: ["gone"] },
    ],
    selectedArrangement: "long",
  } as unknown as Presentation;
  const slides = ["v1a", "v1b", "c1", "c2", "b1", "v2a"].map((id, index) => ({ id, index, enabled: id !== "c1" }));
  const s = librarySection({ id: "song", name: "Song", slideCount: 6 });
  const at = (slide: string, step?: number): Position => ({ presentation: "song", slide, playlist: null, step });

  it("follows the item's arrangement, else the song's own, else document order", () => {
    expect(playOrder(song, "short")).toEqual([0, 1, 2, 3, 5, 2, 3]);
    expect(playOrder(song, null)).toEqual([2, 3, 0, 1]);
    expect(playOrder(song, "missing")).toEqual([2, 3, 0, 1]);
    expect(playOrder({ ...song, selectedArrangement: undefined }, null)).toBeNull();
    expect(playOrder({ ...song, selectedArrangement: undefined }, "empty")).toBeNull();
  });

  it("shows repeats as their own tiles, then the groups left out", () => {
    const a = arrangeSlides(slides, playOrder(song, "short"));
    expect(a.played.map((o) => `${o.n}:${o.slide.id}`)).toEqual(["1:v1a", "2:v1b", "3:c1", "4:c2", "5:v2a", "6:c1", "7:c2"]);
    expect(new Set(a.played.map((o) => o.key)).size).toBe(7);
    expect(a.left.map((o) => [o.slide.id, o.step, o.n])).toEqual([["b1", null, null]]);
    expect(a.ids).toEqual(["v1a", "v1b", "c1", "c2", "v2a", "c1", "c2"]);
    // No arrangement: document order, nothing left out, steps are the document indices.
    const d = arrangeSlides(slides, null);
    expect(d.played.map((o) => o.step)).toEqual([0, 1, 2, 3, 4, 5]);
    expect(d.left).toEqual([]);
  });

  it("marks the live occurrence only, and the next one the server reports", () => {
    const a = arrangeSlides(slides, playOrder(song, "short"));
    const marks = (live: Position | null, cursor: Position | null) =>
      [...a.played, ...a.left].map((o) => {
        const m = occurrenceMarks(s, o.slide.id, o.slide.enabled, o.step, a.ids, live, cursor);
        return m.live ? "L" : m.next ? "N" : ".";
      }).join("");
    // The second chorus is live (step 6 = c2), nothing after it in the song.
    expect(marks(at("c2", 6), null)).toBe("......L.");
    // The first chorus's c2 is live, the cursor is v2a.
    expect(marks(at("c2", 3), at("v2a", 4))).toBe("...LN...");
    // A disabled slide is never NEXT.
    expect(marks(null, at("c1", 5))).toBe("........");
    // Without a step (an older server), the first occurrence.
    expect(marks(at("c2"), null)).toBe("...L....");
    // On the announcements layer: LIVE too (X3-8), and the slide layer's slide stays LIVE.
    const ann = (live: Position | null, announcement: Position | null, cursor: Position | null) =>
      [...a.played, ...a.left].map((o) => {
        const m = occurrenceMarks(s, o.slide.id, o.slide.enabled, o.step, a.ids, live, cursor, announcement);
        return m.live ? "L" : m.next ? "N" : ".";
      }).join("");
    expect(ann(null, at("c2", 3), at("v2a", 4))).toBe("...LN...");
    expect(ann(at("c2", 6), at("c2", 3), null)).toBe("...L..L.");
    // A step from another order (it holds another slide): the first occurrence too.
    expect(atOccurrence(at("c2", 0), "c2", 3, a.ids)).toBe(true);
    expect(atOccurrence(at("c2", 0), "c2", 6, a.ids)).toBe(false);
    // A left-out slide triggered by hand.
    expect(marks(at("b1"), at("v2a", 4))).toBe("....N..L");
  });
});
