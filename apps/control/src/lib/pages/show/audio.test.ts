import { describe, expect, it, vi } from "vitest";
import type { MediaBinDetail, MediaLayerState, PlaylistDetail, PlaylistTreeNode } from "$protocol";
import {
  MeterBallistics,
  audioPlaylists,
  audioTransport,
  binRows,
  deviceOptions,
  dbToFader,
  durationLabel,
  faderToDb,
  formatDb,
  isPlayingRow,
  meterFraction,
  meterZone,
  playlistRows,
  roundDb,
  throttle,
  withoutAudioPlaylists,
} from "./audio";

describe("dB", () => {
  it("formats", () => {
    expect(formatDb(0)).toBe("0.0");
    expect(formatDb(-0.04)).toBe("0.0");
    expect(formatDb(-6.26)).toBe("-6.3");
    expect(formatDb(3)).toBe("+3.0");
    expect(formatDb(-96)).toBe("-∞");
    expect(formatDb(-Infinity)).toBe("-∞");
  });

  it("maps the fader both ways", () => {
    expect(faderToDb(0)).toBe(-96);
    expect(faderToDb(0.75)).toBeCloseTo(0);
    expect(faderToDb(1)).toBeCloseTo(12);
    expect(faderToDb(0.375)).toBeCloseTo(-30);
    for (const db of [-48, -12, -0.5, 0, 6, 12]) expect(faderToDb(dbToFader(db))).toBeCloseTo(db);
    expect(dbToFader(-96)).toBe(0);
    expect(dbToFader(-70)).toBe(0);
    expect(dbToFader(40)).toBe(1);
    expect(roundDb(-3.14159)).toBe(-3.1);
    expect(roundDb(-200)).toBe(-96);
  });
});

describe("meters", () => {
  it("scales and colours", () => {
    expect(meterFraction(-60)).toBe(0);
    expect(meterFraction(-90)).toBe(0);
    expect(meterFraction(-30)).toBeCloseTo(0.5);
    expect(meterFraction(3)).toBe(1);
    expect(meterZone(-30)).toBe("green");
    expect(meterZone(-12)).toBe("yellow");
    expect(meterZone(-3)).toBe("red");
  });

  it("rises at once, falls slowly and holds the peak", () => {
    const m = new MeterBallistics(20, 1000);
    m.setLevels([-10, -20], 0);
    m.step(0);
    expect(m.channels.map((c) => c.level)).toEqual([-10, -20]);
    m.setLevels([-60, -60], 0);
    m.step(200);
    m.step(400);
    expect(m.step(500)).toBe(true);
    expect(m.channels[0]!.level).toBeCloseTo(-20); // 10 dB in 0.5 s at 20 dB/s
    expect(m.channels[0]!.peak).toBe(-10); // held
    m.step(1500);
    expect(m.channels[0]!.peak).toBeLessThan(-10);
    for (let t = 1500; t < 10000; t += 100) m.step(t);
    expect(m.channels[0]!.level).toBe(-60);
    expect(m.step(10100)).toBe(false);
  });

  it("follows the channel count", () => {
    const m = new MeterBallistics();
    m.setLevels([-10, -10, -10], 0);
    expect(m.channels).toHaveLength(3);
    m.setLevels([-10], 1);
    expect(m.channels).toHaveLength(1);
  });
});

const layer = (asset: string): MediaLayerState => ({
  asset,
  name: "Song",
  kind: "audio",
  online: true,
  playback: { state: "playing", position: 12, duration: 180, loop: false, rate: 1, hw: false },
});

