import { describe, expect, it } from "vitest";
import type { DisplayInfo, OutputEntry, ScreenEntry } from "$protocol";
import {
  arrangementOf,
  canvasForTile,
  setArrangement,
  setTile,
  tileLabel,
  tileRects,
  tileSizeOf,
  transportLabel,
  colorToHex,
  createBody,
  decodeMode,
  displayFullscreen,
  displayLabel,
  displayModeKey,
  displayModeLabel,
  displayModeOptions,
  displayModeValue,
  displayOf,
  displayRisk,
  duplicateOutputs,
  encodeMode,
  fillKeyHint,
  freeDisplay,
  fpsLabel,
  keyOutputFor,
  outputTarget,
  setOutputTarget,
  hexToColor,
  isDirty,
  makeDraft,
  maskChoices,
  modeLabel,
  modeOptions,
  mustPark,
  newOutput,
  newlyCovering,
  parkedDoc,
  planApply,
  serverIsLocal,
  setDisplay,
  setDisplayFullscreen,
  setDisplayMode,
  setOutputMode,
  setSourceName,
  setTransportKind,
  uniqueName,
  validateOutputs,
  validateScreen,
  withRole,
  withScreenMask,
  operatorScreens,
  screenRole,
} from "./screens";

const screen: ScreenEntry = { revision: "r1", schemaVersion: 2, id: "s1", name: "Main", size: { width: 1920, height: 1080 } };
const outputs: OutputEntry[] = [
  { revision: "o1", schemaVersion: 2, id: "a", name: "Main", screen: "s1", transport: { type: "idn", sourceName: "Main" } },
  { revision: "o2", schemaVersion: 2, id: "b", name: "Side", screen: "s2", transport: { type: "idn", sourceName: "Side" } },
];

describe("labels", () => {
  it("formats rates and modes", () => {
    expect(fpsLabel({ num: 30000, den: 1001 })).toBe("29.97");
    expect(fpsLabel({ num: 24000, den: 1001 })).toBe("23.976");
    expect(fpsLabel({ num: 25, den: 1 })).toBe("25");
    expect(modeLabel(1280, 720, { num: 60000, den: 1001 })).toBe("720p59.94");
    expect(modeLabel(1080, 1920, { num: 30, den: 1 })).toBe("1080x1920p30");
  });
  it("round-trips colours", () => {
    expect(colorToHex(hexToColor("#ff8000"))).toBe("#ff8000");
    expect(hexToColor("bogus")).toEqual({ r: 0, g: 0, b: 0, a: 1 });
  });
});

describe("modes", () => {
  it("encodes follow and overrides", () => {
    expect(encodeMode(undefined, undefined)).toBe("follow");
    const v = encodeMode({ width: 1280, height: 720 }, { num: 50, den: 1 });
    expect(decodeMode(v)).toEqual({ resolutionOverride: { width: 1280, height: 720 }, frameRateOverride: { num: 50, den: 1 } });
    expect(decodeMode(encodeMode(undefined, { num: 25, den: 1 }))).toEqual({ frameRateOverride: { num: 25, den: 1 } });
  });
  it("lists the current custom mode", () => {
    const cur = encodeMode({ width: 800, height: 600 }, undefined);
    const opts = modeOptions(screen, cur);
    expect(opts[0]!.label).toBe("Follow screen (1080p30)");
    expect(opts.at(-1)!.value).toBe(cur);
  });
  it("setOutputMode clears overrides for follow", () => {
    const o = setOutputMode(outputs[0]!, encodeMode({ width: 1280, height: 720 }, undefined));
    expect(o.resolutionOverride).toEqual({ width: 1280, height: 720 });
    expect("resolutionOverride" in setOutputMode(o, "follow")).toBe(false);
  });
});

