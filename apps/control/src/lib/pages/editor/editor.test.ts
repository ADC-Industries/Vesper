import { describe, expect, it } from "vitest";
import type { Element, PresentationOp, Slide, TextBox, Transition } from "$protocol";
import { alignOps, distributeOps, matchSizeOps, orderAfter, orderOps } from "./arrange";
import { copyTextStyle, pasteTextStyleOps } from "./styleClip";
import { textBoxFromPlain } from "./textField";
import { editorKey, type KeyLike } from "./keys";
import { directionOf, directionsFor, gridChoice, TRANSITION_GRID, withDirection } from "./transitions";

const box = (id: string, x: number, y: number, width: number, height: number, extra: Partial<Element> = {}): Element => ({ id, bounds: { x, y, width, height }, ...extra });
const bounds = (ops: PresentationOp[]) =>
  Object.fromEntries(ops.map((o) => (o.type === "setElement" ? [o.element, o.patch.bounds] : ["?", null])));

describe("arrange", () => {
  const a = box("a", 100, 100, 200, 100);
  const b = box("b", 400, 300, 100, 50);
  const c = box("c", 900, 500, 300, 200);

  it("aligns to each other's common bounds", () => {
    const out = bounds(alignOps("s", [a, b, c], "left"));
    expect(out.b).toEqual({ x: 100, y: 300, width: 100, height: 50 });
    expect(out.c).toEqual({ x: 100, y: 500, width: 300, height: 200 });
    expect(out.a).toBeUndefined(); // already there
  });

  it("aligns to the slide", () => {
    const out = bounds(alignOps("s", [b], "centre", { width: 1920, height: 1080 }));
    expect(out.b).toEqual({ x: 910, y: 300, width: 100, height: 50 });
    expect(alignOps("s", [b], "centre")).toEqual([]); // one object: nothing to align to
  });

  it("leaves locked objects in place (but aligns to them)", () => {
    const locked = box("l", 50, 0, 10, 10, { locked: true });
    const out = bounds(alignOps("s", [locked, b], "left"));
    expect(out.l).toBeUndefined();
    expect(out.b?.x).toBe(50);
  });

  it("distributes three or more", () => {
    expect(distributeOps("s", [a, b], "horizontal")).toEqual([]);
    const out = bounds(distributeOps("s", [a, b, c], "horizontal"));
    // span 100..1200, widths 600 → gaps of 250: b at 550
    expect(out.b?.x).toBe(550);
  });

  it("matches size to the first selected, keeping centres", () => {
    const out = bounds(matchSizeOps("s", [a, b], "width"));
    expect(out.b).toEqual({ x: 350, y: 300, width: 200, height: 50 });
    const both = bounds(matchSizeOps("s", [a, b], "both"));
    expect(both.b).toEqual({ x: 350, y: 275, width: 200, height: 100 });
    expect(matchSizeOps("s", [a], "width")).toEqual([]);
  });

  it("orders: front, back, forward, backward", () => {
    const order = ["1", "2", "3", "4", "5"];
    expect(orderAfter(order, new Set(["2", "4"]), "front")).toEqual(["1", "3", "5", "2", "4"]);
    expect(orderAfter(order, new Set(["2", "4"]), "back")).toEqual(["2", "4", "1", "3", "5"]);
    expect(orderAfter(order, new Set(["2", "4"]), "forward")).toEqual(["1", "3", "2", "5", "4"]);
    expect(orderAfter(order, new Set(["4", "5"]), "forward")).toEqual(order);
    expect(orderAfter(order, new Set(["2", "3"]), "backward")).toEqual(["2", "3", "1", "4", "5"]);
  });

  it("order ops replay to the wanted order", () => {
    const slide: Slide = { id: "s", elements: ["1", "2", "3", "4", "5"].map((id) => box(id, 0, 0, 1, 1)) };
    for (const move of ["front", "back", "forward", "backward"] as const) {
      const ops = orderOps(slide, ["2", "4"], move);
      const cur = slide.elements!.map((e) => e.id);
      for (const op of ops) {
        if (op.type !== "reorderElement") throw new Error("not a reorder");
        cur.splice(cur.indexOf(op.element), 1);
        cur.splice(op.index, 0, op.element);
      }
      expect(cur).toEqual(orderAfter(["1", "2", "3", "4", "5"], new Set(["2", "4"]), move));
    }
  });
});

