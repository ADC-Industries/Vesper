// Stage display helpers for the Stage tab, the Screens menu and the Edit view's "Data link"
// insert: stage message presets (stored per browser), the data links an operator can insert,
// and the Screens ▸ Stage Screen submenu.

import type { DataLink, StageLayoutEntry, StageScreenState } from "$protocol";
import type { MenuItem } from "../../shell/menu";

// ---- stage message presets ------------------------------------------------------------------------

/** Presets offered until the operator changes them. */
export const DEFAULT_PRESETS: readonly string[] = ["Wrap up", "5 minutes", "Look at me"];

/** Longest preset / message (the server refuses longer stage messages). */
export const MAX_MESSAGE = 500;

/** Reads stored presets (JSON list of strings); the defaults when missing or broken. */
export function parsePresets(raw: string | null): string[] {
  if (raw === null) return [...DEFAULT_PRESETS];
  try {
    const v: unknown = JSON.parse(raw);
    if (!Array.isArray(v)) return [...DEFAULT_PRESETS];
    return v.filter((x): x is string => typeof x === "string" && x.trim() !== "").map((x) => x.slice(0, MAX_MESSAGE));
  } catch {
    return [...DEFAULT_PRESETS];
  }
}

/** Adds a preset (trimmed; ignored when empty or already there, case-insensitively). */
export function addPreset(list: readonly string[], text: string): string[] {
  const t = text.trim().slice(0, MAX_MESSAGE);
  if (!t || list.some((p) => p.toLowerCase() === t.toLowerCase())) return [...list];
  return [...list, t];
}

export function removePreset(list: readonly string[], index: number): string[] {
  return list.filter((_, i) => i !== index);
}

// ---- data links ---------------------------------------------------------------------------------

export interface DataLinkChoice {
  id: string;
  label: string;
  /** Design-time text of the new element. */
  preview: string;
  link: DataLink;
  /** A picture link: the element is a frame, not text. */
  picture?: boolean;
  /** A picture frame's width / height (16:9 when omitted). */
  aspect?: number;
}

/** The data links the insert menu offers (timers come from the server's timer list). */
export const DATA_LINKS: readonly DataLinkChoice[] = [
  { id: "currentSlide", label: "Current Slide Text", preview: "Current Slide", link: { type: "slideText" } },
  { id: "nextSlide", label: "Next Slide Text", preview: "Next Slide", link: { type: "slideText", which: "next" } },
  { id: "notes", label: "Slide Notes", preview: "Slide Notes", link: { type: "slideNotes" } },
  { id: "clock", label: "Clock", preview: "12:00 PM", link: { type: "clock" } },
  {
    id: "videoCountdown",
    label: "Video Countdown",
    preview: "0:00",
    link: { type: "videoCountdown", format: { hours: "hidden", minutes: "short" } },
  },
  { id: "currentImage", label: "Current Slide Image", preview: "Current Slide Image", link: { type: "slideImage" }, picture: true },
  { id: "nextImage", label: "Next Slide Image", preview: "Next Slide Image", link: { type: "slideImage", which: "next" }, picture: true },
  { id: "playlistItem", label: "Playlist Item", preview: "Playlist Item", link: { type: "playlistItemName" } },
  { id: "nextPlaylistItem", label: "Next Playlist Item", preview: "Next Item", link: { type: "playlistItemName", which: "next" } },
  { id: "stageMessage", label: "Stage Message", preview: "Stage Message", link: { type: "message", flash: true } },
  { id: "slideLabel", label: "Slide Label", preview: "Slide Label", link: { type: "slideLabel" } },
  { id: "groupName", label: "Group Name", preview: "Group Name", link: { type: "sectionName" } },
  { id: "slideCount", label: "Slide Number", preview: "1", link: { type: "slideCount" } },
  { id: "presentationName", label: "Presentation Name", preview: "Presentation Name", link: { type: "presentationName" } },
  { id: "presentationNotes", label: "Presentation Notes", preview: "Presentation Notes", link: { type: "presentationNotes" } },
  { id: "ccli", label: "CCLI Copyright", preview: "Song Title · © Year Publisher", link: { type: "ccliText" } },
  { id: "songArtwork", label: "Song Artwork", preview: "Song Artwork", link: { type: "songArtwork" }, picture: true, aspect: 1 },
  { id: "chordChart", label: "Chord Chart", preview: "Current Slide", link: { type: "chordChart" } },
  { id: "nextChordChart", label: "Next Chord Chart", preview: "Next Slide", link: { type: "chordChart", which: "next" } },
  { id: "songKey", label: "Song Key", preview: "Key G", link: { type: "songKey" } },
];

