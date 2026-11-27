import { describe, expect, it } from "vitest";
import type { ServerDetails, SiteDeviceMachine } from "$protocol";
import { detailsLines, detailsShort, gbText, machineLines, uptimeText } from "./details";

const d: ServerDetails = {
  name: "Chapel Booth",
  version: "0.1.0",
  os: "Ubuntu 24.04.1 LTS",
  cpu: "Example CPU 8-Core",
  cpus: 16,
  gpu: "Example GPU (Vulkan)",
  uptimeSeconds: 2 * 3600 + 5 * 60,
  products: ["Vesper", "Codex"],
};

describe("this Vesper's details", () => {
  it("says how long it has been up", () => {
    expect(uptimeText(12 * 60)).toBe("12 min");
    expect(uptimeText(2 * 3600 + 5 * 60)).toBe("2 h 5 min");
    expect(uptimeText(3 * 86400 + 4 * 3600)).toBe("3 d 4 h");
  });

  it("lists the version, CPU, GPU and products", () => {
    expect(detailsShort(d).text).toBe("Vesper 0.1.0 · up 2 h 5 min");
    expect(detailsLines(d)).toEqual([
      "Chapel Booth: Vesper 0.1.0 on Ubuntu 24.04.1 LTS",
      "CPU: Example CPU 8-Core (16 CPUs)",
      "GPU: Example GPU (Vulkan)",
      "Up for 2 h 5 min",
      "Runs Vesper, Codex",
    ]);
    expect(detailsLines({ ...d, cpu: undefined, cpus: 1, gpu: undefined })[1]).toBe("CPU: 1 CPU");
    expect(detailsLines({ ...d, gpu: undefined })[2]).toBe("GPU: not rendering yet");
  });
});

describe("another machine's details", () => {
  const now = Date.parse("2026-11-24T12:00:00Z");
  const m: SiteDeviceMachine = {
    cpu: "Example CPU 8-Core",
    cpus: 16,
    gpus: ["Example GPU"],
    os: "Ubuntu",
    osVersion: "24.04",
    kernel: "6.8.0",
    bootedAt: "2026-11-21T08:00:00Z",
    memoryTotal: 32 * 2 ** 30,
    memoryFree: 12.5 * 2 ** 30,
    reportedAt: "2026-11-24T11:59:50Z",
  };

  it("says what its Node reported, as this Vesper's card does", () => {
    expect(gbText(32 * 2 ** 30)).toBe("32.0 GB");
    expect(machineLines(m, { online: true, nodeStartedAt: "2026-11-21T08:01:00Z", now })).toEqual([
      "System: Ubuntu 24.04, kernel 6.8.0",
      "CPU: Example CPU 8-Core (16 CPUs)",
      "GPU: Example GPU",
      "Memory: 12.5 GB free of 32.0 GB (updated up to 30 s ago)",
      "Up for 3 d 4 h",
    ]);
  });

  it("says when its Node restarted since the machine started", () => {
    const lines = machineLines(m, { online: true, nodeStartedAt: "2026-11-24T09:55:00Z", now });
    expect(lines.at(-1)).toBe("Up for 3 d 4 h (IDN Node for 2 h 5 min)");
  });

  it("shows only what lasts for a machine offline", () => {
    expect(machineLines({ ...m, gpus: ["A", "B"] }, { online: false, now })).toEqual([
      "System: Ubuntu 24.04, kernel 6.8.0",
      "CPU: Example CPU 8-Core (16 CPUs)",
      "GPUs: A, B",
      "Memory: 32.0 GB",
    ]);
  });

  it("shows nothing for what wasn't reported, never zeros", () => {
    expect(machineLines({ gpus: [] }, { online: true, now })).toEqual([]);
    expect(machineLines({ gpus: [], os: "Windows 11 Pro", cpus: 8 }, { online: true, now })).toEqual(["System: Windows 11 Pro", "CPU: 8 CPUs"]);
  });
});
