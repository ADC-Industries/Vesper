import { describe, expect, it } from "vitest";
import type { CaptionsState } from "$protocol";
import { captionStatus, downloadBar, lastLine, modelOptions, processorInUse, sourceOf, sourceValue, withDefaults } from "./captions";

const base: CaptionsState = { enabled: true, status: "running", detail: null, progress: null, downloaded: null, downloadSize: null, model: "base.en", device: "CPU (6 threads)", gpu: false, note: null, load: 0.31, lines: [] };

describe("captions", () => {
  it("says what the engine is doing", () => {
    expect(captionStatus(null)).toEqual({ cls: "off", text: "Off" });
    expect(captionStatus({ ...base, enabled: false, status: "off" }).text).toBe("Off");
    expect(captionStatus(base)).toEqual({ cls: "ok", text: "Listening · base.en · load 31 %" });
    expect(captionStatus({ ...base, model: null, load: null, detail: "lines come from another captioner" }).text).toBe("lines come from another captioner");
    expect(captionStatus({ ...base, status: "downloading", progress: 42 })).toEqual({ cls: "busy", text: "Downloading the model · 42 %" });
    expect(captionStatus({ ...base, status: "downloading", progress: 42, detail: "downloading the small.en model" }).text).toBe("Downloading the small.en model · 42 %");
    expect(captionStatus({ ...base, status: "starting", detail: "loading the base.en model" }).text).toBe("Loading the base.en model");
    expect(captionStatus({ ...base, status: "error", detail: "no model" })).toEqual({ cls: "bad", text: "Stopped: no model" });
  });

  it("measures a model download for the progress bar", () => {
    expect(downloadBar(base)).toBeNull();
    expect(downloadBar(null)).toBeNull();
    const dl: CaptionsState = { ...base, status: "downloading", detail: "downloading the small.en model", progress: 50, downloaded: 233_000_000, downloadSize: 466_000_000 };
    expect(downloadBar(dl)).toEqual({ fraction: 0.5, text: "233 MB of 466 MB" });
    expect(downloadBar({ ...dl, enabled: false })).toBeNull();
    // A GB-class model in GB; the size unknown: no measure, the bytes so far.
    expect(downloadBar({ ...dl, progress: 25, downloaded: 400_000_000, downloadSize: 1_600_000_000 })!.text).toBe("0.4 GB of 1.6 GB");
    expect(downloadBar({ ...dl, progress: null, downloaded: 12_400_000, downloadSize: null })).toEqual({ fraction: null, text: "12 MB so far" });
    // An older server: only the percentage.
    expect(downloadBar({ ...dl, downloaded: null, downloadSize: null, progress: 42 })).toEqual({ fraction: 0.42, text: "" });
  });

  it("shows the newest line", () => {
    expect(lastLine(base)).toBe("");
    expect(lastLine({ ...base, lines: [{ line: 1, text: "One.", final: true }, { line: 2, text: "Two", final: false }] })).toBe("Two");
  });

  it("round-trips the source", () => {
    for (const s of [{ type: "program" }, { type: "external" }, { type: "liveSource", name: "HOST (Pulpit)" }] as const) {
      expect(sourceOf(sourceValue(s))).toEqual(s);
    }
    expect(sourceValue(undefined)).toBe("program");
  });

  it("offers multilingual models only for other languages", () => {
    const models = [
      { name: "base.en", sizeMb: 142, note: "a small machine", downloaded: true },
      { name: "large-v3-turbo", sizeMb: 1600, note: "a GPU", downloaded: false },
    ];
    expect(modelOptions(models, "en").map((o) => o.value)).toEqual(["auto", "base.en", "large-v3-turbo"]);
    expect(modelOptions(models, "es").map((o) => o.value)).toEqual(["auto", "large-v3-turbo"]);
    expect(modelOptions(models, "en")[1]!.label).toBe("base.en · 142 MB · a small machine · on the server");
    expect(modelOptions(models, "en")[2]!.label).toContain("1.6 GB");
  });

  it("says what speech recognition runs on", () => {
    expect(processorInUse(base)).toBe("CPU (6 threads)");
    expect(processorInUse({ ...base, device: "Synthetic GPU 1", gpu: true })).toBe("GPU · Synthetic GPU 1");
    expect(processorInUse({ ...base, status: "starting" })).toBeNull();
    expect(processorInUse({ ...base, enabled: false })).toBeNull();
    expect(processorInUse(null)).toBeNull();
    // A server from before GPU captions: no `gpu`, the device alone.
    const { gpu: _gpu, note: _note, ...old } = base;
    expect(processorInUse(old as CaptionsState)).toBe("CPU (6 threads)");
  });

  it("fills in the defaults the server leaves out", () => {
    const c = withDefaults({ schemaVersion: 2, enabled: true, maxLines: 3 });
    expect(c).toMatchObject({ enabled: true, maxLines: 3, language: "en", model: "auto", processor: "cpu", source: { type: "program" }, audience: true, stage: false });
    expect(withDefaults(c)).toEqual(c);
  });
});
