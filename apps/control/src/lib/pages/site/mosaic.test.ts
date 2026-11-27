import { describe, expect, it } from "vitest";
import type { MosaicBox, MosaicChannel, MosaicScreen, MosaicState, MosaicWall } from "$protocol";
import {
  addressNote,
  boxLogoNote,
  logoNote,
  serviceNote,
  updatesNote,
  boxSummary,
  cellChoices,
  cellKey,
  conflicts,
  displaySources,
  displayTiles,
  draftOf,
  editableSettings,
  emptyWall,
  feedChoices,
  feedLabel,
  feedSource,
  feedValue,
  freeScreens,
  parseFeed,
  parseKey,
  placeScreen,
  resizeWall,
  sameSettings,
  screenLabel,
  screenStatus,
  settingsToSave,
  wallBody,
  wallLayout,
  wallProblem,
  wallStatus,
} from "./mosaic";

const settings = { fit: "fit", rotation: 0, overscan: 0, fallback: "logo", fallbackAfterSeconds: 5 } as const;
const screen = (number: number, extra: Partial<MosaicScreen> = {}): MosaicScreen => ({
  number,
  name: `HDMI-${number}`,
  feed: { kind: "none" },
  showing: "",
  settings: { ...settings },
  status: "idle",
  ...extra,
});
const box = (name: string, extra: Partial<MosaicBox> = {}): MosaicBox => ({
  machine: `fp-${name}`,
  name,
  health: "online",
  service: "running",
  added: false,
  vesper: false,
  reachable: true,
  screens: [screen(1), screen(2)],
  ...extra,
});
const main: MosaicChannel = { id: "c1", name: "Main", source: "BOOTH (Program)", screens: 1 };

describe("Site ▸ Displays logic", () => {
  const state: MosaicState = { managed: true, boxes: [box("Lobby"), box("Chapel", { vesper: true, screens: [], conflict: "two programs" })], channels: [main] };

  it("lists every screen of the display boxes, never a Vesper machine's", () => {
    const t = displayTiles(state);
    expect(t.map((x) => x.key)).toEqual(["fp-Lobby/1", "fp-Lobby/2"]);
    expect(screenLabel(t[1]!)).toBe("Lobby · 2 HDMI-2");
    expect(conflicts(state).map((b) => b.name)).toEqual(["Chapel"]);
    expect(displayTiles(null)).toEqual([]);
  });

  it("says what a feed is", () => {
    expect(feedLabel({ kind: "none" }, [main])).toBe("Nothing (desktop)");
    expect(feedLabel({ kind: "source", source: "BOOTH (Stage)" }, [main])).toBe("BOOTH (Stage)");
    expect(feedLabel({ kind: "channel", channel: "c1" }, [main])).toBe("Channel Main · BOOTH (Program)");
    expect(feedLabel({ kind: "channel", channel: "gone" }, [main])).toBe("A deleted channel");
  });

  it("round-trips the feed picker's values and offers channels before sources", () => {
    for (const f of [{ kind: "none" }, { kind: "source", source: "A (b)" }, { kind: "channel", channel: "c1" }] as const) {
      expect(parseFeed(feedValue(f))).toEqual(f);
    }
    const c = feedChoices(["X (1)"], [main], { kind: "source", source: "GONE (2)" });
    expect(c.map((o) => o.label)).toEqual(["Nothing (desktop)", "Channel: Main", "GONE (2)", "X (1)"]);
    expect(displaySources(["omt:GFX (Lower thirds)", "BOOTH (Program)"])).toEqual(["BOOTH (Program)"]);
  });

  it("puts a screen's state in words", () => {
    expect(screenStatus({ status: "showing" })).toEqual({ label: "Showing", status: "ok", title: undefined });
    expect(screenStatus({ status: "showing", testPattern: true }).label).toBe("Showing · test pattern");
    expect(screenStatus({ status: "offline", problem: "no answer" })).toEqual({ label: "Offline", status: "error", title: "no answer" });
  });

  it("sums a box up for Site ▸ Devices", () => {
    const b = box("Lobby", { screens: [screen(1, { feed: { kind: "channel", channel: "c1" } }), screen(2)] });
    expect(boxSummary(b, [main])).toBe("2 screens · Channel Main, idle");
    expect(boxSummary(box("X", { reachable: false, problem: "offline in the site" }), [])).toBe("Displays: offline in the site");
    expect(boxSummary(box("Y", { vesper: true }), [])).toMatch(/Conflict/);
  });
});