describe("copy / paste style", () => {
  const src: Element = {
    id: "src",
    kind: {
      type: "text",
      text: {
        defaultStyle: { fontFamily: "Inter", size: 60 },
        paragraphs: [{ style: { alignment: "left", list: { type: "bullet" } as never }, runs: [{ text: "Hello", style: { color: { r: 1, g: 0, b: 0 }, fontWeight: 700 } }] }],
        fit: "shrinkToFit" as never,
        outline: { enabled: true, width: 4, color: { r: 0, g: 0, b: 0 } },
      },
    },
  };
  const dst: Element = { id: "dst", kind: { type: "text", text: { paragraphs: [{ runs: [{ text: "World!" }] }] } } };

  it("copies the first character's style, the paragraph's look and the box settings", () => {
    const clip = copyTextStyle(src)!;
    expect(clip.character).toMatchObject({ fontFamily: "Inter", size: 60, fontWeight: 700, color: { r: 1, g: 0, b: 0 } });
    expect(clip.paragraph.alignment).toBe("left");
    expect(clip.paragraph.list).toBeUndefined();
    expect(clip.box.fit).toBe("shrinkToFit");
    expect(clip.box.outline?.enabled).toBe(true);
    expect(clip.box.textShadow?.enabled).toBe(false);
    expect(copyTextStyle(box("shape", 0, 0, 1, 1))).toBeNull();
  });

  it("pastes over the whole box: defaults, every character, and the box", () => {
    const ops = pasteTextStyleOps("s", dst, copyTextStyle(src)!);
    expect(ops.map((o) => o.type)).toEqual(["setTextStyle", "setTextStyle", "setTextBox"]);
    const ranged = ops[1]!;
    expect(ranged.type === "setTextStyle" && ranged.range).toEqual({ start: 0, end: 6 });
    expect(pasteTextStyleOps("s", box("shape", 0, 0, 1, 1), copyTextStyle(src)!)).toEqual([]);
  });
});

describe("text field", () => {
  const tb: TextBox = {
    fit: "none" as never,
    paragraphs: [
      { runs: [{ text: "Bright ", style: { size: 50 } }, { text: "morning", style: { size: 50, italic: true } }] },
      { style: { alignment: "center" }, runs: [{ text: "second line", style: { size: 40 } }] },
    ],
  };

  it("keeps unchanged lines' runs", () => {
    expect(textBoxFromPlain(tb, "Bright morning\nsecond line")).toEqual(tb);
  });

  it("a changed line takes its paragraph's first style; a new line the one before", () => {
    const out = textBoxFromPlain(tb, "Bright morning\nother words\nthird");
    expect(out.paragraphs![0]).toEqual(tb.paragraphs![0]);
    expect(out.paragraphs![1]).toEqual({ style: { alignment: "center" }, runs: [{ text: "other words", style: { size: 40 } }] });
    expect(out.paragraphs![2]!.runs).toEqual([{ text: "third", style: { size: 40 } }]);
    expect(out.fit).toBe("none");
  });

  it("fewer lines drop paragraphs", () => {
    expect(textBoxFromPlain(tb, "Bright morning").paragraphs).toHaveLength(1);
  });
});

describe("editing keys", () => {
  const k = (key: string, code: string, m: Partial<KeyLike> = {}): KeyLike => ({ key, code, ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...m });

  it("maps keys to commands", () => {
    expect(editorKey(k("Delete", "Delete"))).toEqual({ cmd: "delete" });
    expect(editorKey(k("Backspace", "Backspace"))).toEqual({ cmd: "delete" });
    expect(editorKey(k("ArrowLeft", "ArrowLeft", { shiftKey: true }))).toEqual({ cmd: "nudge", dx: -10, dy: 0 });
    expect(editorKey(k("a", "KeyA", { ctrlKey: true }))).toEqual({ cmd: "selectAll" });
    expect(editorKey(k("d", "KeyD", { metaKey: true }))).toEqual({ cmd: "duplicate" });
    expect(editorKey(k("=", "Equal", { ctrlKey: true }))).toEqual({ cmd: "zoomIn" });
    expect(editorKey(k("0", "Digit0", { ctrlKey: true }))).toEqual({ cmd: "zoomFit" });
    expect(editorKey(k("x", "KeyX"))).toBeNull();
  });

  it("uses the physical key, and AltGr (Ctrl+Alt) is never a command", () => {
    // AZERTY: the key labelled "q" sits where KeyA is.
    expect(editorKey(k("q", "KeyA", { ctrlKey: true }))).toEqual({ cmd: "selectAll" });
    expect(editorKey(k("@", "Digit0", { ctrlKey: true, altKey: true }))).toBeNull();
    expect(editorKey(k("ArrowUp", "ArrowUp", { altKey: true }))).toBeNull();
  });
});

describe("transitions grid", () => {
  it("offers every kind the server draws", () => {
    expect(TRANSITION_GRID.filter((t) => t.drawn).map((t) => t.id)).toEqual(["cut", "dissolve", "push", "wipe", "zoom", "slide"]);
    expect(gridChoice(undefined)).toBeNull();
    expect(gridChoice({ kind: { type: "dissolve" }, duration: 1 })).toBe("dissolve");
    expect(gridChoice({ kind: { type: "cut" } })).toBe("cut");
    expect(gridChoice({ kind: { type: "push", direction: "left" } })).toBe("push");
    expect(gridChoice({ kind: { type: "vendor", name: "Swirl" } })).toBeNull();
  });

  it("directions per kind", () => {
    expect(directionsFor("wipe").map((d) => d.id)).toContain("center");
    expect(directionsFor("push").map((d) => d.id)).toEqual(["left", "right", "up", "down"]);
    expect(directionsFor("zoom")).toEqual([]);
    const push: Transition = { kind: { type: "push", direction: "left" }, duration: 1, easing: "easeInOut" };
    expect(withDirection(push, "up")).toEqual({ ...push, kind: { type: "push", direction: "up" } });
    expect(directionOf(withDirection({ kind: { type: "zoom" }, duration: 1 }, "out"))).toBe("out");
    expect(directionOf({ kind: { type: "zoom" } })).toBe("in");
  });
});