describe("draft", () => {
  it("starts clean, tracks screen and output edits, plans requests", () => {
    const d = makeDraft(screen, outputs);
    expect(d.outputs.map((o) => o.key)).toEqual(["a"]);
    expect(isDirty(d)).toBe(false);
    d.screen = { ...d.screen, name: "Main 2" };
    expect(isDirty(d)).toBe(true);
    d.outputs[0]!.doc = setSourceName(d.outputs[0]!.doc, "Main Out");
    const n = newOutput(d.screen, ["Main 2"]);
    expect(n.doc.transport).toEqual({ type: "idn", sourceName: "Main 2 2" });
    d.outputs.push(n);
    const p = planApply(d);
    expect(p.screen).toBe(true);
    expect(p.updates.map((o) => o.key)).toEqual(["a"]);
    expect(p.creates).toHaveLength(1);
    expect(createBody(n.doc)).not.toHaveProperty("id");
    d.outputs[0]!.deleted = true;
    expect(planApply(d).deletes.map((o) => o.key)).toEqual(["a"]);
  });
  it("field order does not make a draft dirty", () => {
    const d = makeDraft(screen, outputs);
    d.screen = JSON.parse(JSON.stringify({ size: d.screen.size, ...d.screen }));
    expect(isDirty(d)).toBe(false);
  });
  it("setting a field to its default is not a change", () => {
    const d = makeDraft(screen, outputs);
    d.screen = { ...d.screen, enabled: true, alpha: "opaque", frameRate: { num: 30, den: 1 }, kind: { type: "audience" } };
    d.outputs[0]!.doc = { ...d.outputs[0]!.doc, videoDelay: 0, enabled: true, target: "full" };
    expect(isDirty(d)).toBe(false);
  });
  it("switching transport keeps the name", () => {
    const o = setTransportKind(outputs[0]!, "omt");
    expect(o.transport).toEqual({ type: "omt", name: "Main" });
  });
  it("IDN keeps its groups through renames and back from another transport keeps the name", () => {
    const withGroups = { ...outputs[0]!, transport: { type: "idn" as const, sourceName: "Main", groups: ["booth"] } };
    expect(setSourceName(withGroups, "Key").transport).toEqual({ type: "idn", sourceName: "Key", groups: ["booth"] });
    expect(setTransportKind(withGroups, "idn").transport).toEqual({ type: "idn", sourceName: "Main", groups: ["booth"] });
    const omt = setTransportKind(withGroups, "omt");
    expect(setTransportKind(omt, "idn").transport).toEqual({ type: "idn", sourceName: "Main" });
  });
});

describe("validation", () => {
  it("checks names, even sizes and rates", () => {
    expect(validateScreen({ ...screen, name: " " }).name).toBeTruthy();
    expect(validateScreen({ ...screen, size: { width: 1279, height: 720 } }).size).toMatch(/even/);
    expect(validateScreen({ ...screen, frameRate: { num: 0, den: 1 } }).frameRate).toBeTruthy();
    expect(validateScreen(screen)).toEqual({});
  });
  it("rejects duplicate IDN names across screens and in the draft", () => {
    const d = makeDraft(screen, outputs);
    d.outputs[0]!.doc = setSourceName(d.outputs[0]!.doc, "side");
    expect(validateOutputs(d.outputs, outputs).a!["transport.sourceName"]).toMatch(/already/);
    d.outputs[0]!.doc = setSourceName(d.outputs[0]!.doc, "Main");
    const n = newOutput(d.screen, []);
    n.doc = setSourceName(n.doc, "MAIN");
    d.outputs.push(n);
    expect(validateOutputs(d.outputs, outputs)[n.key]!["transport.sourceName"]).toMatch(/already/);
    n.doc = { ...n.doc, videoDelay: 31 };
    expect(validateOutputs(d.outputs, outputs)[n.key]!.videoDelay).toBeTruthy();
  });
  it("new outputs are IDN and checked against stored IDN names", () => {
    const d = makeDraft(screen, outputs);
    const n = newOutput(d.screen, []);
    expect(n.doc.transport?.type).toBe("idn");
    n.doc = setSourceName(n.doc, "side");
    d.outputs.push(n);
    expect(validateOutputs(d.outputs, outputs)[n.key]!["transport.sourceName"]).toMatch(/already/);
    n.doc = setSourceName(n.doc, "Lyrics");
    expect(validateOutputs(d.outputs, outputs)[n.key]).toBeUndefined();
  });
  it("its own stored name is not a duplicate", () => {
    const d = makeDraft(screen, outputs);
    expect(validateOutputs(d.outputs, outputs)).toEqual({});
  });
  it("uniqueName counts up", () => {
    expect(uniqueName("Screen", ["screen", "Screen 2"])).toBe("Screen 3");
  });
});

