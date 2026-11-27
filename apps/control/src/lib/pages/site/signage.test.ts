import { describe, expect, it } from "vitest";
import type { SignageChannel, SignageFeed, SignagePlaylist, SignageSlot } from "$protocol";
import { channelStatus, contentLabel, daysLabel, feedStatus, hhmm, layoutZones, minuteAt, nowLabel, nowMarker, parseHhmm, playlistLength, setSignageSources, signageLabel, signageSourceNames, slotBlocks, slotFromDrag } from "./signage";
import { feedChoices } from "./mosaic";

const slot = (p: Partial<SignageSlot>): SignageSlot => ({ id: "s", playlist: "p1", days: [], start: 0, end: 1440, priority: 0, ...p });
const playlists: SignagePlaylist[] = [
  { id: "p1", name: "Welcome", items: [] },
  { id: "p2", name: "Evening", items: [] },
];

describe("signage times", () => {
  it("formats and parses times", () => {
    expect(hhmm(0)).toBe("00:00");
    expect(hhmm(570)).toBe("09:30");
    expect(hhmm(1440)).toBe("24:00");
    expect(parseHhmm("9:30")).toBe(570);
    expect(parseHhmm("24:00")).toBe(1440);
    expect(parseHhmm("9:75")).toBeNull();
    expect(parseHhmm("nope")).toBeNull();
  });

  it("names a slot's days", () => {
    expect(daysLabel([])).toBe("Every day");
    expect(daysLabel([4, 0, 1, 2, 3])).toBe("Weekdays");
    expect(daysLabel([5, 6])).toBe("Weekends");
    expect(daysLabel([6])).toBe("Sun");
    expect(daysLabel([0, 2])).toBe("Mon, Wed");
  });
});

describe("the week calendar", () => {
  it("puts a slot on each of its days", () => {
    const b = slotBlocks([slot({ days: [6], start: 540, end: 720, priority: 2 })], playlists);
    expect(b).toEqual([{ slot: 0, day: 6, from: 540, to: 720, label: "Welcome", priority: 2 }]);
    expect(slotBlocks([slot({ playlist: "gone" })], playlists)).toHaveLength(7);
    expect(slotBlocks([slot({ playlist: "gone", days: [0] })], playlists)[0]?.label).toBe("A deleted playlist");
  });

  it("continues a slot past midnight on the next day (Sunday into Monday)", () => {
    const b = slotBlocks([slot({ days: [6], start: 1320, end: 120 })], playlists);
    expect(b.map((x) => [x.day, x.from, x.to])).toEqual([
      [6, 1320, 1440],
      [0, 0, 120],
    ]);
  });

  it("makes a slot from a drag, snapped", () => {
    expect(minuteAt(100, 480)).toBe(300);
    expect(minuteAt(-5, 480)).toBe(0);
    expect(minuteAt(999, 480)).toBe(1440);
    const s = slotFromDrag(2, 200, 180, 480, "p2");
    expect(s).toMatchObject({ playlist: "p2", days: [2], start: 540, end: 600, priority: 0 });
    // A click (no drag) makes an hour.
    const c = slotFromDrag(0, 240, 240, 480, "p1");
    expect([c.start, c.end]).toEqual([720, 780]);
    const late = slotFromDrag(0, 480, 480, 480, "p1");
    expect([late.start, late.end]).toEqual([1380, 1440]);
  });

  it("places now on the week, Monday first", () => {
    expect(nowMarker(new Date(2026, 10, 29, 10, 30))).toEqual({ day: 6, minute: 630 });
    expect(nowMarker(new Date(2026, 10, 23, 0, 5))).toEqual({ day: 0, minute: 5 });
  });
});