describe("audio panel rows", () => {
  const bin = {
    id: "b",
    name: "Bin",
    path: [],
    kind: "manual",
    browser: "audio",
    items: [
      { id: "i1", name: "One", kind: "audio", asset: "a1", availability: "online", enabled: true, duration: 65 },
      { id: "i2", name: "Two", kind: "audio", asset: "a2", availability: "missing", enabled: true, duration: null },
    ],
  } as MediaBinDetail;
  const pl = {
    id: "p",
    name: "Pl",
    items: [
      { id: "x1", displayName: "One", enabled: true, kind: { type: "media", asset: "a1", layer: "audio" }, presentation: null },
      { id: "x2", displayName: "Hdr", enabled: true, kind: { type: "header", name: "Hdr" }, presentation: null },
    ],
  } as unknown as PlaylistDetail;

  it("builds rows", () => {
    const rows = binRows(bin);
    expect(rows[0]).toEqual({ id: "i1", name: "One", asset: "a1", duration: 65, online: true });
    expect(rows[1]!.online).toBe(false);
    const prow = playlistRows(pl, new Map([["a1", 65]]));
    expect(prow[0]!.duration).toBe(65);
    expect(prow[1]!.asset).toBeNull();
  });

  it("finds the playing row", () => {
    const [r1, r2] = binRows(bin);
    expect(isPlayingRow(r1!, { kind: "bin", id: "b" }, layer("a1"), null)).toBe(true);
    expect(isPlayingRow(r2!, { kind: "bin", id: "b" }, layer("a1"), null)).toBe(false);
    // Played from a playlist: the bin does not claim it.
    expect(isPlayingRow(r1!, { kind: "bin", id: "b" }, layer("a1"), { playlist: "p", item: "x1" })).toBe(false);
    const [p1, p2] = playlistRows(pl, new Map());
    expect(isPlayingRow(p1!, { kind: "playlist", id: "p" }, layer("a1"), { playlist: "p", item: "x1" })).toBe(true);
    expect(isPlayingRow(p2!, { kind: "playlist", id: "p" }, layer("a1"), { playlist: "p", item: "x1" })).toBe(false);
    expect(isPlayingRow(p1!, { kind: "playlist", id: "q" }, layer("a1"), { playlist: "p", item: "x1" })).toBe(false);
    expect(isPlayingRow(p1!, { kind: "playlist", id: "p" }, null, { playlist: "p", item: "x1" })).toBe(false);
  });

  it("labels durations", () => {
    expect(durationLabel(65)).toBe("1:05");
    expect(durationLabel(null)).toBe("–:––");
  });

  it("splits audio playlists from the tree", () => {
    const tree: PlaylistTreeNode[] = [
      { id: "s", name: "Sunday", kind: "playlist", purpose: "service", itemCount: 3, children: [] },
      {
        id: "f",
        name: "Music",
        kind: "folder",
        purpose: null,
        itemCount: 0,
        children: [{ id: "a", name: "Walk-in", kind: "playlist", purpose: "audio", itemCount: 5, children: [] }],
      },
    ];
    expect(audioPlaylists(tree)).toEqual([{ id: "a", name: "Walk-in", label: "Music / Walk-in", itemCount: 5 }]);
    const rest = withoutAudioPlaylists(tree);
    expect(rest).toHaveLength(2);
    expect(rest[1]!.children).toEqual([]);
  });

  it("turns the audio layer into a transport", () => {
    expect(audioTransport(null)).toBeNull();
    expect(audioTransport(layer("a1"))).toMatchObject({ layer: "audio", asset: "a1", position: 12, duration: 180, state: "playing" });
  });
});

describe("throttle", () => {
  it("sends the first value at once and the latest one after the wait", () => {
    vi.useFakeTimers();
    const got: number[] = [];
    const t = throttle((v: number) => got.push(v), 50);
    t(1);
    t(2);
    t(3);
    expect(got).toEqual([1]);
    vi.advanceTimersByTime(60);
    expect(got).toEqual([1, 3]);
    t(4);
    t.flush();
    expect(got).toEqual([1, 3, 4]);
    vi.useRealTimers();
  });
});

describe("device options", () => {
  it("lists none, the devices, and a configured device that is gone", () => {
    const opts = deviceOptions(
      [
        { name: "default", description: "Default output", available: true },
        { name: "hw:1,0", description: "USB Audio", available: false, problem: "busy", channels: 2 },
      ],
      "hw:9,0",
    );
    expect(opts.map((o) => o.value)).toEqual(["", "default", "hw:1,0", "hw:9,0"]);
    expect(opts[2]).toMatchObject({ label: "USB Audio (hw:1,0) · 2 ch (unavailable)", disabled: true, title: "busy" });
    expect(opts[3]!.label).toBe("hw:9,0 (not found)");
    expect(deviceOptions([{ name: "hw:1,0", description: "", available: false }], "hw:1,0")[1]!.disabled).toBe(false);
  });
});
