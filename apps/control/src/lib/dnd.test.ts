import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { drag, type DropHandlers } from "./dnd.svelte";

// A window and document just big enough for the drag code (node has neither).
let win: EventTarget;
const pointer = (type: string, x = 0, y = 0) => Object.assign(new Event(type), { clientX: x, clientY: y, button: 0 });

beforeEach(() => {
  win = new EventTarget();
  vi.stubGlobal("window", win);
  vi.stubGlobal("document", { elementFromPoint: () => null });
});
afterEach(() => {
  // Leaves no press behind for the next test.
  win.dispatchEvent(pointer("pointercancel"));
  vi.unstubAllGlobals();
});

function handlers(): DropHandlers & { calls: string[] } {
  const calls: string[] = [];
  return {
    calls,
    onPlaylist: (pl, p) => calls.push(`playlist ${pl} ${p.id}`),
    onList: (after, p) => calls.push(`list ${after} ${p.id}`),
    onSpring: () => {},
  };
}

describe("drag and drop", () => {
  it("a press the browser cancelled (a touch pan) is forgotten: a later pointerup drops nothing (A12-4)", () => {
    const h = handlers();
    drag.press(pointer("pointerdown", 10, 10) as PointerEvent, { kind: "item", id: "stale", name: "Stale" }, h);
    win.dispatchEvent(pointer("pointercancel"));
    win.dispatchEvent(pointer("pointermove", 60, 60));
    expect(drag.payload).toBeNull();
    win.dispatchEvent(pointer("pointerup", 60, 60));
    expect(h.calls).toEqual([]);
    expect(drag.consumeClick()).toBe(false);
  });

  it("a drag cancelled with Esc: the release's click is not a pick (A14-3)", () => {
    const h = handlers();
    drag.press(pointer("pointerdown", 10, 10) as PointerEvent, { kind: "media", id: "a1", name: "Clip" }, h);
    win.dispatchEvent(pointer("pointermove", 40, 40));
    expect(drag.payload?.id).toBe("a1");
    win.dispatchEvent(Object.assign(new Event("keydown"), { key: "Escape" }));
    expect(drag.payload).toBeNull();
    win.dispatchEvent(pointer("pointerup", 10, 10));
    expect(drag.consumeClick()).toBe(true);
    expect(h.calls).toEqual([]);
  });

  it("a drag released where it started (no target): the click that follows is swallowed (A14-3)", () => {
    const h = handlers();
    drag.press(pointer("pointerdown", 10, 10) as PointerEvent, { kind: "media", id: "a1", name: "Clip" }, h);
    win.dispatchEvent(pointer("pointermove", 40, 40));
    win.dispatchEvent(pointer("pointermove", 10, 10));
    win.dispatchEvent(pointer("pointerup", 10, 10));
    expect(drag.consumeClick()).toBe(true);
  });

  it("a plain click is still a click", () => {
    const h = handlers();
    drag.press(pointer("pointerdown", 10, 10) as PointerEvent, { kind: "media", id: "a1", name: "Clip" }, h);
    win.dispatchEvent(pointer("pointerup", 10, 10));
    expect(drag.consumeClick()).toBe(false);
  });
});