describe("signage labels", () => {
  it("lays out the zones as the server does", () => {
    expect(layoutZones("full")).toEqual({ main: [0, 0, 100, 100] });
    const z = layoutZones("mainSidebarTicker");
    expect(z.main).toEqual([0, 0, 74, 91]);
    expect(z.sidebar).toEqual([74, 0, 26, 91]);
    expect(z.ticker).toEqual([0, 91, 100, 9]);
  });

  it("names items, lengths, health and what plays", () => {
    const feeds = [{ id: "f1", name: "Events" }] as SignageFeed[];
    const names = { presentation: (id: string) => (id === "pr" ? "Welcome slides" : undefined), media: () => "Lobby loop", feeds };
    expect(contentLabel({ kind: "slide", presentation: "pr", slide: 2 }, names)).toBe("Welcome slides · slide 3");
    expect(contentLabel({ kind: "slide", presentation: "x", slide: 0 }, names)).toBe("A missing presentation · slide 1");
    expect(contentLabel({ kind: "video", asset: "a" }, names)).toBe("Lobby loop (loop)");
    expect(contentLabel({ kind: "feed", feed: "f1", rows: 4 }, names)).toBe("Feed: Events");
    expect(contentLabel({ kind: "text", title: "Hi", body: "" }, names)).toBe("Hi");
    expect(playlistLength([{ id: "a", content: { kind: "text", title: "x", body: "" }, duration: 90 }])).toBe("1 min 30 s");
    expect(playlistLength([])).toBe("0 s");
    expect(channelStatus(undefined).status).toBe("off");
    expect(channelStatus({ health: "ok", overridden: false }).label).toBe("Playing");
    expect(channelStatus({ health: "problem", problem: "gone.png is missing", overridden: false })).toEqual({ status: "warn", label: "Skipped: gone.png is missing" });
    expect(channelStatus({ health: "ok", overridden: true }).status).toBe("live");
    const ch = { id: "c", name: "Lobby", layout: "full", slots: [slot({ id: "s1", days: [6], start: 540, end: 720 })], ticker: "", screen: "", source: "", now: { playlist: "p2", slot: "s1", health: "ok", overridden: false } } as SignageChannel;
    expect(nowLabel(ch, playlists)).toBe("Evening (Sun 09:00–12:00)");
    expect(nowLabel({ ...ch, now: { playlist: "p1", health: "ok", overridden: false } }, playlists)).toBe("Welcome (default)");
    expect(nowLabel({ ...ch, now: { health: "idle", overridden: false } }, playlists)).toBe("Nothing scheduled: zones only");
  });

  it("says how a feed's last read went", () => {
    const f = { id: "f", name: "E", url: "", format: "auto", refresh: 900, mapping: { title: "", body: "", when: "" }, columns: [], rows: [] } as SignageFeed;
    expect(feedStatus(f, 0).label).toBe("Not read yet");
    expect(feedStatus({ ...f, rows: [{ a: "1" }], readAt: 0 }, 180_000).label).toBe("1 row · read 3 min ago");
    expect(feedStatus({ ...f, rows: [{ a: "1" }], readAt: 0, problem: "timed out" }, 0)).toEqual({ status: "warn", label: "Kept the last rows: timed out" });
    expect(feedStatus({ ...f, problem: "not found" }, 0).status).toBe("error");
  });
});

describe("Mosaic's source picker", () => {
  it("labels signage channels' sources", () => {
    setSignageSources([{ id: "c", name: "Lobby", source: "SITE-SERVER (Lobby)" } as SignageChannel]);
    expect(signageLabel("SITE-SERVER (Lobby)")).toBe("Signage: Lobby");
    expect(signageLabel("BOOTH (Cam 1)")).toBeUndefined();
    expect(signageSourceNames()).toEqual(["SITE-SERVER (Lobby)"]);
    const choices = feedChoices(["BOOTH (Cam 1)", "SITE-SERVER (Lobby)"], []);
    expect(choices.map((c) => c.label)).toEqual(["Nothing (desktop)", "BOOTH (Cam 1)", "Signage: Lobby"]);
    setSignageSources([]);
  });
});
