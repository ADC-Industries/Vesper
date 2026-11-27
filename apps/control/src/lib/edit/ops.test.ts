import { describe, expect, it } from "vitest";
import type { Element, Presentation } from "$protocol";
import {
  addSlideOps,
  boundsOps,
  geometryPatch,
  hotkeyFrom,
  hotkeyLabel,
  notesFromText,
  notesText,
  presentationBackground,
  presentationBackgroundOps,
  reorderIndex,
  saveTextOps,
  songInfoOp,
  textStyleOps,
  transitionFor,
  transitionLabel,
} from "./ops";
import { fromHex, toHex } from "./color";
import { moveTarget } from "./listDrag";

const text: Element = {
  id: "t1",
  bounds: { x: 100, y: 100, width: 400, height: 200 },
  kind: { type: "text", text: { paragraphs: [{ runs: [{ text: "Hi 😀" }] }, { runs: [{ text: "there" }] }] } },
};

const pres: Presentation = {
  schemaVersion: 1,
  id: "p",
  songInfo: { title: "Song" },
  sections: [
    { id: "a", slides: [{ id: "s1" }, { id: "s2", background: { r: 0, g: 0, b: 0 } }] },
    { id: "b", slides: [{ id: "s3" }] },
  ],
};

describe("inspector ops", () => {
  it("geometry with and without the aspect lock", () => {
    expect(geometryPatch(text, "x", 12.345, false)).toEqual({ bounds: { x: 12.35, y: 100, width: 400, height: 200 } });
    expect(geometryPatch(text, "width", 800, true)).toEqual({ bounds: { x: 100, y: 100, width: 800, height: 400 } });
    expect(geometryPatch(text, "rotation", -90, false)).toEqual({ rotation: 270 });
    expect(geometryPatch(text, "height", NaN, false)).toEqual({});
  });
  it("text style for the whole box sets the defaults and every run", () => {
    const ops = textStyleOps("s1", text, { character: { italic: true } }, null);
    expect(ops).toEqual([
      { type: "setTextStyle", slide: "s1", element: "t1", character: { italic: true } },
      { type: "setTextStyle", slide: "s1", element: "t1", character: { italic: true }, range: { start: 0, end: 10 } },
    ]);
  });
  it("text style for a selected range only", () => {
    const ops = textStyleOps("s1", text, { character: { color: { r: 1, g: 0, b: 0 } } }, { start: 5, end: 10 });
    expect(ops).toEqual([{ type: "setTextStyle", slide: "s1", element: "t1", character: { color: { r: 1, g: 0, b: 0 } }, range: { start: 5, end: 10 } }]);
  });
  it("bounds ops skip unchanged elements and round", () => {
    const ops = boundsOps("s1", [
      { id: "a", rect: { x: 1.001, y: 2, width: 3, height: 4 }, was: { x: 1, y: 2, width: 3, height: 4 } },
      { id: "b", rect: { x: 10.126, y: 0, width: 5, height: 5 } },
    ]);
    expect(ops).toEqual([{ type: "setElement", slide: "s1", element: "b", patch: { bounds: { x: 10.13, y: 0, width: 5, height: 5 } } }]);
  });
  it("saving text guards with expectText unless forced", () => {
    const tb = { paragraphs: [] };
    expect(saveTextOps("s", "e", "old", tb).map((o) => o.type)).toEqual(["setSlideText", "replaceTextBox"]);
    expect(saveTextOps("s", "e", "old", tb)[0]).toMatchObject({ text: "old", expectText: "old" });
    expect(saveTextOps("s", "e", "old", tb, true).map((o) => o.type)).toEqual(["replaceTextBox"]);
  });
});

