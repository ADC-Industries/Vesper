import { describe, expect, it } from "vitest";
import type { Look } from "$protocol";
import { freeName, isDirty, languageOf, movedKey, reselect, shows, sortKeyBetween, withLanguage, withLayer, withMask, withStored, withTheme } from "./looks";

const A = "5eed0000-0000-4000-8000-00000000000a";
const B = "5eed0000-0000-4000-8000-00000000000b";
const look = (over: Partial<Look> = {}): Look => ({ schemaVersion: 2, id: "l1", name: "Lyrics", sortKey: "a", ...over }) as Look;

describe("looks matrix", () => {
  it("shows every layer on a screen the look doesn't list", () => {
    expect(shows(look(), A, "media")).toBe(true);
    expect(shows(null, A, "slide")).toBe(true);
  });

  it("hides and shows a layer, dropping entries that change nothing", () => {
    let l = withLayer(look(), A, "media", false);
    expect(l.screens).toEqual([{ screen: A, layers: { media: false } }]);
    expect(shows(l, A, "media")).toBe(false);
    expect(shows(l, B, "media")).toBe(true);
    l = withLayer(l, A, "props", false);
    expect(l.screens?.[0]?.layers).toEqual({ media: false, props: false });
    l = withLayer(withLayer(l, A, "media", true), A, "props", true);
    expect(l.screens).toEqual([]);
  });

  it("keeps an entry with only a theme override or a mask", () => {
    const t = { theme: { id: "t1", name: "Lower" }, slide: "s1" };
    let l = withTheme(look(), B, t);
    expect(l.screens).toEqual([{ screen: B, layers: {}, templateOverride: t }]);
    l = withMask(l, B, "m1");
    expect(l.screens?.[0]?.mask).toBe("m1");
    l = withMask(withTheme(l, B, null), B, null);
    expect(l.screens).toEqual([]);
  });

  it("keeps an entry with only a second-language display, automatic by default", () => {
    expect(languageOf(look(), A)).toBe("auto");
    const l = withLanguage(look(), A, "sideBySide");
    expect(l.screens).toEqual([{ screen: A, layers: {}, language: "sideBySide" }]);
    expect(languageOf(l, A)).toBe("sideBySide");
    expect(isDirty(l, look())).toBe(true);
    expect(withLanguage(l, A, "auto").screens).toEqual([]);
  });

  it("never mutates the look it was given", () => {
    const l = look({ screens: [{ screen: A, layers: { media: false } }] });
    const before = JSON.stringify(l);
    withLayer(l, A, "slide", false);
    withTheme(l, A, null);
    expect(JSON.stringify(l)).toBe(before);
  });

  it("tells a changed draft from the stored look", () => {
    const stored = look({ screens: [{ screen: A, layers: { media: false } }] });
    expect(isDirty(stored, stored)).toBe(false);
    // Hiding and showing again is no change; an empty layer map neither.
    expect(isDirty(withLayer(withLayer(stored, A, "props", false), A, "props", true), stored)).toBe(false);
    expect(isDirty(withLayer(stored, A, "props", false), stored)).toBe(true);
    expect(isDirty({ ...stored, name: " Lyrics " }, stored)).toBe(false);
    expect(isDirty({ ...stored, name: "Other" }, stored)).toBe(true);
    expect(isDirty({ ...stored, transition: { kind: { type: "dissolve" }, duration: 1 } }, stored)).toBe(true);
  });
});

describe("look order", () => {
  it("makes keys between neighbours like the server", () => {
    expect(sortKeyBetween(null, null)).toBe("V");
    expect(sortKeyBetween("a", "b")).toBe("aV");
    expect(sortKeyBetween("a", null) > "a").toBe(true);
    const k = sortKeyBetween("a", "c");
    expect(k > "a" && k < "c").toBe(true);
    const low = sortKeyBetween(null, "a");
    expect(low < "a" && low.length > 0).toBe(true);
  });

  it("moves a look up or down one place", () => {
    const list = [look({ id: "1", sortKey: "a" }), look({ id: "2", sortKey: "b" }), look({ id: "3", sortKey: "c" })];
    const up = movedKey(list, "3", -1)!;
    expect(up > "a" && up < "b").toBe(true);
    const down = movedKey(list, "1", 1)!;
    expect(down > "b" && down < "c").toBe(true);
    const top = movedKey(list, "2", -1)!;
    expect(top < "a").toBe(true);
    expect(movedKey(list, "1", -1)).toBeNull();
    expect(movedKey(list, "3", 1)).toBeNull();
  });

  it("finds a free name", () => {
    expect(freeName([look({ name: "Look 1" }), look({ name: "look 2" })])).toBe("Look 3");
  });
});

describe("look selection and own writes", () => {
  it("keeps a dirty draft pinned when the Live box is clicked (E08-1)", () => {
    // Look A is live and edited (pinned): clicking Live is no switch, and A stays pinned, so a
    // later live-look change can't drop the edits.
    expect(reselect(null, "A", "A", true)).toBe("A");
    // Without edits the Live box follows the live look again.
    expect(reselect(null, "A", "A", false)).toBeNull();
    // The same look clicked: stays selected.
    expect(reselect("A", "A", "A", false)).toBe("A");
    // Another look: a real switch (asks first when dirty).
    expect(reselect("B", "A", "A", true)).toBeUndefined();
    expect(reselect(null, "B", "A", true)).toBeUndefined();
  });

  it("puts this client's own write in the stored list (E08-2)", () => {
    const a: Look = { schemaVersion: 2, id: "a", name: "A", sortKey: "V" };
    const b: Look = { schemaVersion: 2, id: "b", name: "B", sortKey: "k" };
    const moved: Look = { ...a, sortKey: "f" };
    const list = withStored([a, b], moved);
    // The stored look now matches what the move answered: no "changed by another operator".
    expect(JSON.stringify(list.find((l) => l.id === "a"))).toBe(JSON.stringify(moved));
    expect(list[1]).toBe(b);
  });
});
