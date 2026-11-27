// Site ▸ Signage's pure logic (docs/SIGNAGE-PLAN.md): the week calendar's blocks and drags,
// the "now" marker, the layouts' zones, item and health labels, and the labels Mosaic's source
// picker gives signage channels' sources.

import type { SignageChannel, SignageContent, SignageFeed, SignageItem, SignageLayout, SignageNow, SignagePlaylist, SignageSlot } from "$protocol";
import type { Status } from "../../ui/types";

export const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"] as const;
export const DAY_MINUTES = 1440;
/** Drags snap to this many minutes. */
export const SNAP = 15;

/** "09:30" (1440 is "24:00"). */
export function hhmm(m: number): string {
  const h = Math.floor(m / 60);
  return `${String(h).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
}

/** "09:30" → 570; null when it isn't a time. */
export function parseHhmm(s: string): number | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(s.trim());
  if (!m) return null;
  const v = Number(m[1]) * 60 + Number(m[2]);
  return Number(m[2]) < 60 && v <= DAY_MINUTES ? v : null;
}

/** One block of the week calendar: a slot on one day (a slot past midnight makes two). */
export interface SlotBlock {
  slot: number;
  day: number;
  /** Minutes from midnight. */
  from: number;
  to: number;
  label: string;
  priority: number;
}

/** The days a slot starts on (none listed: every day). */
export function slotDays(s: SignageSlot): number[] {
  return s.days.length ? [...s.days].sort((a, b) => a - b) : [0, 1, 2, 3, 4, 5, 6];
}

/** Every slot's blocks on the week (Monday first). */
export function slotBlocks(slots: readonly SignageSlot[], playlists: readonly SignagePlaylist[]): SlotBlock[] {
  const name = (id: string) => playlists.find((p) => p.id === id)?.name ?? "A deleted playlist";
  const out: SlotBlock[] = [];
  slots.forEach((s, i) => {
    for (const d of slotDays(s)) {
      const base = { slot: i, label: name(s.playlist), priority: s.priority };
      if (s.end > s.start) out.push({ ...base, day: d, from: s.start, to: s.end });
      else {
        out.push({ ...base, day: d, from: s.start, to: DAY_MINUTES });
        if (s.end > 0) out.push({ ...base, day: (d + 1) % 7, from: 0, to: s.end });
      }
    }
  });
  return out;
}

/** The minute a point `y` px down a day column `height` px tall is at, snapped. */
export function minuteAt(y: number, height: number): number {
  const m = (Math.max(0, Math.min(y, height)) / Math.max(1, height)) * DAY_MINUTES;
  return Math.min(DAY_MINUTES, Math.round(m / SNAP) * SNAP);
}

/** A new slot from a drag on a day column (at least one snap long). */
export function slotFromDrag(day: number, y0: number, y1: number, height: number, playlist: string): SignageSlot {
  let a = minuteAt(Math.min(y0, y1), height);
  let b = minuteAt(Math.max(y0, y1), height);
  if (b - a < SNAP) b = Math.min(DAY_MINUTES, a + 60);
  if (b - a < SNAP) a = b - 60;
  return { id: "", playlist, days: [day], start: a, end: b, priority: 0 };
}

/** Where "now" is on the week: the day (Monday 0) and the minute. */
export function nowMarker(d: Date): { day: number; minute: number } {
  return { day: (d.getDay() + 6) % 7, minute: d.getHours() * 60 + d.getMinutes() };
}

/** A slot's repeat in words: "Every day", "Weekdays", "Sun", "Mon, Wed". */
export function daysLabel(days: readonly number[]): string {
  const d = [...new Set(days)].sort((a, b) => a - b);
  if (d.length === 0 || d.length === 7) return "Every day";
  if (d.join() === "0,1,2,3,4") return "Weekdays";
  if (d.join() === "5,6") return "Weekends";
  return d.map((x) => DAYS[x]).join(", ");
}

/** A layout's zones in % of the screen (the picker's little pictures), as the server lays them. */
export function layoutZones(l: SignageLayout): { main: [number, number, number, number]; sidebar?: [number, number, number, number]; ticker?: [number, number, number, number] } {
  const side = l === "mainSidebar" || l === "mainSidebarTicker";
  const tick = l === "mainTicker" || l === "mainSidebarTicker";
  const sw = side ? 26 : 0;
  const th = tick ? 9 : 0;
  return {
    main: [0, 0, 100 - sw, 100 - th],
    ...(side ? { sidebar: [100 - sw, 0, sw, 100 - th] as [number, number, number, number] } : {}),
    ...(tick ? { ticker: [0, 100 - th, 100, th] as [number, number, number, number] } : {}),
  };
}

export const LAYOUTS: { value: SignageLayout; label: string }[] = [
  { value: "full", label: "Full screen" },
  { value: "mainTicker", label: "Main + ticker" },
  { value: "mainSidebar", label: "Main + sidebar" },
  { value: "mainSidebarTicker", label: "Main + sidebar + ticker" },
];

/** Names of what items show (presentations, media, feeds), by ID. */
export interface Names {
  presentation: (id: string) => string | undefined;
  media: (id: string) => string | undefined;
  feeds: readonly SignageFeed[];
}

/** What an item shows, in words. */
export function contentLabel(c: SignageContent, n: Names): string {
  switch (c.kind) {
    case "slide":
      return `${n.presentation(c.presentation) ?? "A missing presentation"} · slide ${c.slide + 1}`;
    case "image":
      return n.media(c.asset) ?? "A missing image";
    case "video":
      return `${n.media(c.asset) ?? "A missing video"} (loop)`;
    case "text":
      return c.title;
    case "feed":
      return `Feed: ${n.feeds.find((f) => f.id === c.feed)?.name ?? "a deleted feed"}`;
  }
}

/** The kind of an item, for its badge. */
export function contentKind(c: SignageContent): string {
  return { slide: "Slide", image: "Image", video: "Video", text: "Announcement", feed: "Feed" }[c.kind];
}

/** A playlist's total time in words: "2 min 30 s". */
export function playlistLength(items: readonly SignageItem[]): string {
  const s = Math.round(items.reduce((a, i) => a + (i.duration || 0), 0));
  return s >= 60 ? `${Math.floor(s / 60)} min${s % 60 ? ` ${s % 60} s` : ""}` : `${s} s`;
}

/** A channel's health as a status dot. */
export function channelStatus(now: SignageNow | undefined): { status: Status; label: string } {
  if (now?.overridden) return { status: "live", label: "Override showing" };
  switch (now?.health) {
    case "ok":
      return { status: "ok", label: "Playing" };
    case "problem":
      return { status: "warn", label: now.problem ? `Skipped: ${now.problem}` : "A problem" };
    default:
      return { status: "off", label: "Nothing scheduled" };
  }
}

/** What a channel plays now, in words: "Welcome (Sunday morning)". */
export function nowLabel(c: SignageChannel, playlists: readonly SignagePlaylist[]): string {
  const p = playlists.find((x) => x.id === c.now?.playlist);
  if (!p) return "Nothing scheduled: zones only";
  const slot = c.now?.slot ? c.slots.find((s) => s.id === c.now?.slot) : undefined;
  const why = slot ? `${daysLabel(slot.days)} ${hhmm(slot.start)}–${hhmm(slot.end)}` : "default";
  return `${p.name} (${why})`;
}

/** A feed's last read in words. */
export function feedStatus(f: SignageFeed, now: number): { status: Status; label: string } {
  if (f.problem) return { status: f.rows.length ? "warn" : "error", label: f.rows.length ? `Kept the last rows: ${f.problem}` : f.problem };
  if (f.readAt === undefined || f.readAt === null) return { status: "off", label: "Not read yet" };
  const min = Math.max(0, Math.round((now - f.readAt) / 60000));
  return { status: "ok", label: `${f.rows.length} row${f.rows.length === 1 ? "" : "s"} · read ${min ? `${min} min ago` : "just now"}` };
}

// ---- Mosaic's source picker ----------------------------------------------------------------

let signageSources = new Map<string, string>();

/** The signage channels' sources (`HOST (Lobby)` → "Lobby"), for Mosaic's source picker. */
export function setSignageSources(channels: readonly SignageChannel[]): void {
  signageSources = new Map(channels.filter((c) => c.source).map((c) => [c.source, c.name]));
}

/** A source's label in Mosaic's picker: "Signage: Lobby" for a signage channel's. */
export function signageLabel(source: string): string | undefined {
  const n = signageSources.get(source);
  return n === undefined ? undefined : `Signage: ${n}`;
}

/** The signage channels' sources (listed even before the network lists them). */
export function signageSourceNames(): string[] {
  return [...signageSources.keys()];
}