describe("display outputs", () => {
  const hd = { width: 1920, height: 1080, refreshMhz: 60000 };
  const ntsc = { width: 1920, height: 1080, refreshMhz: 59940 };
  const hdmi: DisplayInfo = { id: "HDMI-A-1", index: 0, name: "Example Monitor", connected: true, modes: [hd, ntsc], preferred: hd, boundTo: [] };
  const dp: DisplayInfo = { id: "DP-1", index: 1, name: "DP-1", connected: false, modes: [], boundTo: [] };

  it("labels displays and their modes", () => {
    expect(displayModeLabel(hd)).toBe("1920×1080 @ 60 Hz");
    expect(displayModeLabel(ntsc)).toBe("1920×1080 @ 59.94 Hz");
    expect(displayLabel(hdmi)).toBe("Example Monitor (HDMI-A-1) · 1920×1080");
    expect(displayLabel(dp)).toBe("DP-1 · not connected");
    expect(displayModeOptions(hdmi, "").map((o) => o.label)).toEqual(["Preferred (1920×1080 @ 60 Hz)", "1920×1080 @ 60 Hz", "1920×1080 @ 59.94 Hz"]);
    expect(displayModeOptions(dp, "1280x720@60000").at(-1)!.label).toBe("1280×720 @ 60 Hz (not offered)");
  });

  it("binds a display, keeps an offered mode and names the output", () => {
    const o = setTransportKind(newOutput(screen, []).doc, "display");
    expect(o.transport).toEqual({ type: "display" });
    const a = setDisplay(o, hdmi);
    expect(a.name).toBe("Example Monitor");
    expect(a.transport).toEqual({ type: "display", selector: { connector: "HDMI-A-1", name: "Example Monitor" } });
    expect(displayOf(a)).toBe("HDMI-A-1");
    const b = setDisplayMode(a, displayModeKey(ntsc));
    expect(displayModeValue(b)).toBe("1920x1080@59940");
    expect(displayModeValue(setDisplayMode(b, ""))).toBe("");
    // Moving to a display without that mode drops it (the new display's preferred mode).
    const c = setDisplay(b, dp);
    expect(c.transport).toEqual({ type: "display", selector: { connector: "DP-1" } });
    expect(freeDisplay([dp, hdmi], [])!.id).toBe("HDMI-A-1");
    expect(freeDisplay([dp, hdmi], ["hdmi-a-1"])).toBeUndefined();
  });

  // Monitors of the PC the server runs on (window source): the main one with Oriel on it, and a
  // projector to its right.
  const main: DisplayInfo = { id: "\\\\.\\DISPLAY1", index: 0, name: "Display 1", connected: true, modes: [hd], preferred: hd, source: "window", position: { x: 0, y: 0 }, primary: true, boundTo: [] };
  const proj: DisplayInfo = { id: "\\\\.\\DISPLAY2", index: 1, name: "Display 2", connected: true, modes: [hd], preferred: hd, source: "window", position: { x: 1920, y: 0 }, boundTo: [] };

  it("shows a display output full screen or as a window", () => {
    const a = setDisplay(setTransportKind(newOutput(screen, []).doc, "display"), proj);
    expect(displayFullscreen(a)).toBe(true);
    const w = setDisplayFullscreen(a, false);
    expect(w.transport).toMatchObject({ type: "display", fullscreen: false });
    // Another monitor keeps the window choice; full screen again drops the flag (the default).
    expect(displayFullscreen(setDisplay(w, main))).toBe(false);
    expect(setDisplayFullscreen(w, true).transport).not.toHaveProperty("fullscreen");
    expect(displayLabel(main)).toBe("Display 1 (\\\\.\\DISPLAY1) · 1920×1080 · at 0, 0 · main monitor");
  });

  it("knows which monitors would cover the operator's work", () => {
    // The projector is picked before the main monitor.
    expect(freeDisplay([main, proj], [])!.id).toBe(proj.id);
    expect(freeDisplay([main, proj], [proj.id])!.id).toBe(main.id);
    expect(displayRisk(main, null)).toBe("the main monitor");
    expect(displayRisk(proj, null)).toBeNull();
    // Oriel's window in the middle of the projector (it was dragged there).
    expect(displayRisk(proj, { x: 2880, y: 540 })).toBe("the monitor Oriel is on");
    expect(displayRisk(main, { x: 2880, y: 540 })).toBe("the main monitor");
    expect(displayRisk(hdmi, { x: 10, y: 10 })).toBeNull();
    expect(serverIsLocal("http://localhost:5555")).toBe(true);
    expect(serverIsLocal("http://127.0.0.1:5555/")).toBe(true);
    expect(serverIsLocal("http://[::1]:5555")).toBe(true);
    expect(serverIsLocal("http://192.0.2.10:5555")).toBe(false);
  });

  it("validates the display choice", () => {
    const d = makeDraft(screen, outputs);
    const n = newOutput(screen, []);
    n.doc = setTransportKind(n.doc, "display");
    d.outputs.push(n);
    expect(validateOutputs(d.outputs, outputs)[n.key]!["transport.sourceName"]).toBe("Choose a display.");
    n.doc = setDisplay(n.doc, hdmi);
    expect(validateOutputs(d.outputs, outputs)[n.key]).toBeUndefined();
    const taken: OutputEntry[] = [...outputs, { revision: "o3", schemaVersion: 2, id: "c", name: "Side", screen: "s2", transport: { type: "display", selector: { connector: "HDMI-A-1" } } }];
    expect(validateOutputs(d.outputs, taken)[n.key]!["transport.sourceName"]).toMatch(/already uses/);
  });
});

