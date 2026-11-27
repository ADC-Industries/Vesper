import { describe, expect, it } from "vitest";
import type { MediaBinNode, PlaylistTreeNode } from "$protocol";
import { addToBinOps, addToPlaylistOps, binDropTarget, binLabel, binMove, colorFromHex, groupMove, hexOf, listMove, mediaBinRows, mediaPlaylists, nameOk, newBinItem } from "./bins";
import { parseDropTarget } from "./binDrag.svelte";

const bin = (id: string, name: string, path: string[], extra: Partial<MediaBinNode> = {}): MediaBinNode => ({
  id,
  name,
  path,
  kind: "manual",
  browser: "media",
  itemCount: 1,
  children: [],
  ...extra,
});

const TREE: MediaBinNode[] = [
  {
    id: null,
    name: "Visuals",
    path: ["Visuals"],
    kind: "folder",
    browser: "media",
    itemCount: 3,
    children: [bin("a", "Backgrounds", ["Visuals"]), bin("b", "Loops", ["Visuals"], { kind: "watched" })],
  },
  bin("c", "Stills", []),
  bin("d", "Walk-in Music", [], { browser: "audio" }),
  bin("e", "Countdown", []),
];

describe("media bin rows", () => {
  it("lists the media bins in display order, without audio bins", () => {
    const rows = mediaBinRows(TREE);
    expect(rows.map((r) => r.id)).toEqual(["a", "b", "c", "e"]);
    expect(rows[1]!.kind).toBe("watched");
    expect(binLabel(rows[0]!)).toBe("Visuals › Backgrounds");
    expect(binLabel(rows[2]!)).toBe("Stills");
  });

  it("moves a bin within its folder only", () => {
    const rows = mediaBinRows(TREE);
    expect(binMove(rows, "a", -1)).toBeNull();
    expect(binMove(rows, "a", 1)).toEqual({ after: "b" });
    expect(binMove(rows, "b", -1)).toEqual({ after: null });
    expect(binMove(rows, "b", 1)).toBeNull();
    expect(binMove(rows, "e", -1)).toEqual({ after: null });
    expect(binMove(rows, "c", 1)).toEqual({ after: "e" });
    expect(binMove(rows, "zz", 1)).toBeNull();
  });

  it("moves in a flat list", () => {
    expect(listMove(["x", "y", "z"], "z", -1)).toEqual({ after: "x" });
    expect(listMove(["x", "y", "z"], "x", 1)).toEqual({ after: "y" });
    expect(groupMove([{ id: "x" }, { id: "y" }], "y", -1)).toEqual({ after: null });
    expect(groupMove([{ id: "x" }, { id: "y" }], "y", 1)).toBeNull();
  });
});

describe("adding media", () => {
  let n = 0;
  const ids = () => `id${++n}`;

  it("appends new files after the last item, skipping ones the bin holds", () => {
    n = 0;
    const ops = addToBinOps(
      [{ id: "i1", asset: "f1" }],
      [
        { asset: "f1", kind: "image" },
        { asset: "f2", kind: "video" },
        { asset: "f2", kind: "video" },
        { asset: "f3", kind: "audio" },
      ],
      ids,
    );
    expect(ops).toEqual([
      { type: "addItem", after: "i1", item: { id: "id1", trigger: { asset: "f2", layer: "background" } } },
      { type: "addItem", after: "id1", item: { id: "id2", trigger: { asset: "f3", layer: "audio" } } },
    ]);
    expect(addToBinOps([], [{ asset: "f1" }], ids)[0]).toEqual({ type: "addItem", item: { id: "id3", trigger: { asset: "f1", layer: "background" } } });
    expect(addToBinOps([{ id: "i1", asset: "f1" }], [{ asset: "f1" }], ids)).toEqual([]);
  });

  it("appends media items to a playlist", () => {
    n = 0;
    expect(addToPlaylistOps([{ id: "p1" }], [{ asset: "f1", kind: "video" }], ids)).toEqual([
      { type: "addItem", after: "p1", item: { id: "id1", kind: { type: "media", asset: "f1", layer: "background" } } },
    ]);
    expect(newBinItem("f9", "image", "x").trigger.layer).toBe("background");
  });
});

describe("media playlists and names", () => {
  it("finds the media playlists in the tree", () => {
    const tree: PlaylistTreeNode[] = [
      { id: "s", name: "Sunday", kind: "playlist", purpose: "service", itemCount: 3, children: [] },
      {
        id: "f",
        name: "Loops",
        kind: "folder",
        purpose: null,
        itemCount: 0,
        children: [{ id: "m", name: "Pre-service", kind: "playlist", purpose: "media", itemCount: 4, children: [] }],
      },
    ];
    expect(mediaPlaylists(tree)).toEqual([{ id: "m", name: "Pre-service", path: ["Loops"], itemCount: 4 }]);
  });

  it("checks names as the server does", () => {
    expect(nameOk(" Backgrounds ")).toBe(true);
    expect(nameOk("   ")).toBe(false);
    expect(nameOk("Tab\there")).toBe(false);
    expect(nameOk("x".repeat(200))).toBe(true);
    expect(nameOk("x".repeat(201))).toBe(false);
  });

  it("converts colours and drop targets", () => {
    expect(colorFromHex("#ff8000")).toEqual({ r: 1, g: 128 / 255, b: 0 });
    expect(colorFromHex("red")).toBeNull();
    expect(hexOf({ r: 1, g: 128 / 255, b: 0 })).toBe("#ff8000");
    expect(hexOf(null)).toBe("#808080");
    expect(parseDropTarget("bin:abc")).toEqual({ kind: "bin", id: "abc" });
    expect(parseDropTarget("playlist:p")).toEqual({ kind: "playlist", id: "p" });
    expect(parseDropTarget("other")).toBeNull();
    expect(parseDropTarget(undefined)).toBeNull();
  });
});

describe("bin drop targets (D09-4)", () => {
  it("only manual bins take dropped media", () => {
    expect(binDropTarget({ id: "b", name: "Backgrounds", path: [], kind: "manual", itemCount: 0 })).toBe("bin:b");
    expect(binDropTarget({ id: "w", name: "Watched", path: [], kind: "watched", itemCount: 3 })).toBeUndefined();
  });
});