import { ownDisplays } from "./mosaic";

describe("Site ▸ Displays: a screen's settings (H08-1)", () => {
  const tile = { rows: 1, columns: 2, row: 0, column: 1, bezelMm: [0, 0] as [number, number], group: "wall:Front" };
  it("never sends a wall's tile back, so a screen out of its wall isn't cropped again", () => {
    const inWall = { ...settings, tile };
    const draft = editableSettings(inWall);
    expect(draft).not.toHaveProperty("tile");
    // The wall went (the server's settings have no tile now); the operator changes Rotation.
    const body = settingsToSave(draft, { ...draft, rotation: 90 }, { ...settings });
    expect(body).toEqual({ ...settings, rotation: 90 });
    expect(body).not.toHaveProperty("tile");
    // Even a draft copied with its tile by an older build.
    expect(settingsToSave(inWall, { ...inWall, overscan: 2 }, { ...settings, tile })).not.toHaveProperty("tile");
  });
  it("sends only what the operator changed, over what the server has now", () => {
    const base = { ...settings };
    // Another admin set the fallback meanwhile; this one changes the fit.
    const body = settingsToSave(base, { ...base, fit: "fill" }, { ...settings, fallback: "black" });
    expect(body).toEqual({ ...settings, fit: "fill", fallback: "black" });
    // Rotation from the picker as text is sent as a number.
    expect(settingsToSave(base, { ...base, rotation: "180" as never }, base).rotation).toBe(180);
    // A sound taken off is taken off.
    const withSound = { ...settings, sound: { device: "hdmi", volumeDb: 0, mute: false } };
    expect(settingsToSave(withSound, { ...settings }, withSound)).not.toHaveProperty("sound");
  });
  it("tells settings apart, the tile aside", () => {
    expect(sameSettings({ ...settings, tile }, { ...settings })).toBe(true);
    expect(sameSettings({ ...settings }, { ...settings, overscan: 1 })).toBe(false);
  });
});

describe("this server's own displays", () => {
  it("names what drives each", () => {
    const d = ownDisplays(
      [
        { id: "HDMI-A-1", name: "Stage Monitor", connected: true, boundTo: ["o1"] },
        { id: "DP-1", name: "DP-1", connected: false, boundTo: [] },
      ],
      [{ id: "o1", name: "Stage out", screen: "s1" }],
      [{ id: "s1", name: "Stage" }],
    );
    expect(d[0]).toEqual({ id: "HDMI-A-1", name: "Stage Monitor (HDMI-A-1)", connected: true, drivenBy: ["Stage out → Stage"] });
    expect(d[1]!.name).toBe("DP-1");
  });
});