describe("display outputs across an Apply (batch 7)", () => {
  const hd = { width: 1920, height: 1080, refreshMhz: 60000 };
  const disp = (id: string, extra: Partial<DisplayInfo> = {}): DisplayInfo => ({ id, index: 0, name: id, connected: true, modes: [hd], preferred: hd, boundTo: [], ...extra });
  const out = (id: string, scr: string, transport: OutputEntry["transport"], extra: Partial<OutputEntry> = {}): OutputEntry => ({
    revision: `r-${id}`,
    schemaVersion: 2,
    id,
    name: id,
    screen: scr,
    transport,
    ...extra,
  });

  it("a swap of two displays frees them first (A16-7)", () => {
    const stored = [
      out("a", "s1", { type: "display", selector: { connector: "DISPLAY1" } }),
      out("b", "s1", { type: "display", selector: { connector: "DISPLAY2" } }),
      out("c", "s1", { type: "idn", sourceName: "Main" }),
    ];
    const d = makeDraft(screen, stored);
    d.outputs[0]!.doc = setDisplay(d.outputs[0]!.doc, disp("DISPLAY2"));
    d.outputs[1]!.doc = setDisplay(d.outputs[1]!.doc, disp("DISPLAY1"));
    expect(validateOutputs(d.outputs, stored)).toEqual({});
    expect(mustPark(d).map((o) => o.key)).toEqual(["a", "b"]);
    const parked = parkedDoc(stored[0]!, stored);
    expect(parked.enabled).toBe(false);
    expect(displayOf(parked)).toBe("parked-a");
    // Moving to a free display, or a name nobody else takes: nothing to free.
    const e = makeDraft(screen, stored);
    e.outputs[0]!.doc = setDisplay(e.outputs[0]!.doc, disp("DISPLAY3"));
    expect(mustPark(e)).toEqual([]);
  });

  it("a name or display handed to another output is freed first (A16-7)", () => {
    const stored = [out("a", "s1", { type: "idn", sourceName: "Main" }), out("b", "s1", { type: "idn", sourceName: "Side" })];
    const d = makeDraft(screen, stored);
    d.outputs[0]!.doc = setSourceName(d.outputs[0]!.doc, "Side");
    d.outputs[1]!.doc = setSourceName(d.outputs[1]!.doc, "MAIN");
    expect(mustPark(d).map((o) => o.key)).toEqual(["a", "b"]);
    const p = parkedDoc(stored[0]!, stored);
    expect(p.transport).toEqual({ type: "idn", sourceName: "Main (moving)" });
    expect(p.enabled).toBe(false);
    // A new output takes the display an existing one moves away from.
    const s2 = [out("a", "s1", { type: "display", selector: { connector: "DISPLAY1" } })];
    const e = makeDraft(screen, s2);
    e.outputs[0]!.doc = setDisplay(e.outputs[0]!.doc, disp("DISPLAY2"));
    const n = newOutput(screen, []);
    n.doc = setDisplay(setTransportKind(n.doc, "display"), disp("DISPLAY1"));
    e.outputs.push(n);
    expect(mustPark(e).map((o) => o.key)).toEqual(["a"]);
  });

  it("a duplicated screen's display output takes a free display, switched off (A16-2)", () => {
    const stored = [
      out("a", "s1", { type: "display", selector: { connector: "DISPLAY1" } }),
      out("c", "s1", { type: "idn", sourceName: "Main" }),
      out("x", "s9", { type: "idn", sourceName: "Main copy" }),
    ];
    const r = duplicateOutputs(stored, "s1", "s2", [disp("DISPLAY1"), disp("DISPLAY2"), disp("DISPLAY3", { connected: false })]);
    expect(r.copies.map((c) => c.screen)).toEqual(["s2", "s2"]);
    expect(displayOf(r.copies[0]!)).toBe("DISPLAY2");
    expect(r.copies[0]!.enabled).toBe(false);
    expect(r.copies[0]!.id).toBe("a");
    expect(r.copies[1]!.transport).toEqual({ type: "idn", sourceName: "Main copy 2" });
    expect(r.off).toEqual(["DISPLAY2"]);
    expect(r.skipped).toEqual([]);
    // No free display: that output is left out (the server would refuse it).
    const none = duplicateOutputs(stored, "s1", "s2", [disp("DISPLAY1")]);
    expect(none.copies.map((c) => c.transport?.type)).toEqual(["idn"]);
    expect(none.skipped).toEqual(["a"]);
  });

  it("switching a full screen output or its screen back on asks again (A16-8)", () => {
    const full = out("a", "s1", { type: "display", selector: { connector: "DISPLAY1" } });
    // Unchanged: nothing newly covers.
    expect(newlyCovering(makeDraft(screen, [full]))).toEqual([]);
    // The output was off.
    const d = makeDraft(screen, [{ ...full, enabled: false }]);
    expect(newlyCovering(d)).toEqual([]);
    delete d.outputs[0]!.doc.enabled;
    expect(newlyCovering(d).map((o) => o.key)).toEqual(["a"]);
    // The screen was off.
    const e = makeDraft({ ...screen, enabled: false }, [full]);
    expect(newlyCovering(e)).toEqual([]);
    e.screen.enabled = true;
    expect(newlyCovering(e).map((o) => o.key)).toEqual(["a"]);
    // A window made full screen, and a new output, still count; a switched-off one doesn't.
    const w = makeDraft(screen, [out("a", "s1", { type: "display", selector: { connector: "DISPLAY1" }, fullscreen: false })]);
    w.outputs[0]!.doc = setDisplayFullscreen(w.outputs[0]!.doc, true);
    expect(newlyCovering(w)).toHaveLength(1);
    w.outputs[0]!.doc.enabled = false;
    expect(newlyCovering(w)).toEqual([]);
  });

  it("an unknown window position warns for every monitor (A16-5)", () => {
    expect(displayRisk(disp("DISPLAY2", { source: "window" }), "unknown")).toBe("a monitor Oriel may be on");
    expect(displayRisk(disp("DISPLAY1", { source: "window", primary: true }), "unknown")).toBe("the main monitor");
    // A display the server drives directly has no desktop on it.
    expect(displayRisk(disp("HDMI-A-1", { source: "kms" }), "unknown")).toBeNull();
    expect(displayRisk(disp("DISPLAY2"), null)).toBeNull();
  });
});

