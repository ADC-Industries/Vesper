import { describe, expect, it } from "vitest";
import { APP_SHORTCUTS, SHORTCUT_GROUPS, SHORTCUT_TABLE, rowsOf, shortcutLabel } from "./shortcuts";

describe("shortcut table", () => {
  it("gives menus their labels, and none for desktop-only keys in a browser", () => {
    expect(shortcutLabel("viewEdit")).toBe("Ctrl+E");
    expect(shortcutLabel("pageScreens")).toBe("Ctrl+Alt+S");
    expect(shortcutLabel("redo")).toBe("Ctrl+Shift+Z");
    expect(shortcutLabel("pageStage", true)).toBe("Ctrl+T");
    expect(shortcutLabel("pageStage", false)).toBeUndefined();
    expect(shortcutLabel("newPresentation", false)).toBeUndefined();
  });

  it("has no two actions on one key combination", () => {
    const seen = new Map<string, string>();
    for (const r of APP_SHORTCUTS) {
      for (const b of r.bindings) {
        const k = `${b.ctrl}${!!b.shift}${!!b.alt}${b.code}`;
        expect(seen.get(k) ?? r.id).toBe(r.id);
        seen.set(k, r.id);
      }
    }
  });

  it("keeps F4 / F5 / F6 in today's order and has no Save, Open or Print", () => {
    const f = (k: string) => SHORTCUT_TABLE.find((r) => r.keys.includes(k))?.action;
    expect([f("F4"), f("F5"), f("F6")]).toEqual(["Clear audio", "Clear props", "Clear messages"]);
    expect(SHORTCUT_TABLE.some((r) => /^(Save|Open|Print)/.test(r.action))).toBe(false);
    expect(f("Ctrl+S")).toBe("Back to Show");
  });

  it("lists every row in one of the groups", () => {
    expect(SHORTCUT_GROUPS.flatMap(rowsOf).length).toBe(SHORTCUT_TABLE.length);
  });
});
