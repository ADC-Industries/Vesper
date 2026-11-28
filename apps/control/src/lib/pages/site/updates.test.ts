import { describe, expect, it } from "vitest";
import type { LauncherEntry, LauncherProduct } from "$protocol";
import { addVersion, attentionCount, broughtIn, byProduct, failedAdds, bytes, githubLine, packageSource, productName, queuedAdds, roleLabel, canAct, healthLine, outcomeText, pendingFor, platformLabel, seenAgo, sourcesText, stateLabel, stateTone, updateFor } from "./updates";

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

  it("says why an Add app failed, until it is queued again or installed", () => {
    const failed = { product: "vesper-oriel", name: "Vesper Oriel", outcome: { to: "0.2.0", result: "failed" as const, message: "apt couldn't install it (exit status: 100): E: Unable to correct problems", at: "2026-11-27T10:00:00Z" } };
    const e = entry({}, {});
    e.report.failedInstalls = [failed];
    expect(failedAdds(e).map((f) => f.text)).toEqual(["Adding Vesper Oriel 0.2.0 failed: apt couldn't install it (exit status: 100): E: Unable to correct problems"]);
    expect(attentionCount([e])).toBe(1);
    const rolled = entry();
    rolled.report.failedInstalls = [{ ...failed, outcome: { ...failed.outcome, result: "rolledBack", message: "" } }];
    expect(failedAdds(rolled)[0]!.text).toBe("Adding Vesper Oriel 0.2.0 failed: it didn't come up healthy and was taken out again");
    const again = entry({ pending: [{ type: "install", product: "vesper-oriel", version: "0.2.0" }] });
    again.report.failedInstalls = [failed];
    expect(failedAdds(again)).toEqual([]);
    const installed = entry({}, { product: "vesper-oriel", name: "Vesper Oriel", version: "0.2.0" });
    installed.report.failedInstalls = [failed];
    expect(failedAdds(installed)).toEqual([]);
    expect(failedAdds(entry())).toEqual([]);
  });

  it("names what was brought in from GitHub", () => {
    expect(broughtIn("vesper-idn-node 0.3.8 for linux-x86_64")).toBe("IDN Node (bundled) 0.3.8 for Linux (x86_64)");
    expect(broughtIn("vesper-oriel 0.1.0 for windows-x86_64")).toBe("Vesper Oriel 0.1.0 for Windows (x86_64)");
    expect(broughtIn("something else")).toBe("something else");
  });

  it("words Add app and the GitHub check", () => {
    const e = entry({ pending: [{ type: "install", product: "vesper-oriel", version: "0.2.0" }] });
    expect(queuedAdds(e)).toEqual(["Adding Vesper Oriel 0.2.0 (queued)"]);
    expect(pendingFor(e, "vesper-oriel")).toBe("Adding 0.2.0 queued");
    expect(productName("vesper-idn-node")).toBe("IDN Node (bundled)");
    const c = { product: "vesper", name: "Vesper", beta: "0.3.0-beta1", platforms: ["linux-x86_64"], sources: ["channel", "github"] };
    expect(addVersion(c)).toBe("0.3.0-beta1");
    expect(addVersion({ ...c, stable: "0.2.0" })).toBe("0.2.0");
    expect(sourcesText(c)).toBe("the update channel or GitHub");
    const p = { id: "1", product: "vesper", version: "0.2.0", platform: "linux-x86_64", fileName: "f", size: 1, sha256: "", keyId: "K", trustedComment: "", uploadedAt: "" };
    expect(packageSource(p)).toBe("Uploaded");
    expect(packageSource({ ...p, source: "github" })).toBe("GitHub");
    const settings = { enabled: true, repository: "example/vesper", channel: "stable" as const, intervalHours: 6 };
    const when = (iso: string) => iso.slice(11, 16);
    expect(githubLine({ settings, added: [], checking: false }, when)).toBe("Not checked yet");
    expect(githubLine({ settings, added: [], checking: false, lastCheck: "2026-11-27T10:02:00Z", lastSuccess: "2026-11-27T10:02:00Z", nextCheck: "2026-11-27T16:02:00Z" }, when)).toBe("Last checked 10:02 · next 16:02");
    expect(githubLine({ settings, added: [], checking: true }, when)).toBe("Checking GitHub now…");
    expect(githubLine({ settings: { ...settings, enabled: false }, added: [], checking: false }, when)).toBe("Off: new releases aren't looked for.");
  });
});