/** A timer link (`timer` null = the most recently started timer). */
export function timerLink(timer: { id: string; name?: string } | null): DataLinkChoice {
  return {
    id: timer ? `timer-${timer.id}` : "timer-last",
    label: timer ? (timer.name ?? "Timer") : "Most Recently Started",
    preview: "00:00",
    link: timer ? { type: "timer", timer: { id: timer.id, ...(timer.name ? { name: timer.name } : {}) } } : { type: "timer" },
  };
}

/** What an element's data link shows, for the Objects list ("Clock", "Next Slide Text"). */
export function dataLinkLabel(link: DataLink): string {
  switch (link.type) {
    case "slideText":
      return link.which === "next" ? "Next Slide Text" : "Current Slide Text";
    case "slideNotes":
      return link.which === "next" ? "Next Slide Notes" : "Slide Notes";
    case "slideImage":
      return link.which === "next" ? "Next Slide Image" : "Current Slide Image";
    case "timer": {
      const base = link.timer?.name ? `Timer: ${link.timer.name}` : "Timer";
      return link.show === "elapsed" ? `${base} (elapsed)` : link.show === "remaining" ? `${base} (remaining)` : base;
    }
    case "clock":
      return "Clock";
    case "videoCountdown":
      return "Video Countdown";
    case "audioCountdown":
      return "Audio Countdown";
    case "message":
      return "Stage Message";
    case "playlistItemName":
      return link.which === "next" ? "Next Playlist Item" : "Playlist Item";
    case "slideLabel":
      return "Slide Label";
    case "sectionName":
      return "Group Name";
    case "slideCount":
      return link.count === "total" ? "Slide Count" : link.count === "remaining" ? "Slides Remaining" : "Slide Number";
    case "presentationName":
      return link.source === "announcements" ? "Announcement Name" : "Presentation Name";
    case "presentationNotes":
      return "Presentation Notes";
    case "ccliText":
      return "CCLI Copyright";
    case "songArtwork":
      return "Song Artwork";
    case "chordChart":
      return link.which === "next" ? "Next Chord Chart" : "Chord Chart";
    case "songKey":
      return "Song Key";
    default:
      return "Data Link";
  }
}

// ---- Screens ▸ Stage Screen ---------------------------------------------------------------------

/** The built-in layout a stage screen shows when there are no layouts (`layout: null`). */
export const BUILT_IN_LAYOUT = "Current / Next";

/**
 * The Stage Screen submenu: the layouts per stage screen (one screen: the layouts directly;
 * several: a submenu per screen). `choose(screen, layout)` sets a screen's layout.
 */
export function stageScreenMenu(
  screens: readonly StageScreenState[],
  layouts: readonly Pick<StageLayoutEntry, "id" | "name">[],
  choose: (screen: string, layout: string | null) => void,
): MenuItem[] {
  const layoutItems = (s: StageScreenState): MenuItem[] =>
    layouts.length
      ? layouts.map((l) => ({
          type: "item" as const,
          id: `stage-${s.screen}-${l.id}`,
          label: l.name || "Layout",
          checked: s.layout === l.id,
          run: () => choose(s.screen, l.id),
        }))
      : [{ type: "item" as const, id: `stage-${s.screen}-builtin`, label: BUILT_IN_LAYOUT, checked: true, run: () => choose(s.screen, null) }];
  if (screens.length === 1) return layoutItems(screens[0]!);
  return screens.map((s) => ({ type: "item" as const, id: `stage-screen-${s.screen}`, label: s.name || "Stage", detail: s.layoutName, submenu: layoutItems(s) }));
}
