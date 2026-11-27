import { describe, expect, it } from "vitest";
import type { LiveSource } from "$protocol";
import { bindingFor, isOmt, inputLine, inputSource, inputStatus, liveFillChoices, liveFillValue, manualSource, sourceHost, sourceName, statusLine, transportTag, visibleSources } from "./sources";

const src = (name: string, more: Partial<LiveSource> = {}): LiveSource => ({ name, idn: true, listed: true, status: "idle", ...more });

describe("live sources", () => {
  it("splits full names", () => {
    expect(sourceName("BOOTH-PC (Cam 1 (wide))")).toBe("Cam 1 (wide)");
    expect(sourceHost("BOOTH-PC (Cam 1)")).toBe("BOOTH-PC");
    expect(sourceName("192.0.2.10:5961")).toBe("192.0.2.10:5961");
    expect(sourceHost("192.0.2.10:5961")).toBeNull();
  });

  it("describes the status", () => {
    expect(statusLine(src("A (B)", { status: "live", width: 1920, height: 1080, frameRate: 29.97002997 }))).toBe("Receiving · 1920×1080 · 29.97 fps");
    expect(statusLine(src("A (B)", { status: "lost" }))).toBe("Lost · retrying");
    expect(statusLine(src("A (B)"))).toBe("Ready");
    expect(statusLine(src("A (B)", { listed: false }))).toBe("Not on the network");
  });

  it("puts the live source first, then sorts by name, and filters", () => {
    const list = [src("H (Zeta)"), src("H (alpha)"), src("LOBBY (Beta)")];
    expect(visibleSources(list, "H (Zeta)", "").map((s) => s.name)).toEqual(["H (Zeta)", "H (alpha)", "LOBBY (Beta)"]);
    expect(visibleSources(list, null, "lobby").map((s) => s.name)).toEqual(["LOBBY (Beta)"]);
    expect(manualSource("  ")).toBeNull();
    expect(manualSource(" 192.0.2.5:5961 ")).toBe("192.0.2.5:5961");
  });

  it("describes logical inputs by their binding", () => {
    const list = [src("BOOTH (Cam 1)", { status: "live", width: 1280, height: 720 }), src("BOOTH (Cam 2)")];
    const cam1 = { source: { type: "idn" as const, sourceName: "BOOTH (Cam 1)" } };
    expect(inputSource(cam1)).toBe("BOOTH (Cam 1)");
    expect(inputLine(cam1, list)).toBe("Cam 1 · Receiving · 1280×720");
    expect(inputStatus(cam1, list)).toBe("live");
    expect(inputLine({}, list)).toBe("Not bound here");
    expect(inputStatus({}, list)).toBe("unbound");
    expect(inputSource({ source: { type: "device", name: "Capture 1" } })).toBeNull();
    expect(inputLine({ source: { type: "device", name: "Capture 1" } }, list)).toBe("Capture device · not supported yet");
    const gone = { source: { type: "idn" as const, sourceName: "192.0.2.9:5961" } };
    expect(inputLine(gone, list)).toBe("192.0.2.9:5961 · Not on the network");
    expect(inputStatus(gone, list)).toBe("idle");
  });

  it("offers inputs first in the fill picker, keeping an unlisted current value", () => {
    const inputs = [{ id: "a", name: "Wide" }, { id: "b", name: "" }];
    const list = [src("BOOTH (Cam 1)")];
    expect(liveFillChoices(inputs, list, {}).map((c) => [c.group, c.value, c.label])).toEqual([
      ["inputs", "input:a", "Wide"],
      ["inputs", "input:b", "Unnamed input"],
      ["sources", "BOOTH (Cam 1)", "BOOTH (Cam 1)"],
    ]);
    expect(liveFillChoices(inputs, list, { source: "192.0.2.9:5961" }).map((c) => c.value)).toEqual(["input:a", "input:b", "192.0.2.9:5961", "BOOTH (Cam 1)"]);
    expect(liveFillChoices(inputs, list, { input: "gone", source: "x" })[0]).toEqual({ group: "inputs", value: "input:gone", label: "Missing input" });
    expect(liveFillValue({ input: "a", source: "BOOTH (Cam 1)" })).toBe("input:a");
    expect(liveFillValue({ source: "BOOTH (Cam 1)" })).toBe("BOOTH (Cam 1)");
    expect(liveFillValue({})).toBe("");
  });

  it("names OMT sources with their omt: prefix and binds them as OMT", () => {
    const omt = "omt:GFX-PC (Lower thirds)";
    expect(isOmt(omt)).toBe(true);
    expect(isOmt("GFX-PC (Lower thirds)")).toBe(false);
    expect(sourceName(omt)).toBe("Lower thirds");
    expect(sourceHost(omt)).toBe("GFX-PC");
    expect(sourceName("omt:192.0.2.7:6400")).toBe("192.0.2.7:6400");
    expect(transportTag(src(omt, { idn: false, omt: true }))).toBe("OMT");
    expect(transportTag(src("BOOTH (Cam 1)"))).toBeNull();
    expect(bindingFor(omt)).toEqual({ type: "omt", name: "GFX-PC (Lower thirds)" });
    expect(bindingFor("omt://192.0.2.7:6400")).toEqual({ type: "omt", name: "omt://192.0.2.7:6400" });
    expect(bindingFor("BOOTH (Cam 1)")).toEqual({ type: "idn", sourceName: "BOOTH (Cam 1)" });
    const bound = { source: { type: "omt" as const, name: "GFX-PC (Lower thirds)" } };
    expect(inputSource(bound)).toBe(omt);
    const list = [src(omt, { idn: false, omt: true, status: "live", width: 1920, height: 1080 })];
    expect(inputLine(bound, list)).toBe("OMT · Lower thirds · Receiving · 1920×1080");
    expect(inputStatus(bound, list)).toBe("live");
    expect(liveFillChoices([], list, {}).map((c) => [c.value, c.label])).toEqual([[omt, "OMT: GFX-PC (Lower thirds)"]]);
  });
});
