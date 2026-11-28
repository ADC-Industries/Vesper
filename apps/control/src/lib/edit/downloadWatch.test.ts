import { describe, expect, it } from "vitest";
import { DownloadWatch, isPartial, type DownloadEntry } from "./downloadWatch";

const f = (name: string, size: number, modifiedMs: number): DownloadEntry => ({ name, size, modifiedMs });

describe("the SongSelect Downloads watcher's choice of files", () => {
  it("leaves the files already there alone and reads a new one once it has settled", () => {
    const w = new DownloadWatch(2000);
    expect(w.step([f("old-song.txt", 900, 1_000)], 10_000)).toEqual([]);
    // New: seen once, not read yet (it may still be growing).
    expect(w.step([f("old-song.txt", 900, 1_000), f("new-song.txt", 1200, 10_500)], 13_000)).toEqual([]);
    // The same in the next listing and quiet for 2 s: read, once.
    const got = w.step([f("old-song.txt", 900, 1_000), f("new-song.txt", 1200, 10_500)], 16_000);
    expect(got.map((e) => e.name)).toEqual(["new-song.txt"]);
    expect(w.step([f("old-song.txt", 900, 1_000), f("new-song.txt", 1200, 10_500)], 19_000)).toEqual([]);
  });

  it("waits while a file grows and until it is quiet", () => {
    const w = new DownloadWatch(2000);
    w.step([], 0);
    expect(w.step([f("song.usr", 100, 1_000)], 1_000)).toEqual([]);
    expect(w.step([f("song.usr", 400, 2_000)], 2_000)).toEqual([]);
    // Unchanged, but written 500 ms ago: not yet.
    expect(w.step([f("song.usr", 400, 2_000)], 2_500)).toEqual([]);
    expect(w.step([f("song.usr", 400, 2_000)], 4_500).map((e) => e.name)).toEqual(["song.usr"]);
  });

  it("never reads a partial download, a hidden or temporary file or an empty one", () => {
    const w = new DownloadWatch(0);
    w.step([], 0);
    const list = [f("song.txt.crdownload", 900, 1), f("song.txt.part", 900, 1), f("Unconfirmed 123.crdownload", 9, 1), f(".song.txt", 900, 1), f("~$song.txt", 9, 1), f("empty.txt", 0, 1)];
    w.step(list, 10);
    expect(w.step(list, 20)).toEqual([]);
    expect(isPartial("song.txt.opdownload")).toBe(true);
    expect(isPartial("song.txt")).toBe(false);
  });

  it("reads a finished download that took over a partial one's name, and a file changed again", () => {
    const w = new DownloadWatch(0);
    w.step([f("song.txt", 500, 1)], 0);
    w.step([f("song.txt", 500, 1), f("next.txt.crdownload", 300, 5)], 10);
    // The browser renamed it when done.
    w.step([f("song.txt", 500, 1), f("next.txt", 800, 20)], 30);
    expect(w.step([f("song.txt", 500, 1), f("next.txt", 800, 20)], 40).map((e) => e.name)).toEqual(["next.txt"]);
    // The file that was there before, downloaded again over itself: a new version.
    w.step([f("song.txt", 640, 50), f("next.txt", 800, 20)], 60);
    expect(w.step([f("song.txt", 640, 50), f("next.txt", 800, 20)], 70).map((e) => e.name)).toEqual(["song.txt"]);
  });
});
