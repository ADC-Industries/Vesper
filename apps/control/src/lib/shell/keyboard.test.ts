import { describe, expect, it } from "vitest";
import { keyAction } from "./keyboard";

function ev(key: string, target: Partial<HTMLInputElement> | null = null, mods: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return { key, target, ctrlKey: false, metaKey: false, altKey: false, ...mods } as unknown as KeyboardEvent;
}
const body = { tagName: "BODY" } as Partial<HTMLInputElement>;
const ctx = { dialogOpen: false };

describe("keyboard shortcuts", () => {
  it("maps navigation keys", () => {
    for (const k of [" ", "ArrowRight", "ArrowDown", "PageDown"]) {
      expect(keyAction(ev(k, body), ctx)).toEqual({ kind: "command", command: { type: "next" } });
    }
    for (const k of ["ArrowLeft", "ArrowUp", "PageUp"]) {
      expect(keyAction(ev(k, body), ctx)).toEqual({ kind: "command", command: { type: "prev" } });
    }
  });

  it("maps function keys to clears", () => {
    expect(keyAction(ev("F1", body), ctx)).toEqual({ kind: "command", command: { type: "clearAll" } });
    expect(keyAction(ev("F2", body), ctx)).toEqual({ kind: "command", command: { type: "clearLayer", layer: "slide" } });
    expect(keyAction(ev("F3", body), ctx)).toEqual({ kind: "command", command: { type: "clearLayer", layer: "media" } });
    expect(keyAction(ev("F4", body), ctx)).toEqual({ kind: "command", command: { type: "clearLayer", layer: "audio" } });
  });

  it("ignores keys while typing", () => {
    const input = { tagName: "INPUT", type: "search" } as Partial<HTMLInputElement>;
    expect(keyAction(ev(" ", input), ctx)).toBeNull();
    expect(keyAction(ev("F1", input), ctx)).toBeNull();
    expect(keyAction(ev("?", input), ctx)).toBeNull();
  });

  it("leaves Space to a focused button and arrows to a focused slider", () => {
    expect(keyAction(ev(" ", { tagName: "BUTTON" } as Partial<HTMLInputElement>), ctx)).toBeNull();
    const range = { tagName: "INPUT", type: "range" } as Partial<HTMLInputElement>;
    expect(keyAction(ev("ArrowRight", range), ctx)).toBeNull();
  });

  it("leaves a key a control already took to that control (A13-1)", () => {
    // ArrowLeft on a focused tab strip or segmented control switches it (preventDefault) and
    // must not also move the live slide.
    const button = { tagName: "BUTTON" } as Partial<HTMLInputElement>;
    for (const k of ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"]) {
      expect(keyAction(ev(k, button, { defaultPrevented: true }), ctx)).toBeNull();
    }
    expect(keyAction(ev("ArrowRight", button), ctx)).toEqual({ kind: "command", command: { type: "next" } });
  });

  it("handles help, Esc and modifiers", () => {
    expect(keyAction(ev("?", body), ctx)).toEqual({ kind: "help" });
    expect(keyAction(ev("Escape", body), { dialogOpen: true })).toEqual({ kind: "closeDialog" });
    expect(keyAction(ev("Escape", body), ctx)).toBeNull();
    expect(keyAction(ev(" ", body), { dialogOpen: true })).toBeNull();
    expect(keyAction(ev("ArrowRight", body, { ctrlKey: true }), ctx)).toBeNull();
  });

  it("maps app shortcuts by physical key, also while typing", () => {
    const input = { tagName: "INPUT", type: "search" } as Partial<HTMLInputElement>;
    expect(keyAction(ev("1", body, { ctrlKey: true, code: "Digit1" } as Partial<KeyboardEvent>), ctx)).toEqual({ kind: "app", id: "toggleAudience" });
    expect(keyAction(ev("2", input, { metaKey: true, code: "Digit2" } as Partial<KeyboardEvent>), ctx)).toEqual({ kind: "app", id: "toggleStage" });
    // A French layout: Ctrl+& is still Ctrl+1 (same physical key).
    expect(keyAction(ev("&", body, { ctrlKey: true, code: "Digit1" } as Partial<KeyboardEvent>), ctx)).toEqual({ kind: "app", id: "toggleAudience" });
    expect(keyAction(ev("m", body, { ctrlKey: true, code: "KeyM" } as Partial<KeyboardEvent>), ctx)).toEqual({ kind: "app", id: "toggleMediaBin" });
    expect(keyAction(ev("l", input, { ctrlKey: true, code: "KeyL" } as Partial<KeyboardEvent>), ctx)).toEqual({ kind: "app", id: "pageLibrary" });
    expect(keyAction(ev("f", body, { ctrlKey: true, code: "KeyF" } as Partial<KeyboardEvent>), ctx)).toEqual({ kind: "app", id: "findInLibrary" });
    expect(keyAction(ev(",", body, { ctrlKey: true, code: "Comma" } as Partial<KeyboardEvent>), ctx)).toEqual({ kind: "app", id: "settings" });
  });

  it("follows the cleaned-up table: page keys, and the dropped ones", () => {
    const k = (code: string, mods: Partial<KeyboardEvent> = {}) => keyAction(ev("x", body, { ctrlKey: true, code, ...mods } as Partial<KeyboardEvent>), ctx);
    expect(k("KeyS")).toEqual({ kind: "app", id: "viewShow" });
    expect(k("KeyB")).toEqual({ kind: "app", id: "pageBible" });
    expect(k("KeyT", { altKey: true })).toEqual({ kind: "app", id: "pageTimers" });
    expect(k("KeyS", { altKey: true })).toEqual({ kind: "app", id: "pageScreens" });
    expect(k("KeyM", { altKey: true })).toEqual({ kind: "app", id: "pageSite" });
    // Dropped: Ctrl+Alt+1 (now Ctrl+Alt+S), Ctrl+Shift+V (now Ctrl+M), the panel keys, Ctrl+O, Ctrl+P.
    expect(k("Digit1", { altKey: true })).toBeNull();
    expect(k("KeyV", { shiftKey: true })).toBeNull();
    expect(k("KeyA", { shiftKey: true })).toBeNull();
    expect(k("KeyO")).toBeNull();
    expect(k("KeyP")).toBeNull();
    expect(keyAction(ev("s", body, { altKey: true, code: "KeyS" } as Partial<KeyboardEvent>), ctx)).toBeNull();
  });

  it("gives desktop-only keys to the browser outside the desktop app", () => {
    const k = (code: string, desktop: boolean, mods: Partial<KeyboardEvent> = {}) =>
      keyAction(ev("x", body, { ctrlKey: true, code, ...mods } as Partial<KeyboardEvent>), { dialogOpen: false, desktop });
    expect(k("KeyT", false)).toBeNull();
    expect(k("KeyT", true)).toEqual({ kind: "app", id: "pageStage" });
    expect(k("Equal", false)).toBeNull();
    expect(k("Equal", true)).toEqual({ kind: "app", id: "thumbLarger" });
    expect(k("Minus", true)).toEqual({ kind: "app", id: "thumbSmaller" });
    expect(k("KeyN", false)).toBeNull();
    expect(k("KeyN", true)).toEqual({ kind: "app", id: "newPresentation" });
    expect(k("KeyN", true, { shiftKey: true })).toEqual({ kind: "app", id: "newPlaylist" });
  });

  it("treats AltGr (Ctrl+Alt on Windows layouts) right", () => {
    // AltGr+E on a Polish layout types "ę": Windows reports Ctrl+Alt+KeyE. It is not Ctrl+E.
    const altGr = (key: string, code: string, target = body) =>
      keyAction(ev(key, target, { ctrlKey: true, altKey: true, code } as Partial<KeyboardEvent>), ctx);
    expect(altGr("ę", "KeyE")).toBeNull();
    // Outside a text field Ctrl+Alt+S is Screens & Looks, whatever the layout types for it.
    expect(altGr("ś", "KeyS")).toEqual({ kind: "app", id: "pageScreens" });
    // In a text field AltGr types: Ctrl+Alt+S, T and M stay with the field.
    const input = { tagName: "INPUT", type: "text" } as Partial<HTMLInputElement>;
    expect(altGr("ś", "KeyS", input)).toBeNull();
    expect(altGr("ţ", "KeyT", input)).toBeNull();
    expect(altGr("µ", "KeyM", { isContentEditable: true, tagName: "DIV" } as unknown as Partial<HTMLInputElement>)).toBeNull();
  });

  it("leaves browser shortcuts alone", () => {
    for (const code of ["KeyC", "KeyV", "KeyT", "KeyW", "Equal", "KeyD", "KeyA"]) {
      expect(keyAction(ev("x", body, { ctrlKey: true, code } as Partial<KeyboardEvent>), ctx)).toBeNull();
    }
  });

  it("maps the editing shortcuts; undo / redo stay with text fields", () => {
    const k = (code: string, mods: Partial<KeyboardEvent> = {}, target = body) =>
      keyAction(ev("x", target, { ctrlKey: true, code, ...mods } as Partial<KeyboardEvent>), { dialogOpen: false, desktop: true });
    expect(k("KeyR")).toEqual({ kind: "app", id: "viewReflow" });
    expect(k("KeyE")).toEqual({ kind: "app", id: "viewEdit" });
    expect(k("KeyN")).toEqual({ kind: "app", id: "newPresentation" });
    expect(k("KeyZ")).toEqual({ kind: "app", id: "undo" });
    expect(k("KeyZ", { shiftKey: true })).toEqual({ kind: "app", id: "redo" });
    expect(k("KeyY")).toEqual({ kind: "app", id: "redo" });
    const input = { tagName: "INPUT", type: "text" } as Partial<HTMLInputElement>;
    expect(k("KeyZ", {}, input)).toBeNull();
    expect(k("KeyY", {}, input)).toBeNull();
    // Page keys still work from a text field (Reflow's text, the library search).
    expect(k("KeyR", {}, input)).toEqual({ kind: "app", id: "viewReflow" });
  });

  it("blocks app keys behind a dialog and the operator keys on the Screens page", () => {
    expect(keyAction(ev("e", body, { ctrlKey: true, code: "KeyE" } as Partial<KeyboardEvent>), { dialogOpen: true })).toBeNull();
    expect(keyAction(ev(" ", body), { dialogOpen: false, noShowKeys: true })).toBeNull();
    expect(keyAction(ev("F1", body), { dialogOpen: false, noShowKeys: true })).toBeNull();
    expect(keyAction(ev("?", body), { dialogOpen: false, noShowKeys: true })).toEqual({ kind: "help" });
  });
});

describe("edit mode keys", () => {
  it("arrows and Space do not navigate the show while editing", () => {
    const ev = (key: string) => ({ key, code: "", ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, target: null }) as unknown as KeyboardEvent;
    expect(keyAction(ev("ArrowRight"), { dialogOpen: false, editing: true })).toBeNull();
    expect(keyAction(ev(" "), { dialogOpen: false, editing: true })).toBeNull();
    expect(keyAction(ev("ArrowRight"), { dialogOpen: false })).toEqual({ kind: "command", command: { type: "next" } });
    expect(keyAction(ev("F1"), { dialogOpen: false, editing: true })).toEqual({ kind: "command", command: { type: "clearAll" } });
  });
});