describe("screen role (X2-3)", () => {
  it("an audience screen drops a stage screen's mask; Stage again brings it back", () => {
    const base = { id: "s", name: "Stage", kind: { type: "stage" }, mask: "m1" } as unknown as import("$protocol").Screen;
    const aud = withRole(base, base, "audience");
    expect(aud.kind).toEqual({ type: "audience" });
    expect("mask" in aud).toBe(false);
    expect(withRole(aud, base, "stage").mask).toBe("m1");
  });

  it("a signage channel's screen is not an operator screen and keeps its role (H02-7)", () => {
    const sg = { id: "g", name: "Signage: Lobby", kind: { type: "signage", channel: "c1" } } as unknown as import("$protocol").Screen;
    const aud = { id: "a", name: "Main", kind: { type: "audience" } } as unknown as import("$protocol").Screen;
    expect(screenRole(sg)).toBe("signage");
    expect(operatorScreens([aud, sg])).toEqual([aud]);
    expect(withRole(sg, sg, "stage")).toBe(sg);
  });
});

describe("stage mask picker", () => {
  const masks = [
    { revision: "m", schemaVersion: 2, id: "m1", name: "Center 4:3", sortKey: "a", slide: { id: "x", elements: [] } },
    { revision: "m", schemaVersion: 2, id: "m2", name: "", sortKey: "b", slide: { id: "y", elements: [] } },
  ] as unknown as import("$protocol").MaskEntry[];
  const ids = (c: ReturnType<typeof maskChoices>) => c.items.map((i) => i.id);

  it("lists None, the masks and Edit masks…, with the current one checked", () => {
    const c = maskChoices(masks, "m1");
    expect(c.label).toBe("Center 4:3");
    expect(c.missing).toBe(false);
    expect(ids(c)).toEqual(["none", "m1", "m2", "sep", "edit"]);
    expect(c.items.filter((i) => i.checked).map((i) => i.id)).toEqual(["m1"]);
    expect(c.items[2]!.label).toBe("Unnamed mask");
    expect(maskChoices(masks, undefined).label).toBe("None");
    expect(maskChoices(masks, undefined).items[0]!.checked).toBe(true);
  });

  it("keeps a deleted mask as Missing mask, and says Mask while the list loads", () => {
    const c = maskChoices(masks, "gone");
    expect(c.label).toBe("Missing mask");
    expect(c.missing).toBe(true);
    expect(ids(c)).toEqual(["none", "m1", "m2", "gone", "sep", "edit"]);
    const loading = maskChoices(null, "m1");
    expect(loading.label).toBe("Mask");
    expect(loading.missing).toBe(false);
    expect(ids(loading)).toEqual(["none", "sep", "edit"]);
  });

  it("sets and clears a stage screen's mask; an audience screen gets none", () => {
    const stage = { id: "s", name: "Stage", kind: { type: "stage" } } as unknown as import("$protocol").Screen;
    const set = withScreenMask(stage, "m2");
    expect(set.mask).toBe("m2");
    expect("mask" in withScreenMask(set, "none")).toBe(false);
    expect("mask" in withScreenMask(set, null)).toBe(false);
    const aud = { id: "a", name: "Main" } as unknown as import("$protocol").Screen;
    expect("mask" in withScreenMask(aud, "m1")).toBe(false);
  });
});

