import { describe, expect, it } from "vitest";
import type { LauncherEntry, LauncherProduct } from "$protocol";
import { attentionCount, byProduct, bytes, productName, roleLabel, canAct, healthLine, outcomeText, pendingFor, platformLabel, seenAgo, stateLabel, stateTone, updateFor } from "./updates";

const product = (p: Partial<LauncherProduct> = {}): LauncherProduct => ({ product: "vesper", name: "Vesper", version: "0.1.0", state: "running", restarts: 0, ...p });
const entry = (e: Partial<LauncherEntry> = {}, p: Partial<LauncherProduct> = {}): LauncherEntry => ({
  report: { machineId: "m1", machineName: "Booth A", launcherVersion: "0.1.0", platform: "linux-x86_64", roles: [], products: [product(p)], health: { cpus: 8, gpus: [] } },
  lastSeenSeconds: 3,
  online: true,
  pending: [],
  updates: [],
  ...e,
});

describe("Site ▸ Updates words", () => {
  it("names states and outcomes", () => {
    expect(stateLabel("rollingBack")).toBe("Rolling back");
    expect(stateTone("running")).toBe("ok");
    expect(stateTone("updating")).toBe("warn");
    expect(stateTone("failed")).toBe("error");
    expect(outcomeText({ from: "0.1.0", to: "0.2.0", result: "installed", message: "", at: "" })).toBe("Updated 0.1.0 → 0.2.0");
    expect(outcomeText({ from: "0.1.0", to: "0.2.0", result: "rolledBack", message: "x", at: "" })).toBe("0.2.0 didn't come up healthy: back on 0.1.0");
    expect(outcomeText({ from: "0.2.0", to: "0.1.0", result: "installed", message: "rolled back by hand", at: "" })).toBe("Rolled back 0.2.0 → 0.1.0");
    expect(outcomeText({ to: "0.2.0", result: "failed", message: "bad signature", at: "" })).toBe("0.2.0 failed: bad signature");
  });

  it("finds updates and queued actions; acts only when it can", () => {
    const e = entry({ updates: [{ product: "vesper", version: "0.2.0", package: "p1" }] });
    expect(updateFor(e, "vesper")).toBe("0.2.0");
    expect(updateFor(e, "oriel")).toBeNull();
    expect(canAct(e, e.report.products[0]!)).toBe(true);
    const queued = entry({ pending: [{ type: "update", product: "vesper", version: "0.2.0" }] });
    expect(pendingFor(queued, "vesper")).toBe("Update to 0.2.0 queued");
    expect(canAct(queued, queued.report.products[0]!)).toBe(false);
    expect(pendingFor(entry({ pending: [{ type: "rollback", product: "vesper" }] }), "vesper")).toBe("Roll back queued");
    const offline = entry({ online: false });
    expect(canAct(offline, offline.report.products[0]!)).toBe(false);
    const busy = entry({}, { state: "updating" });
    expect(canAct(busy, busy.report.products[0]!)).toBe(false);
  });

  it("counts machines needing attention", () => {
    expect(attentionCount([entry(), entry({ online: false }), entry({}, { state: "failed" }), entry({}, { lastUpdate: { to: "2", result: "failed", message: "", at: "" } })])).toBe(3);
  });

  it("formats numbers, platforms and times", () => {
    expect(bytes(512)).toBe("512 B");
    expect(bytes(48_211_456)).toBe("46.0 MB");
    expect(bytes(undefined)).toBe("–");
    expect(platformLabel("windows-x86_64")).toBe("Windows (x86_64)");
    expect(roleLabel("siteManager")).toBe("Site Manager");
    expect(roleLabel("somethingNew")).toBe("somethingNew");
    expect(productName("oriel")).toBe("Vesper Oriel");
    expect(seenAgo(10)).toBe("just now");
    expect(seenAgo(185)).toBe("3 min ago");
    expect(healthLine({ cpus: 8, cpuPercent: 12.4, memoryTotal: 16 * 2 ** 30, memoryAvailable: 8 * 2 ** 30, diskTotal: 100 * 2 ** 30, diskFree: 50 * 2 ** 30, gpus: ["amdgpu"] })).toBe(
      "CPU 12 % of 8 · memory 8.0 GB free of 16.0 GB · disk 50.0 GB free of 100 GB · GPU amdgpu",
    );
    expect(healthLine({ cpus: 4, gpus: [] })).toBe("4 CPUs");
  });

  it("groups packages by product", () => {
    const p = (product: string, version: string) => ({ id: version, product, version, platform: "linux-x86_64", fileName: "f", size: 1, sha256: "", keyId: "K", trustedComment: "", uploadedAt: "" });
    expect(byProduct([p("vesper", "0.2.0"), p("vesper", "0.1.0"), p("oriel", "0.1.0")]).map(([k, v]) => [k, v.length])).toEqual([
      ["vesper", 2],
      ["oriel", 1],
    ]);
  });
});
