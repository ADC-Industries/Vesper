import { describe, expect, it } from "vitest";
import type { StageScreenState } from "$protocol";
import { DATA_LINKS, DEFAULT_PRESETS, addPreset, dataLinkLabel, parsePresets, removePreset, stageScreenMenu, timerLink } from "./stage";
import { newDataLinkElement } from "../../edit/elements";

describe("stage message presets", () => {
  it("defaults when nothing or garbage is stored", () => {
    expect(parsePresets(null)).toEqual([...DEFAULT_PRESETS]);
    expect(parsePresets("{oops")).toEqual([...DEFAULT_PRESETS]);
    expect(parsePresets('{"a":1}')).toEqual([...DEFAULT_PRESETS]);
    // An emptied list stays empty.
    expect(parsePresets("[]")).toEqual([]);
    expect(parsePresets('["Go", 3, "  ", "Stop"]')).toEqual(["Go", "Stop"]);
  });

  it("adds and removes", () => {
    let l = addPreset(["Wrap up"], "  Pray  ");
    expect(l).toEqual(["Wrap up", "Pray"]);
    expect(addPreset(l, "wrap UP")).toEqual(l);
    expect(addPreset(l, "   ")).toEqual(l);
    expect(removePreset(l, 0)).toEqual(["Pray"]);
  });
});

describe("data links", () => {
  it("offers the stage display links", () => {
    const ids = DATA_LINKS.map((d) => d.id);
    for (const id of ["currentSlide", "nextSlide", "notes", "clock", "videoCountdown", "currentImage", "playlistItem", "stageMessage"]) expect(ids).toContain(id);
    expect(timerLink(null).link).toEqual({ type: "timer" });
    expect(timerLink({ id: "t1", name: "Walk-in" }).link).toEqual({ type: "timer", timer: { id: "t1", name: "Walk-in" } });
    expect(dataLinkLabel({ type: "slideText", which: "next" })).toBe("Next Slide Text");
    expect(dataLinkLabel({ type: "timer", timer: { id: "x", name: "Walk-in" } })).toBe("Timer: Walk-in");
  });

  it("makes text elements carrying the link (pictures without text)", () => {
    const canvas = { width: 1920, height: 1080 };
    const style = { fontFamily: "DejaVu Sans", size: 72 };
    const next = DATA_LINKS.find((d) => d.id === "nextSlide")!;
    const e = newDataLinkElement("e1", next, canvas, style);
    expect(e.dataLink).toEqual({ type: "slideText", which: "next" });
    expect(e.kind?.type).toBe("text");
    expect(e.name).toBe("Next Slide Text");
    expect(e.kind?.type === "text" && e.kind.text?.paragraphs?.[0]?.runs?.[0]?.text).toBe("Next Slide");
    const pic = newDataLinkElement("e2", DATA_LINKS.find((d) => d.id === "currentImage")!, canvas, style);
    expect(pic.dataLink).toEqual({ type: "slideImage" });
    expect(pic.bounds?.width! / pic.bounds?.height!).toBeCloseTo(16 / 9, 1);
    // Song artwork: a square picture frame.
    const art = newDataLinkElement("e3", DATA_LINKS.find((d) => d.id === "songArtwork")!, canvas, style);
    expect(art.dataLink).toEqual({ type: "songArtwork" });
    expect(art.bounds?.width).toBeCloseTo(art.bounds?.height ?? 0);
  });

  it("offers presentation info and labels the new links", () => {
    const ids = DATA_LINKS.map((d) => d.id);
    for (const id of ["presentationName", "presentationNotes", "ccli", "songArtwork", "groupName", "slideLabel", "slideCount"]) expect(ids).toContain(id);
    expect(dataLinkLabel({ type: "presentationName", source: "announcements" })).toBe("Announcement Name");
    expect(dataLinkLabel({ type: "ccliText" })).toBe("CCLI Copyright");
    expect(dataLinkLabel({ type: "timer", show: "elapsed" })).toBe("Timer (elapsed)");
    expect(dataLinkLabel({ type: "slideCount", count: "total" })).toBe("Slide Count");
  });
});

describe("Stage Screen menu", () => {
  const s = (id: string, layout: string | null): StageScreenState => ({ screen: id, name: `Screen ${id}`, layout, layoutName: layout ?? "Current / Next" });
  const layouts = [
    { id: "a", name: "Current / Next" },
    { id: "b", name: "Notes" },
  ];

  it("lists layouts directly for one stage screen", () => {
    const chosen: [string, string | null][] = [];
    const items = stageScreenMenu([s("1", "b")], layouts, (sc, l) => chosen.push([sc, l]));
    expect(items.map((i) => i.type === "item" && i.label)).toEqual(["Current / Next", "Notes"]);
    expect(items.map((i) => i.type === "item" && i.checked)).toEqual([false, true]);
    const first = items[0]!;
    if (first.type === "item") first.run?.();
    expect(chosen).toEqual([["1", "a"]]);
  });

  it("nests per screen when there are several, and shows the built-in layout when none exist", () => {
    const items = stageScreenMenu([s("1", "a"), s("2", null)], layouts, () => {});
    expect(items).toHaveLength(2);
    const second = items[1]!;
    expect(second.type === "item" && second.submenu?.length).toBe(2);
    const none = stageScreenMenu([s("1", null)], [], () => {});
    expect(none.map((i) => i.type === "item" && [i.label, i.checked])).toEqual([["Current / Next", true]]);
  });
});
