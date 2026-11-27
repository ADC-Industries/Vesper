import { describe, expect, it } from "vitest";
import type { MediaTransport } from "$protocol";
import { formatTime, fraction, neighbour, positionAt, rateLabel, seekTarget, stepAction, timeLabel } from "./transport";

const t = (o: Partial<MediaTransport>): MediaTransport => ({
  layer: "media",
  asset: "a",
  name: "Clip",
  state: "playing",
  position: 10,
  duration: 30,
  loop: false,
  rate: 1,
  hw: true,
  ...o,
});

describe("transport", () => {
  it("formats times", () => {
    expect(formatTime(0)).toBe("0:00");
    expect(formatTime(9.9)).toBe("0:09");
    expect(formatTime(75)).toBe("1:15");
    expect(formatTime(3725)).toBe("1:02:05");
    expect(formatTime(-3)).toBe("0:00");
    expect(formatTime(null)).toBe("0:00");
    expect(timeLabel(12, 90)).toBe("0:12 / 1:30");
    expect(timeLabel(12, null)).toBe("0:12");
  });

  it("interpolates playing positions between states", () => {
    expect(positionAt(t({}), 500)).toBeCloseTo(10.5);
    expect(positionAt(t({ rate: 2 }), 500)).toBeCloseTo(11);
    expect(positionAt(t({ state: "paused" }), 5000)).toBe(10);
    expect(positionAt(t({ state: "stopped", position: 30 }), 5000)).toBe(30);
    // A single play holds at the end, a loop wraps.
    expect(positionAt(t({ position: 29.5 }), 2000)).toBe(30);
    expect(positionAt(t({ position: 29.5, loop: true }), 1000)).toBeCloseTo(0.5);
    expect(positionAt(t({ duration: null }), 1000)).toBeCloseTo(11);
  });

  it("maps the scrub bar", () => {
    expect(fraction(15, 30)).toBe(0.5);
    expect(fraction(40, 30)).toBe(1);
    expect(fraction(5, null)).toBe(0);
    expect(seekTarget(150, 100, 200, 60)).toBe(15);
    expect(seekTarget(50, 100, 200, 60)).toBe(0);
    expect(seekTarget(400, 100, 200, 60)).toBe(60);
    expect(seekTarget(150, 100, 0, 60)).toBe(0);
  });
});

describe("neighbour", () => {
  it("steps and wraps", () => {
    expect(neighbour(["a", "b", "c"], "b", 1)).toBe("c");
    expect(neighbour(["a", "b", "c"], "c", 1)).toBe("a");
    expect(neighbour(["a", "b", "c"], "a", -1)).toBe("c");
  });
  it("starts at an end when the current item is elsewhere", () => {
    expect(neighbour(["a", "b"], "x", 1)).toBe("a");
    expect(neighbour(["a", "b"], null, -1)).toBe("b");
    expect(neighbour([], "a", 1)).toBeNull();
  });
});

describe("rateLabel", () => {
  it("shows whole rates plainly and NTSC rates with two decimals", () => {
    expect(rateLabel(30, 1)).toBe("30");
    expect(rateLabel(30000, 1001)).toBe("29.97");
    expect(rateLabel(60000, 1001)).toBe("59.94");
  });
});

describe("stepAction", () => {
  const bin = ["v1", "v2", "v3"];
  it("Previous on a foreground (Slide-layer) video restarts it, never a bin video (A14-2)", () => {
    const fg = { asset: "cue", layer: "slide" };
    expect(stepAction({ d: -1, target: fg, position: 1, list: bin })).toEqual({ kind: "restart" });
    expect(stepAction({ d: -1, target: fg, position: 10, list: bin })).toEqual({ kind: "restart" });
    expect(stepAction({ d: 1, target: fg, position: 1, list: bin })).toBeNull();
  });

  it("steps the bin on the Media layer, restarting after RESTART_AFTER", () => {
    const bg = { asset: "v2", layer: "media" };
    expect(stepAction({ d: -1, target: bg, position: 1, list: bin })).toEqual({ kind: "trigger", asset: "v1" });
    expect(stepAction({ d: -1, target: bg, position: 5, list: bin })).toEqual({ kind: "restart" });
    expect(stepAction({ d: 1, target: bg, position: 5, list: bin })).toEqual({ kind: "trigger", asset: "v3" });
    expect(stepAction({ d: 1, target: null, position: 0, list: bin })).toEqual({ kind: "trigger", asset: "v1" });
    expect(stepAction({ d: -1, target: bg, position: 1, list: [] })).toEqual({ kind: "restart" });
    expect(stepAction({ d: 1, target: null, position: 0, list: [] })).toBeNull();
  });

  it("an audio playlist steps on the server", () => {
    const a = { asset: "t1", layer: "audio" };
    expect(stepAction({ d: 1, target: a, position: 1, list: [], serverSteps: true })).toEqual({ kind: "server", next: true });
    expect(stepAction({ d: -1, target: a, position: 9, list: [], serverSteps: true })).toEqual({ kind: "restart" });
  });
});
