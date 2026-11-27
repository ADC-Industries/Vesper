import { describe, expect, it } from "vitest";
import type { MediaAsset, MediaPrepJob } from "$protocol";
import { formatGain, formatLufs, formatTime, parseTime, prepBadge, prepSummary, trimProblem, trimSpan } from "./prep";

const job = (step: MediaPrepJob["step"], progress = 0, error?: string): MediaPrepJob => ({
  asset: "a",
  collection: "c",
  name: "Clip.mov",
  step,
  progress,
  ...(error ? { error } : {}),
});

const converted: Pick<MediaAsset, "kind" | "prep"> = {
  kind: "video",
  prep: {
    loudness: -23,
    peak: -12.4,
    gainDb: 7,
    posterAt: 1,
    analysed: "abc",
    original: {
      location: { type: "library", id: "1", sha256: "x" },
      probe: { codec: "mpeg4", width: 320, height: 180, frameRate: { num: 30000, den: 1001 } },
      reasons: ["MPEG-4 Part 2 video isn't decoded by the GPU", "sound at 44.1 kHz (the house rate is 48 kHz)"],
      format: "H.264 320×180 25 fps, AAC 48 kHz",
    },
  },
};

describe("media preparation", () => {
  it("formats loudness, gain and times", () => {
    expect(formatLufs(-23)).toBe("−23.0 LUFS");
    expect(formatGain(7)).toBe("+7.0 dB");
    expect(formatGain(-3.46)).toBe("−3.5 dB");
    expect(formatGain(0.01)).toBe("0 dB");
    expect(formatTime(62.54)).toBe("1:02.5");
    expect(formatTime(5)).toBe("0:05.0");
    expect(parseTime("1:02.5")).toBe(62.5);
    expect(parseTime("62")).toBe(62);
    expect(parseTime("abc")).toBeNull();
  });

  it("sums up a converted file, a running job and a failure", () => {
    const s = prepSummary(converted, null);
    expect(s.label).toBe("Converted to the house format");
    expect(s.tone).toBe("ok");
    expect(s.converted).toBe(true);
    expect(s.reasons).toHaveLength(2);
    expect(s.facts.map((f) => f.k)).toEqual(["Loudness", "Level at playback", "Peak", "Poster frame", "Plays", "Original"]);
    expect(s.facts.find((f) => f.k === "Original")!.v).toBe("MPEG4 320×180 29.97 fps");
    expect(prepSummary(converted, job("converting", 0.42)).label).toBe("Converting… 42 %");
    expect(prepSummary({ kind: "video" }, job("waitingForLive")).tone).toBe("busy");
    const failed = prepSummary({ kind: "video", prep: { error: "FFmpeg failed" } }, null);
    expect([failed.label, failed.tone, failed.error]).toEqual(["Preparation failed", "warn", "FFmpeg failed"]);
    expect(prepSummary({ kind: "video", prep: { analysed: "x" } }, null).label).toBe("Ready: plays as it is");
    expect(prepSummary({ kind: "video" }, null).label).toBe("Not prepared yet");
  });

  it("badges tiles while a job runs or after a failure", () => {
    expect(prepBadge(job("converting", 0.5), {})).toEqual({ text: "PREPARING 50%", warn: false });
    expect(prepBadge(job("waitingForLive"), {})).toEqual({ text: "AFTER THE SHOW", warn: false });
    expect(prepBadge(null, { prep: { error: "x" } })).toEqual({ text: "PREP FAILED", warn: true });
    expect(prepBadge(job("done", 1), {})).toBeNull();
  });

  it("places and checks the trim", () => {
    expect(trimSpan(10, 2, 8)).toEqual({ a: 0.2, b: 0.8 });
    expect(trimSpan(10, null, null)).toEqual({ a: 0, b: 1 });
    expect(trimSpan(0, 2, 8)).toEqual({ a: 0, b: 1 });
    expect(trimProblem(10, 2, 8)).toBeNull();
    expect(trimProblem(10, 8, 2)).toMatch(/after the in point/);
    expect(trimProblem(10, null, 12)).toMatch(/after the end/);
    expect(trimProblem(null, -1, null)).toMatch(/before the start/);
  });
});