describe("Site ▸ Displays: the wall designer", () => {
  const wall: MosaicWall = {
    id: "w1",
    name: "Front",
    rows: 1,
    columns: 2,
    bezelMm: [10, 0],
    screenMm: [1000, 500],
    feed: { kind: "channel", channel: "c1" },
    cells: [{ row: 0, column: 0, machine: "fp-Lobby", screen: 1 }],
    status: "showing",
    synced: true,
    delayMs: 41.6,
    lateRecently: 0,
  };

  it("cuts the picture by the tiles, the bezels left out", () => {
    const l = wallLayout(wall);
    // 1000 + 10 + 1000 wide, 500 high.
    expect(l.aspect).toBeCloseTo(2010 / 500);
    expect(l.cells).toHaveLength(2);
    expect(l.cells[1]?.x).toBeCloseTo(1010 / 2010);
    expect(l.cells[1]?.w).toBeCloseTo(1000 / 2010);
    expect(l.cells[0]?.h).toBe(1);
    // No size given: 16:9 screens.
    expect(wallLayout({ rows: 2, columns: 2, bezelMm: [0, 0] }).aspect).toBeCloseTo(16 / 9);
  });

  it("places, moves and takes out screens", () => {
    const a = { machine: "fp-A", screen: 1 };
    let cells = placeScreen([], 0, 1, a);
    expect(cellKey(cells, 0, 1)).toBe("fp-A/1");
    cells = placeScreen(cells, 0, 0, a);
    expect(cells).toEqual([{ row: 0, column: 0, machine: "fp-A", screen: 1 }]);
    cells = placeScreen(cells, 0, 0, { machine: "fp-B", screen: 2 });
    expect(cells).toEqual([{ row: 0, column: 0, machine: "fp-B", screen: 2 }]);
    expect(placeScreen(cells, 0, 0, null)).toEqual([]);
    expect(parseKey("fp-a/b/3")).toEqual({ machine: "fp-a/b", screen: 3 });
    expect(parseKey("")).toBeNull();
  });

  it("drops cells outside a smaller wall, and checks a draft", () => {
    const d = { ...emptyWall("Side"), rows: 2, columns: 2, cells: [{ row: 1, column: 1, machine: "m", screen: 1 }] };
    expect(resizeWall(d, 1, 2).cells).toEqual([]);
    expect(resizeWall(d, 40, 0).rows).toBe(16);
    expect(wallProblem(d, [wall], null)).toBeNull();
    expect(wallProblem({ ...d, name: "front" }, [wall], null)).toMatch(/has that name/);
    expect(wallProblem({ ...d, name: "front" }, [wall], "w1")).toBeNull();
    expect(wallProblem({ ...d, bezelMm: [5, 0] }, [], null)).toMatch(/visible size/);
    expect(wallProblem({ ...d, bezelMm: [5, 0], screenMm: [1200, 680] }, [], null)).toBeNull();
    expect(wallProblem({ ...d, name: " " }, [], null)).toMatch(/name/);
    expect(draftOf(wall).cells).toEqual(wall.cells);
  });

  it("keeps a size typed away or emptied out of the cells' way (H08-3)", () => {
    const d = { ...emptyWall("Side"), rows: 2, columns: 4, cells: [{ row: 1, column: 3, machine: "m", screen: 1 }] };
    // An emptied field keeps the size, and the cells.
    expect(resizeWall(d, Number.NaN, d.columns)).toEqual(d);
    expect(resizeWall(d, d.rows, Number.NaN)).toEqual(d);
    // The whole value, once: 12 columns keep every cell.
    expect(resizeWall(d, 2, 12).cells).toEqual(d.cells);
  });

  it("keeps the wall's live feed unless the designer's Shows was changed (H08-2)", () => {
    const d = { ...draftOf(wall), name: " Front " };
    const now = { feed: { kind: "source", source: "CAM (1)" } as const };
    // The feed changed elsewhere while the designer was open; the bezels are saved.
    expect(wallBody(d, now, false).feed).toEqual(now.feed);
    expect(wallBody(d, now, false).name).toBe("Front");
    // The designer's own Shows was changed: that one.
    expect(wallBody({ ...d, feed: { kind: "none" } }, now, true).feed).toEqual({ kind: "none" });
    // A new wall: the draft's.
    expect(wallBody(d, undefined, false).feed).toEqual(wall.feed);
    // No visible size: left out.
    expect(wallBody({ ...d, screenMm: [0, 0] }, now, false)).not.toHaveProperty("screenMm");
  });

  it("keeps a cell's screen that no box lists now, so Save keeps it (R1-5)", () => {
    const st: MosaicState = { managed: true, channels: [main], boxes: [box("Lobby", { screens: [screen(2)] })] };
    const free = freeScreens(displayTiles(st), "w1");
    // fp-Gone's box was removed; fp-Lobby doesn't list screen 1 now.
    const cells = [
      { row: 0, column: 0, machine: "fp-Gone", screen: 3 },
      { row: 0, column: 1, machine: "fp-Lobby", screen: 1 },
    ];
    const a = cellChoices(cells, 0, 0, free);
    expect(a.map((o) => o.value)).toEqual(["", "fp-Gone/3", "fp-Lobby/2"]);
    expect(a[1]?.label).toBe("Screen 3 (not listed now)");
    expect(cellChoices(cells, 0, 1, free)[1]).toEqual({ value: "fp-Lobby/1", label: "Screen 1 (not listed now)" });
    // An empty cell offers only the listed screens; a listed screen in another cell moves here.
    expect(cellChoices(cells, 1, 0, free).map((o) => o.value)).toEqual(["", "fp-Lobby/2"]);
    const placed = [...cells, { row: 1, column: 1, machine: "fp-Lobby", screen: 2 }];
    expect(cellChoices(placed, 1, 0, free)[1]?.label).toBe("Lobby · 2 HDMI-2 (moves here)");
    expect(cellChoices(placed, 1, 1, free)[1]?.label).toBe("Lobby · 2 HDMI-2");
  });

  it("offers the screens not in another wall", () => {
    const st: MosaicState = { managed: true, channels: [main], boxes: [box("Lobby", { screens: [screen(1, { wall: "w1" }), screen(2), screen(3, { wall: "w2" })] })] };
    const tiles = displayTiles(st);
    expect(freeScreens(tiles, null).map((t) => t.screen.number)).toEqual([2]);
    expect(freeScreens(tiles, "w1").map((t) => t.screen.number)).toEqual([1, 2]);
  });

  it("says a wall's state and sync", () => {
    expect(wallStatus(wall)).toEqual({ label: "In step · 42 ms", status: "ok" });
    expect(wallStatus({ ...wall, synced: false, lateRecently: 4 }).status).toBe("warn");
    expect(wallStatus({ ...wall, status: "offline" }).label).toBe("Offline");
    expect(wallStatus({ ...wall, cells: [] }).label).toBe("No screens yet");
    expect(feedSource(wall.feed, [main])).toBe("BOOTH (Program)");
    expect(feedSource({ kind: "none" }, [main])).toBe("");
  });

  it("says where a box's address came from (M3-2)", () => {
    expect(addressNote(box("Lobby", { address: "192.0.2.40", addressFrom: "site" }))).toBe("Address where the site sees its IDN Node");
    expect(addressNote(box("Lobby", { address: "lobby.local", addressFrom: "admin" }))).toContain("overrides");
    expect(addressNote(box("Lobby", { reachable: false }))).toBe("Lobby isn't answering · address not known yet (its IDN Node isn't online in the site)");
  });

  it("says how a box's display service runs and who has the logo (M3-4, M3-5)", () => {
    expect(serviceNote(box("Lobby", { serviceMode: "headless" }))).toBe("Display service running · headless (as set here)");
    expect(serviceNote(box("Lobby", { serviceMode: "desktop", serviceModeReported: true }))).toBe("Display service running · in the desktop session");
    expect(serviceNote(box("Lobby", { service: "" }))).toBe("Display service not reported");
    expect(updatesNote(box("Lobby", { updateManagedBy: "Vesper" }))).toBe("Updates managed by Vesper");
    expect(updatesNote(box("Lobby"))).toBeNull();
    const st: MosaicState = { managed: true, channels: [], boxes: [box("A", { logoState: "onBox" }), box("B", { logoState: "sending" }), box("C", { vesper: true })], logo: { hash: "h", width: 800, height: 400 } };
    expect(logoNote(st)).toBe("800 × 400 · on 1 of 2 display boxes (1 sending)");
    expect(boxLogoNote({ logoState: "failed" })).toBe("Failed");
    expect(logoNote({ ...st, logo: undefined })).toContain("No logo");
  });
});
