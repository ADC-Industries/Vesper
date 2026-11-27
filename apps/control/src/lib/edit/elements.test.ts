import { describe, expect, it } from "vitest";
import type { FillKind, Slide } from "$protocol";
import { copyElements, elementName, fillDrawn, fillKindPatch, mediaFillWith, elementType, newScrollingTextElement, newShapeElement, newTextElement, newVideoElement, pasteElements, PASTE_OFFSET } from "./elements";

let n = 0;
const newId = () => `new-${++n}`;
const canvas = { width: 1920, height: 1080 };

const slide: Slide = {
  id: "s1",
  elements: [
    { id: "e1", name: "Box", bounds: { x: 10, y: 20, width: 100, height: 50 }, kind: { type: "shape", shape: { type: "rectangle" } } },
    newTextElement("e2", canvas, { fontFamily: "Carlito", size: 100 }),
    { id: "e3", bounds: { x: 0, y: 0, width: 5, height: 5 } },
  ],
};

describe("element clipboard", () => {
  it("copies in z-order and pastes with fresh IDs, offset on the same slide", () => {
    const clip = copyElements(slide, ["e2", "e1"]);
    expect(clip.elements.map((e) => e.id)).toEqual(["e1", "e2"]);
    const pasted = pasteElements(clip, slide, newId);
    expect(pasted.map((e) => e.id)).toEqual(["new-1", "new-2"]);
    expect(new Set(pasted.map((e) => e.id)).size).toBe(2);
    expect(pasted[0]!.bounds).toEqual({ x: 10 + PASTE_OFFSET, y: 20 + PASTE_OFFSET, width: 100, height: 50 });
    // The originals are not touched.
    expect(slide.elements![0]!.id).toBe("e1");
    expect(slide.elements![0]!.bounds!.x).toBe(10);
  });
  it("pastes onto another slide in place", () => {
    const clip = copyElements(slide, ["e1"]);
    const other: Slide = { id: "s2", elements: [] };
    const p = pasteElements(clip, other, newId);
    expect(p[0]!.bounds!.x).toBe(10);
    expect(p[0]!.id).not.toBe("e1");
  });
});

describe("new elements", () => {
  it("are centred on the canvas", () => {
    const t = newTextElement("t", canvas, {});
    expect(t.bounds!.x! + t.bounds!.width! / 2).toBe(960);
    const line = newShapeElement("l", "line", canvas);
    expect(elementType(line)).toBe("line");
    expect(elementName(newShapeElement("x", "ellipse", canvas))).toBe("Ellipse");
    expect(elementType(slide.elements![1]!)).toBe("text");
    expect(elementName(slide.elements![2]!)).toBe("Object");
  });
});

describe("scrolling text", () => {
  it("is a full-width ticker near the bottom, scrolling with the server's defaults", () => {
    const e = newScrollingTextElement("s", { width: 1920, height: 1080 }, {});
    expect(e.scroll).toEqual({ enabled: true });
    expect(e.bounds).toEqual({ x: 0, y: 1080 - 162, width: 1920, height: 108 });
    expect(e.kind?.type).toBe("text");
  });
});

describe("video element", () => {
  it("is a looping media fill at the video's aspect ratio", () => {
    const e = newVideoElement("v", "a1", "", { width: 1920, height: 1080 }, { width: 640, height: 480 });
    expect(e.name).toBe("Video");
    expect(e.fill).toEqual({ enabled: true, kind: { type: "media", asset: "a1", display: { scale: "fit" }, playback: { repeat: { type: "loop" } } } });
    expect(e.bounds!.width! / e.bounds!.height!).toBeCloseTo(4 / 3);
  });
});

describe("media fill chooser (D10-4)", () => {
  it("keeps the display and a video's playback when the clip is swapped", () => {
    const cur = { enabled: true, kind: { type: "media", asset: "v1", display: { scale: "fill" }, playback: { repeat: { type: "once" } } } } as const;
    expect(mediaFillWith(cur as never, "v2", "video")).toEqual({
      enabled: true,
      kind: { type: "media", asset: "v2", display: { scale: "fill" }, playback: { repeat: { type: "once" } } },
    });
    // An image has no playback; a new video loops.
    expect(mediaFillWith(cur as never, "i1", "image")).toEqual({ enabled: true, kind: { type: "media", asset: "i1", display: { scale: "fill" } } });
    expect(mediaFillWith(undefined, "v3", "video")).toEqual({
      enabled: true,
      kind: { type: "media", asset: "v3", display: { scale: "fit" }, playback: { repeat: { type: "loop" } } },
    });
  });

  it("keeps a video's sound, and drops what only fits the old file (E10-2)", () => {
    const cur = {
      enabled: true,
      kind: {
        type: "media",
        asset: "v1",
        display: { scale: "fill", flip: { horizontal: true }, crop: { top: 10, left: 0, bottom: 0, right: 0 }, rotation: "cw90" },
        playback: { repeat: { type: "once" }, inPoint: 4, outPoint: 9 },
        audio: { muted: true, volumeDb: -6 },
      },
    } as const;
    // A muted video stays muted with the new clip; the old clip's crop, rotation and range go.
    expect(mediaFillWith(cur as never, "v2", "video")).toEqual({
      enabled: true,
      kind: { type: "media", asset: "v2", display: { scale: "fill", flip: { horizontal: true } }, playback: { repeat: { type: "once" } }, audio: { muted: true, volumeDb: -6 } },
    });
    // An image has no sound.
    expect(mediaFillWith(cur as never, "i1", "image")).toEqual({ enabled: true, kind: { type: "media", asset: "i1", display: { scale: "fill", flip: { horizontal: true } } } });
  });
});