describe("fill and key outputs (G13)", () => {
  const hd = { width: 1920, height: 1080, refreshMhz: 60000 };
  const fillMon: DisplayInfo = { id: "HDMI-A-1", index: 0, name: "Fill Monitor", connected: true, modes: [hd], preferred: hd, boundTo: [] };
  const keyMon: DisplayInfo = { id: "HDMI-A-2", index: 1, name: "Key Monitor", connected: true, modes: [hd], preferred: hd, boundTo: [] };

  it("sets and reads the target; full is left out", () => {
    const o = newOutput(screen, []).doc;
    expect(outputTarget(o)).toBe("full");
    const f = setOutputTarget(o, "fill");
    expect(f.target).toBe("fill");
    expect(outputTarget(f)).toBe("fill");
    expect("target" in setOutputTarget(f, "full")).toBe(false);
    // A target from a newer server reads as full here.
    expect(outputTarget({ ...o, target: "unknown" })).toBe("full");
  });

  it("adds the key on the next free display with the fill's delivery", () => {
    let fill = setOutputTarget(setDisplay(setTransportKind(newOutput(screen, []).doc, "display"), fillMon), "fill");
    fill = { ...fill, videoDelay: 2 };
    const k = keyOutputFor(fill, ["Fill Monitor"], [fillMon, keyMon], ["HDMI-A-1"]).doc;
    expect(k.target).toBe("key");
    expect(k.screen).toBe(fill.screen);
    expect(displayOf(k)).toBe("HDMI-A-2");
    expect(k.name).toBe("Fill Monitor Key");
    expect(k.videoDelay).toBe(2);
    // No free display: the key output still comes, without a display chosen.
    expect(displayOf(keyOutputFor(fill, [], [fillMon], ["HDMI-A-1"]).doc)).toBe("");
    // An IDN fill gets an IDN key with its own source name.
    const idnFill = setOutputTarget(newOutput(screen, []).doc, "fill");
    const idnKey = keyOutputFor(idnFill, ["Main", "Main Key"], [], []).doc;
    expect(idnKey.transport).toEqual({ type: "idn", sourceName: "Main Key 2" });
  });

  it("hints at the keyer setting, an opaque screen and IDN alpha", () => {
    const d = setDisplay(setTransportKind(newOutput(screen, []).doc, "display"), fillMon);
    expect(fillKeyHint(d, "straight")).toBeNull();
    expect(fillKeyHint(setOutputTarget(d, "fill"), "straight")).toMatch(/Pre Multiplied Key off/);
    expect(fillKeyHint({ ...setOutputTarget(d, "fill"), alphaOverride: { type: "premultiplied" } }, "straight")).toMatch(/Pre Multiplied Key on/);
    expect(fillKeyHint(setOutputTarget(d, "key"), "opaque")).toMatch(/solid white/);
    expect(fillKeyHint(setOutputTarget(newOutput(screen, []).doc, "key"), "straight")).toMatch(/IDN already carries alpha/);
  });
});

