import { describe, expect, it } from "vitest";
import { encoderChoices, encoderProblem, encodersFor, encodersLine, formatMbps, parseMbps, presetMbps, qualityChoices, sameSettings } from "./mediaPrep";

describe("media preparation settings", () => {
  it("shows the presets' bit rates at 1080p30 (HEVC 0.6 of H.264's)", () => {
    expect([presetMbps("standard", "h264"), presetMbps("high", "h264"), presetMbps("maximum", "h264")]).toEqual([12, 20, 30]);
    expect([presetMbps("standard", "hevc"), presetMbps("high", "hevc"), presetMbps("maximum", "hevc")]).toEqual([7, 12, 18]);
    expect(qualityChoices("h264").map((c) => c.label)).toEqual([
      "Standard (12 Mb/s at 1080p30)",
      "High (20 Mb/s at 1080p30)",
      "Maximum (30 Mb/s at 1080p30)",
    ]);
  });

  it("names the encoders this box has", () => {
    const both = { gpuEncoder: "VA-API", cpuEncoder: "OpenH264 (CPU)" };
    expect(encoderChoices(both).map((c) => c.label)).toEqual(["Auto (GPU)", "GPU: VA-API", "CPU: OpenH264"]);
    expect(encodersLine(both)).toBe("This box: GPU VA-API · CPU OpenH264");
    expect(encoderProblem("auto", both)).toBeNull();
    // HEVC on Ubuntu's FFmpeg without a GPU encoder: nothing.
    const none = {};
    expect(encoderChoices(none).map((c) => c.label)).toEqual([
      "Auto (none here)",
      "GPU (none on this box)",
      "CPU (none on this box for this codec)",
    ]);
    expect(encodersLine(none)).toBe("This box: no GPU encoder · no CPU encoder");
    expect(encoderProblem("auto", none)).toMatch(/no encoder/);
    const cpuOnly = { cpuEncoder: "OpenH264 (CPU)" };
    expect(encoderChoices(cpuOnly)[0]?.label).toBe("Auto (CPU)");
    expect(encoderProblem("gpu", cpuOnly)).toMatch(/no GPU encoder/);
    expect(encoderProblem("cpu", { gpuEncoder: "NVENC" })).toMatch(/no CPU encoder/);
  });

  it("reads an explicit bit rate in Mb/s", () => {
    expect(parseMbps("")).toBeUndefined();
    expect(parseMbps("  ")).toBeUndefined();
    expect(parseMbps("12")).toBe(12_000);
    expect(parseMbps("12,5")).toBe(12_500);
    expect(parseMbps("0.2")).toBe(1_000);
    expect(parseMbps("250")).toBe(60_000);
    expect(parseMbps("fast")).toBeNull();
    expect(parseMbps("-3")).toBeNull();
    expect(formatMbps(undefined)).toBe("");
    expect(formatMbps(12_000)).toBe("12");
    expect(formatMbps(12_500)).toBe("12.5");
    // Shown as saved, read back the same (a saved form stays clean).
    expect(formatMbps(7_250)).toBe("7.25");
    expect(formatMbps(12_345)).toBe("12.345");
    for (const k of [1_000, 7_250, 7_300, 12_345, 59_999, 60_000]) expect(parseMbps(formatMbps(k))).toBe(k);
  });

  it("takes the encoders of the codec being edited", () => {
    const st = {
      gpuEncoder: "VA-API",
      cpuEncoder: "OpenH264 (CPU)",
      codecEncoders: { h264: { gpu: "VA-API", cpu: "OpenH264 (CPU)" }, hevc: { gpu: "VA-API" } },
    };
    expect(encodersFor(st, "h264")).toEqual({ gpuEncoder: "VA-API", cpuEncoder: "OpenH264 (CPU)" });
    // HEVC chosen, not saved yet: no CPU encoder on this box (Ubuntu's FFmpeg).
    const hevc = encodersFor(st, "hevc");
    expect(hevc).toEqual({ gpuEncoder: "VA-API" });
    expect(encoderChoices(hevc)[2]?.label).toBe("CPU (none on this box for this codec)");
    expect(encoderProblem("cpu", hevc)).toMatch(/no CPU encoder/);
    // A server without the per-codec list: the saved codec's.
    expect(encodersFor({ cpuEncoder: "OpenH264 (CPU)" }, "hevc")).toEqual({ cpuEncoder: "OpenH264 (CPU)" });
  });

  it("compares settings whatever their keys' order", () => {
    expect(sameSettings({ a: 1, videoKbps: 9000, b: "x" }, { b: "x", a: 1, videoKbps: 9000 })).toBe(true);
    expect(sameSettings({ a: 1, videoKbps: undefined }, { a: 1 })).toBe(true);
    expect(sameSettings({ a: 1, videoKbps: 9000 }, { a: 1 })).toBe(false);
  });
});