type MediaFill = Extract<FillKind, { type: "media" }>;
type LiveFill = Extract<FillKind, { type: "liveSource" }>;

describe("fill settings with several objects selected (E10-3)", () => {
  const video = (id: string, asset: string) => ({ id, bounds: { x: 0, y: 0, width: 10, height: 10 }, fill: { enabled: true, kind: { type: "media", asset, display: { scale: "fit" } } } }) as never;
  const camera = (id: string, source: string) => ({ id, bounds: { x: 0, y: 0, width: 10, height: 10 }, fill: { enabled: true, kind: { type: "liveSource", source, display: { scale: "fit" } } } }) as never;
  const colour = { id: "c", bounds: { x: 0, y: 0, width: 10, height: 10 }, fill: { enabled: true, kind: { type: "color", color: { r: 1, g: 0, b: 0 } } } } as never;

  it("mutes each video on its own clip and leaves other fills alone", () => {
    const mute = (k: MediaFill): MediaFill => ({ ...k, audio: { muted: true } });
    expect(fillKindPatch(video("a", "clipA"), "media", mute)).toEqual({ fill: { enabled: true, kind: { type: "media", asset: "clipA", display: { scale: "fit" }, audio: { muted: true } } } });
    expect(fillKindPatch(video("b", "clipB"), "media", mute)).toEqual({ fill: { enabled: true, kind: { type: "media", asset: "clipB", display: { scale: "fit" }, audio: { muted: true } } } });
    expect(fillKindPatch(colour, "media", mute)).toEqual({});
  });

  it("rescales each camera box on its own source", () => {
    const fill = (k: LiveFill): LiveFill => ({ ...k, display: { scale: "fill" } });
    expect(fillKindPatch(camera("a", "CAM (1)"), "liveSource", fill)).toEqual({ fill: { enabled: true, kind: { type: "liveSource", source: "CAM (1)", display: { scale: "fill" } } } });
    expect(fillKindPatch(camera("b", "CAM (2)"), "liveSource", fill)).toEqual({ fill: { enabled: true, kind: { type: "liveSource", source: "CAM (2)", display: { scale: "fill" } } } });
    expect(fillKindPatch(video("v", "clipA"), "liveSource", fill)).toEqual({});
  });

  it("leaves a fill that is switched off switched off (F11-2)", () => {
    // As the server sends a switched-off fill: no `enabled` at all (H09-1).
    const mute = (k: MediaFill): MediaFill => ({ ...k, audio: { muted: true } });
    const hidden = { id: "h", bounds: { x: 0, y: 0, width: 10, height: 10 }, fill: { kind: { type: "media", asset: "clipB", display: { scale: "fit" } } } } as never;
    const patched = fillKindPatch(hidden, "media", mute);
    expect(patched).toEqual({ fill: { kind: { type: "media", asset: "clipB", display: { scale: "fit" }, audio: { muted: true } } } });
    expect(fillDrawn(patched.fill)).toBe(false);
    const hiddenCam = { id: "hc", bounds: { x: 0, y: 0, width: 10, height: 10 }, fill: { kind: { type: "liveSource", source: "CAM (2)", display: { scale: "fit" } } } } as never;
    expect(fillKindPatch(hiddenCam, "liveSource", (k: LiveFill): LiveFill => ({ ...k, display: { scale: "fill" } }))).toEqual({
      fill: { kind: { type: "liveSource", source: "CAM (2)", display: { scale: "fill" } } },
    });
  });
  it("reads a fill without `enabled` as switched off, as the server does (H09-1)", () => {
    expect(fillDrawn({ kind: { type: "color", color: { r: 1, g: 0, b: 0 } } })).toBe(false);
    expect(fillDrawn({ enabled: false })).toBe(false);
    expect(fillDrawn(undefined)).toBe(false);
    expect(fillDrawn({ enabled: true, kind: { type: "color", color: { r: 1, g: 0, b: 0 } } })).toBe(true);
    // A box whose video fill is switched off isn't listed as an image.
    const box = { id: "v", bounds: { x: 0, y: 0, width: 10, height: 10 }, kind: { type: "shape", shape: { type: "rectangle" } }, fill: { kind: { type: "media", asset: "clipB", display: { scale: "fit" } } } } as never;
    expect(elementType(box)).toBe("shape");
    expect(elementType({ ...(box as object), fill: { enabled: true, kind: { type: "media", asset: "clipB", display: { scale: "fit" } } } } as never)).toBe("image");
  });
});