describe("arrangements", () => {
  const plain = { ...screen } as unknown as Parameters<typeof setArrangement>[0];

  it("keeps the tile size when the grid changes", () => {
    const g = setArrangement(plain, { kind: "grouped", columns: 2 });
    expect(g.arrangement).toEqual({ kind: "grouped", columns: 2 });
    expect(g.size).toEqual({ width: 3840, height: 1080 });
    const b = setArrangement(g, { bezel: { x: 64, y: 0 } });
    expect(b.size).toEqual({ width: 3904, height: 1080 });
    expect(tileSizeOf(arrangementOf(b), b.size!)).toEqual({ width: 1920, height: 1080 });
    const e = setArrangement(b, { kind: "edgeBlend", overlap: { x: 200, y: 0 } });
    expect(e.size).toEqual({ width: 3640, height: 1080 });
    expect(e.arrangement).toEqual({ kind: "edgeBlend", columns: 2, overlap: { x: 200, y: 0 } });
    // Back to one picture: the screen is one tile again, Single isn't stored.
    const s = setArrangement(e, { kind: "single" });
    expect(s.arrangement).toBeUndefined();
    expect(s.size).toEqual({ width: 1920, height: 1080 });
    expect(setArrangement(plain, { kind: "mirror" }).arrangement).toEqual({ kind: "mirror" });
  });

  it("lays out tiles and labels them", () => {
    const a = arrangementOf(setArrangement(plain, { kind: "grouped", columns: 2, rows: 2, bezel: { x: 0, y: 40 } }));
    const size = canvasForTile(a, { width: 1920, height: 1080 });
    expect(size).toEqual({ width: 3840, height: 2200 });
    const t = tileRects(a, size);
    expect(t.map((r) => r.index)).toEqual([0, 1, 2, 3]);
    expect(t[3]!.x).toBeCloseTo(0.5);
    expect(t[3]!.y).toBeCloseTo(1120 / 2200);
    expect(tileLabel(a, 3)).toBe("Tile 4 (row 2, column 2)");
  });

  it("validates the grid and sets tiles and placeholders", () => {
    const tooSmall = setArrangement({ ...plain, size: { width: 64, height: 64 } }, { kind: "edgeBlend", columns: 2 }, { width: 16, height: 16 });
    expect(validateScreen(setArrangement(tooSmall, { overlap: { x: 20, y: 0 } })).arrangement).toBe("The overlap must be smaller than a tile.");
    expect(validateScreen(setArrangement(plain, { kind: "grouped", columns: 2 })).arrangement).toBeUndefined();
    // Three tiles across: at most half a tile of overlap (F08-3).
    const tile = { width: 2080, height: 1080 };
    expect(validateScreen(setArrangement(plain, { kind: "edgeBlend", columns: 3, overlap: { x: 1200, y: 0 } }, tile)).arrangement).toBe(
      "With three or more tiles in a row or column, the overlap is at most half a tile.",
    );
    expect(validateScreen(setArrangement(plain, { kind: "edgeBlend", columns: 3, overlap: { x: 400, y: 0 } }, tile)).arrangement).toBeUndefined();
    const o = setTile(outputs[0]!, 1);
    expect(o.tile).toBe(1);
    expect("tile" in setTile(o, null)).toBe(false);
    const p = setTransportKind(outputs[0]!, "placeholder");
    expect(p.transport).toEqual({ type: "placeholder" });
    expect(transportLabel("placeholder")).toBe("Placeholder");
  });
});
