import { describe, expect, it, vi } from "vitest";
import type { MenuItem } from "./menu";
import { LATER, orielMenu, type OrielMenuContext } from "./orielMenu";

function ctx(over: Partial<OrielMenuContext> = {}): OrielMenuContext {
  const act = new Proxy({}, { get: (_t, k) => (k === "app" ? vi.fn() : vi.fn()) }) as OrielMenuContext["act"];
  return {
    appName: "Oriel",
    desktop: true,
    route: "show",
    available: () => true,
    undoLabel: null,
    redoLabel: null,
    undoBusy: false,
    audienceOn: true,
    stageOn: false,
    continuous: false,
    slideView: "grid",
    mediaVisible: true,
    act,
    ...over,
  };
}
const items = (m: ReturnType<typeof orielMenu>) => m.flatMap((d) => d.items).filter((i): i is Extract<MenuItem, { type: "item" }> => i.type === "item");

describe("Oriel menu", () => {
  it("has the canvas's seven panes, and Reports (batch 16)", () => {
    expect(orielMenu(ctx()).map((m) => m.label)).toEqual(["File", "Edit", "View", "Import", "Export", "Settings", "Reports", "Help"]);
    const reports = items([orielMenu(ctx())[6]!]);
    expect(reports.map((i) => i.label)).toEqual(["CCLI Usage…"]);
    expect(reports[0]!.run).toBeTruthy();
  });

  it("has no Open, Save or Save As, and every item runs something or says it comes later", () => {
    const all = items(orielMenu(ctx({ undoLabel: "Rename", redoLabel: "Rename" })));
    expect(all.some((i) => /^(Open|Save)/.test(i.label))).toBe(false);
    for (const i of all) {
      if (i.disabled) expect(i.note === LATER || !!i.title).toBe(true);
      else expect(i.run ?? i.submenu).toBeTruthy();
    }
  });

  it("keeps Lyrics from Text and the later Codex export and import items", () => {
    const m = orielMenu(ctx());
    const imp = items([m[3]!]);
    expect(imp.find((i) => i.id === "lyrics")?.disabled).toBeFalsy();
    expect(imp.find((i) => i.id === "codex-import")?.note).toBe(LATER);
    expect(items([m[4]!]).find((i) => i.id === "codex-backup")?.note).toBe(LATER);
  });

  it("imports PowerPoint, Keynote and Google Slides decks", () => {
    const importSlides = vi.fn();
    const act = new Proxy({}, { get: (_t, k) => (k === "importSlides" ? importSlides : vi.fn()) }) as OrielMenuContext["act"];
    const pptx = items([orielMenu({ ...ctx(), act })[3]!]).find((i) => i.id === "pptx");
    expect(pptx?.label).toBe("PowerPoint / Keynote / Google Slides…");
    expect(pptx?.disabled).toBeFalsy();
    pptx?.run?.();
    expect(importSlides).toHaveBeenCalledOnce();
  });

  it("lists only Settings sections Oriel has, and nothing that opens an empty pane", () => {
    const settings = items([orielMenu(ctx())[5]!]);
    expect(settings.map((i) => i.label)).toEqual([
      "General…",
      "Screens & Looks…",
      "Audio…",
      "Captions…",
      "Keyboard Shortcuts…",
      "Network & IDN…",
      "Recording & Streaming…",
      "Media Preparation…",
      "Slide Labels…",
      "Licenses (CCLI)…",
      "Check for Updates…",
    ]);
    for (const i of settings) if (!i.disabled) expect(i.run).toBeTruthy();
    expect(settings.find((i) => i.id === "s-labels")?.note).toBe(LATER);
    expect(settings.find((i) => i.id === "s-ccli")?.note).toBeUndefined();
  });

  it("checks the current page and disables pages the server can't show", () => {
    const all = items(orielMenu(ctx({ route: "timers", available: (r) => r !== "site" })));
    expect(all.find((i) => i.id === "p-timers")?.checked).toBe(true);
    expect(all.find((i) => i.id === "v-show")?.checked).toBe(false);
    const site = all.find((i) => i.id === "p-site")!;
    expect(site.disabled).toBe(true);
    expect(site.title).toMatch(/site/);
  });

  it("shows accelerators from the table, without desktop-only keys in a browser", () => {
    const desk = items(orielMenu(ctx()));
    const web = items(orielMenu(ctx({ desktop: false })));
    expect(desk.find((i) => i.id === "p-stage")?.shortcut).toBe("Ctrl+T");
    expect(web.find((i) => i.id === "p-stage")?.shortcut).toBeUndefined();
    expect(web.find((i) => i.id === "p-screens")?.shortcut).toBe("Ctrl+Alt+S");
    expect(web.find((i) => i.id === "v-edit")?.shortcut).toBe("Ctrl+E");
  });

  it("runs app actions through the context", () => {
    const c = ctx();
    const app = vi.fn();
    c.act = { ...c.act, app };
    items(orielMenu(c)).find((i) => i.id === "p-library")!.run!();
    expect(app).toHaveBeenCalledWith("pageLibrary");
  });
});
