import { describe, expect, it, vi } from "vitest";
import { Debounce, PrefsWatcher, collectPrefs, mergePrefs, prefsFromBackup } from "./prefsBackup";

describe("preferences", () => {
  it("collects only the backed-up keys that are set", () => {
    const store: Record<string, string> = { thumbPx: "180", slideView: "grid", token: "secret" };
    expect(collectPrefs((k) => store[k] ?? null)).toEqual({ thumbPx: "180", slideView: "grid" });
  });
  it("reads a backup in either shape, keeping known string keys", () => {
    expect(prefsFromBackup({ schemaVersion: 1, preferences: { thumbPx: "200", other: "x", binTab: 3 } })).toEqual({ thumbPx: "200" });
    expect(prefsFromBackup({ slideView: "list" })).toEqual({ slideView: "list" });
    expect(prefsFromBackup({ schemaVersion: 99, preferences: { thumbPx: "200" } })).toBeNull();
    expect(prefsFromBackup(null)).toBeNull();
  });
  it("merges a backup over this computer's preferences", () => {
    expect(mergePrefs({ thumbPx: "120", rightTab: "timers" }, { thumbPx: "200", slideView: "list" })).toEqual({
      thumbPx: "200",
      rightTab: "timers",
      slideView: "list",
    });
  });
});

describe("debounce", () => {
  it("runs the last function once, after the quiet time", () => {
    vi.useFakeTimers();
    const d = new Debounce(1000);
    const calls: number[] = [];
    d.trigger(() => calls.push(1));
    vi.advanceTimersByTime(600);
    d.trigger(() => calls.push(2));
    vi.advanceTimersByTime(600);
    expect(calls).toEqual([]);
    expect(d.pending).toBe(true);
    vi.advanceTimersByTime(400);
    expect(calls).toEqual([2]);
    expect(d.pending).toBe(false);
    vi.useRealTimers();
  });
});

describe("watcher", () => {
  it("saves once at start, then a few seconds after the last change", async () => {
    vi.useFakeTimers();
    let prefs: Record<string, string> = { thumbPx: "120" };
    const saves: Record<string, string>[] = [];
    const w = new PrefsWatcher({ read: () => prefs, save: async (p) => (saves.push(p), true), delayMs: 3000 });
    w.check();
    await vi.advanceTimersByTimeAsync(3000);
    expect(saves).toEqual([{ thumbPx: "120" }]);
    w.check();
    await vi.advanceTimersByTimeAsync(5000);
    expect(saves.length).toBe(1);
    prefs = { thumbPx: "140" };
    w.check();
    await vi.advanceTimersByTimeAsync(2000);
    prefs = { thumbPx: "160" };
    w.check();
    await vi.advanceTimersByTimeAsync(2000);
    expect(saves.length).toBe(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(saves).toEqual([{ thumbPx: "120" }, { thumbPx: "160" }]);
    vi.useRealTimers();
  });
  it("tries a failed save again at the next check, and skips restored preferences", async () => {
    vi.useFakeTimers();
    const prefs = { slideView: "list" };
    let ok = false;
    let tries = 0;
    const w = new PrefsWatcher({ read: () => prefs, save: async () => (tries++, ok), delayMs: 1000 });
    w.check();
    await vi.advanceTimersByTimeAsync(1000);
    expect(tries).toBe(1);
    ok = true;
    w.check();
    await vi.advanceTimersByTimeAsync(1000);
    expect(tries).toBe(2);
    const w2 = new PrefsWatcher({ read: () => prefs, save: async () => (tries++, true), delayMs: 1000 });
    w2.markSaved(prefs);
    w2.check();
    await vi.advanceTimersByTimeAsync(2000);
    expect(tries).toBe(2);
    vi.useRealTimers();
  });
});
