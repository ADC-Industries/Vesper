import { describe, expect, it } from "vitest";
import { HEADER_PAGES, PAGES, parseRoute, routeAvailable, routeHash } from "./pages";

describe("page routes", () => {
  it("round-trips every route through the URL hash", () => {
    for (const r of ["show", "edit", "reflow", ...PAGES.map((p) => p.id)] as const) expect(parseRoute(routeHash(r))).toBe(r);
    expect(parseRoute("#library")).toBe("library");
    expect(parseRoute("")).toBeNull();
    expect(parseRoute("#/settings")).toBe("settings");
    expect(parseRoute("#/nowhere")).toBeNull();
    expect(parseRoute("#/library/extra")).toBeNull();
  });

  it("lists the header's pages in the canvas's order with Done or Close", () => {
    expect(HEADER_PAGES.map((p) => p.label)).toEqual(["Bible", "Library", "Stage", "Timers", "Screens", "Site"]);
    expect(PAGES.filter((p) => p.closeLabel === "Done").map((p) => p.id)).toEqual(["screens"]);
  });

  it("needs the server's capabilities for Bible, Library, Screens and Site", () => {
    const none = () => false;
    expect(routeAvailable("show", none)).toBe(true);
    expect(routeAvailable("edit", none)).toBe(true);
    expect(routeAvailable("bible", none)).toBe(false);
    expect(routeAvailable("bible", (c) => c === "scripture")).toBe(true);
    expect(routeAvailable("timers", none)).toBe(true);
    expect(routeAvailable("settings", none)).toBe(true);
    expect(routeAvailable("library", none)).toBe(false);
    expect(routeAvailable("site", none)).toBe(false);
    expect(routeAvailable("screens", none)).toBe(false);
    expect(routeAvailable("library", (c) => c === "codex")).toBe(true);
  });
});