describe("presentation and slide ops", () => {
  it("background applies to every slide that differs", () => {
    expect(presentationBackground(pres)).toBe("mixed");
    const ops = presentationBackgroundOps(pres, { r: 0, g: 0, b: 0 });
    expect(ops.map((o) => (o.type === "setSlide" ? o.slide : ""))).toEqual(["s1", "s3"]);
    expect(presentationBackgroundOps(pres, null).length).toBe(1);
  });
  it("copyright fields merge into songInfo; clearing the last clears it", () => {
    expect(songInfoOp(pres, "ccliNumber", "12345")).toEqual({ type: "setPresentation", songInfo: { title: "Song", ccliNumber: "12345" } });
    expect(songInfoOp(pres, "title", "")).toEqual({ type: "setPresentation", songInfo: null });
    expect(songInfoOp(pres, "display", true)).toEqual({ type: "setPresentation", songInfo: { title: "Song", display: true } });
  });
  it("notes round-trip", () => {
    expect(notesText(notesFromText("a\n\nb"))).toBe("a\n\nb");
    expect(notesFromText("")).toEqual({});
  });
  it("transitions", () => {
    expect(transitionFor("default", 1)).toBeNull();
    expect(transitionFor("dissolve", 0.5)).toEqual({ kind: { type: "dissolve" }, duration: 0.5 });
    // Moving kinds start left with ease in-out, and keep a previous direction and easing.
    expect(transitionFor("push", 0.8)).toEqual({ kind: { type: "push", direction: "left" }, duration: 0.8, easing: "easeInOut" });
    const wipe = { kind: { type: "wipe" as const, direction: "center" as const }, duration: 1, easing: "linear" as const };
    expect(transitionFor("wipe", 1, wipe)).toEqual(wipe);
    // Only a wipe opens from the centre.
    expect(transitionFor("slide", 1, wipe)).toEqual({ kind: { type: "slide", direction: "left" }, duration: 1, easing: "linear" });
    expect(transitionFor("zoom", 1)).toEqual({ kind: { type: "zoom" }, duration: 1, easing: "easeInOut" });
    expect(transitionLabel({ kind: { type: "wipe", direction: "upLeft" }, duration: 1 })).toBe("Wipe up-left (1 s)");
    expect(transitionLabel({ kind: { type: "zoom", direction: "out" } })).toBe("Zoom out");
    expect(transitionLabel({ kind: { type: "vendor", name: "Swirl" }, duration: 2 })).toBe("Swirl (2 s)");
  });
  it("hotkeys", () => {
    const k = hotkeyFrom({ code: "KeyA", key: "a", shiftKey: true, ctrlKey: false, altKey: false, metaKey: false });
    expect(k).toEqual({ code: "KeyA", shift: true });
    expect(hotkeyLabel(k!)).toBe("Shift+A");
    expect(hotkeyFrom({ code: "ShiftLeft", key: "Shift", shiftKey: true, ctrlKey: false, altKey: false, metaKey: false })).toBeNull();
  });
  it("adds slides after a slide or at the end", () => {
    expect(addSlideOps(pres, "s1", () => "x")).toEqual([{ type: "addSlide", section: "a", after: "s1" }]);
    expect(addSlideOps(pres, null, () => "x")).toEqual([{ type: "addSlide", section: "b", after: "s3" }]);
    expect(addSlideOps({ schemaVersion: 1, id: "q" }, null, () => "x")[0]).toEqual({ type: "addSection", section: { id: "x", name: "Group" } });
  });
  it("maps the Objects list position to a z index", () => {
    expect(reorderIndex(4, 0)).toBe(3);
    expect(reorderIndex(4, 3)).toBe(0);
    expect(reorderIndex(4, 9)).toBe(0);
  });
});


describe("colours and list moves", () => {
  it("hex round trip keeps alpha", () => {
    expect(toHex({ r: 1, g: 0, b: 0.5 })).toBe("#ff0080");
    expect(fromHex("#ff0080", 0.5)).toEqual({ r: 1, g: 0, b: 0.502, a: 0.5 });
    expect(fromHex("#000000")).toEqual({ r: 0, g: 0, b: 0 });
  });
  it("list drop targets", () => {
    expect(moveTarget(0, 0)).toBeNull();
    expect(moveTarget(0, 1)).toBeNull();
    expect(moveTarget(0, 3)).toBe(2);
    expect(moveTarget(3, 0)).toBe(0);
  });
});
