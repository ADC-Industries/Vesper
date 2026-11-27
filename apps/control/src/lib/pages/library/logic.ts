// The Library page's logic (plain TypeScript, unit tested): title sorting and the A–Z rail,
// the name filter, what is live (for the switch confirmation), the connection card's words and
// small formatters.

import type { CodexStatus, MediaAsset, MediaKind, MediaMatchReport, Presentation, RelinkReport, ShowState, Slide } from "$protocol";
import type { Status } from "../../ui";

/** A media asset's kind: the server leaves `kind` out for images (the default). */
export function mediaKind(a: Pick<MediaAsset, "kind">): MediaKind {
  return a.kind ?? "image";
}

/** Lower case without accents, for matching and sorting. */
export function fold(s: string): string {
  return s.normalize("NFD").replace(/[̀-ͯ]/g, "").toLowerCase();
}

/** A title's sort key: folded, leading quotes and punctuation left out. */
export function titleKey(name: string): string {
  return fold(name.trim()).replace(/^[^\p{L}\p{N}]+/u, "");
}

/** Compares two titles as the table sorts them (numbers in order: "Psalm 9" before "Psalm 10"). */
export function compareTitles(a: string, b: string): number {
  return titleKey(a).localeCompare(titleKey(b), undefined, { numeric: true, sensitivity: "base" });
}

/** The A–Z rail: "#" (digits and other letters) and A to Z. */
export const RAIL: readonly string[] = ["#", ..."ABCDEFGHIJKLMNOPQRSTUVWXYZ"];

/** The rail letter a title files under. */
export function railLetter(name: string): string {
  const c = titleKey(name).charAt(0).toUpperCase();
  return c >= "A" && c <= "Z" ? c : "#";
}

/** The first row of each rail letter in rows already sorted by title (either direction). */
export function railIndex<T>(rows: readonly T[], nameOf: (row: T) => string): Map<string, number> {
  const out = new Map<string, number>();
  rows.forEach((r, i) => {
    const l = railLetter(nameOf(r));
    if (!out.has(l)) out.set(l, i);
  });
  return out;
}

/** The row a rail letter jumps to: its first row, else the next letter's that has rows. */
export function railTarget(index: Map<string, number>, letter: string, descending = false): number | null {
  const order = descending ? [...RAIL].reverse() : [...RAIL];
  for (let i = order.indexOf(letter); i >= 0 && i < order.length; i++) {
    const at = index.get(order[i]!);
    if (at !== undefined) return at;
  }
  return null;
}

/** Every word of the filter starts a word of the name (case and accents ignored). */
export function matchesFilter(name: string, filter: string): boolean {
  const words = fold(filter).split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const hay = fold(name).split(/[^\p{L}\p{N}']+/u).filter(Boolean);
  return words.every((w) => hay.some((h) => h.startsWith(w)));
}

/** What is live on the box, in words ("the slide “Morning Lantern” and a prop"), or null. */
export function liveSummary(s: ShowState | null): string | null {
  const l = s?.layers;
  if (!l) return null;
  const parts: string[] = [];
  if (l.slide) parts.push(l.slide.presentationName ? `a slide of “${l.slide.presentationName}”` : "a slide");
  if (l.announcements) parts.push("an announcement");
  if (l.media) parts.push("background media");
  if (l.foreground) parts.push("foreground media");
  if (l.props.length) parts.push(l.props.length === 1 ? "a prop" : `${l.props.length} props`);
  if (l.messages.length) parts.push(l.messages.length === 1 ? "a message" : `${l.messages.length} messages`);
  if (l.audio) parts.push("audio");
  if (!parts.length) return null;
  return parts.length === 1 ? parts[0]! : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

/** The connection card under the tree (plan 3.4): no sync counts or presence until G8/G14. */
export interface ConnectionCard {
  status: Status;
  title: string;
  detail: string;
  /** Editing is read-only (a show box whose Codex can't be reached). */
  readOnly: boolean;
}

export function connectionCard(codex: CodexStatus | null, online: boolean, serverName: string): ConnectionCard {
  if (!online) {
    return { status: "error", title: "Not connected", detail: `${serverName} can't be reached. Slides and edits wait until the connection is back.`, readOnly: true };
  }
  if (!codex) return { status: "off", title: "Connected", detail: `${serverName} · Codex status unknown`, readOnly: false };
  if (codex.role === "host") {
    return { status: "ok", title: "Connected", detail: `The Codex is on this server (${serverName}).`, readOnly: false };
  }
  const sync = codex.sync;
  if (codex.role === "dormant") {
    return { status: "warn", title: "Joining the site's Codex", detail: "The first copy of the collection is on its way; until then this box shows its own, read only.", readOnly: true };
  }
  if (sync && !sync.reachable) {
    return {
      status: "warn",
      title: "Codex unreachable",
      detail: `Editing needs the site's server. The show keeps playing${sync.lastSync ? `; last heard ${shortTime(sync.lastSync)}` : ""}.`,
      readOnly: true,
    };
  }
  return { status: "ok", title: "Connected", detail: `${serverName} shows a copy of the site's Codex.`, readOnly: false };
}

/** "14:05" today, else "23 Nov". */
export function shortTime(iso: string, now = new Date()): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const same = d.toDateString() === now.toDateString();
  return same
    ? d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

/** Whole days from now until `iso` (at least 0). */
export function daysLeft(iso: string, now = Date.now()): number {
  const t = Date.parse(iso);
  return Number.isNaN(t) ? 0 : Math.max(0, Math.ceil((t - now) / 86_400_000));
}

/** "1.2 GB", "340 MB", "12 KB". */
export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 1000) return `${Math.max(0, Math.round(n || 0))} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1000;
  let u = 0;
  while (v >= 1000 && u < units.length - 1) {
    v /= 1000;
    u++;
  }
  return `${v >= 10 ? Math.round(v) : v.toFixed(1)} ${units[u]}`;
}

/** "0:30", "4:05", "1:02:03". */
export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** "1 song", "3 songs". */
export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

/** "Find missing files" in words. */
export function relinkReportLines(r: RelinkReport): string[] {
  const out: string[] = [];
  const n = (k: number, one: string, many: string) => `${k} ${k === 1 ? one : many}`;
  if (!r.checked) return ["No files are missing."];
  out.push(`${n(r.relinked.length, "file was", "files were")} found and linked again.`);
  if (r.ambiguous.length) out.push(`${n(r.ambiguous.length, "file has", "files have")} more than one candidate: pick ${r.ambiguous.length === 1 ? "it" : "them"} with Relink…`);
  if (r.notFound.length) out.push(`${n(r.notFound.length, "file wasn't", "files weren't")} found. Try “Match a folder on this computer”.`);
  return out;
}

/** A folder match in words. */
export function matchReportLines(r: MediaMatchReport): string[] {
  const out: string[] = [];
  const n = (k: number, one: string, many: string) => `${k} ${k === 1 ? one : many}`;
  out.push(r.matches.length ? `${n(r.matches.length, "missing file was", "missing files were")} found in the folder.` : "None of the missing files are in that folder.");
  if (r.ambiguous.length) out.push(`${n(r.ambiguous.length, "file matches", "files match")} more than one file there.`);
  if (r.unmatched.length) out.push(`${n(r.unmatched.length, "file is", "files are")} still missing.`);
  return out;
}

/** What a song's slides do with themes: the theme they follow (the first one), how many of its
 * slides follow it, the slides that override it, and the theme name it was copied from when no
 * slide follows one. */
export interface SongThemeState {
  theme: string | null;
  linked: number;
  total: number;
  /** Slides that follow a theme and change something it sets ("Reset to theme"). */
  overridden: string[];
  madeFrom: string | null;
}

export function songThemeState(doc: Presentation): SongThemeState {
  let theme: string | null = null;
  let linked = 0;
  let total = 0;
  let madeFrom: string | null = doc.templateOrigin?.theme.name ?? null;
  const overridden: string[] = [];
  for (const sec of doc.sections ?? []) {
    for (const s of sec.slides ?? []) {
      total++;
      if (!madeFrom && s.templateOrigin?.theme.name) madeFrom = s.templateOrigin.theme.name;
      if (!s.themeLink || !s.templateOrigin) continue;
      linked++;
      theme ??= s.templateOrigin.theme.id;
      if (slideOverrides(s).length) overridden.push(s.id);
    }
  }
  return { theme, linked, total, overridden, madeFrom };
}

/** Plain names of what a slide (or one element of it) sets itself instead of its theme. */
export function slideOverrides(s: Slide, element?: string): string[] {
  if (!s.themeLink) return [];
  if (element) return (s.elements ?? []).find((e) => e.id === element)?.fromTheme?.overrides ?? [];
  const own = (s.themeLink.overrides ?? []).filter((o) => !o.startsWith("removed:"));
  const els = (s.elements ?? []).flatMap((e) => e.fromTheme?.overrides ?? []);
  const removed = (s.themeLink.overrides ?? []).some((o) => o.startsWith("removed:"));
  return [...new Set([...own, ...els, ...(removed ? ["removed"] : [])])];
}

const OVERRIDE_LABELS: Record<string, string> = {
  name: "Name",
  bounds: "Position and size",
  rotation: "Rotation",
  opacity: "Opacity",
  flip: "Flip",
  hidden: "Hidden",
  locked: "Locked",
  aspectLocked: "Aspect lock",
  shape: "Shape",
  fill: "Fill",
  stroke: "Outline",
  shadow: "Shadow",
  feather: "Soft edge",
  visibility: "Visibility",
  dataLink: "Live data",
  "text.paragraphs": "Text",
  "text.defaultStyle": "Font and colour",
  "text.defaultParagraph": "Alignment and spacing",
  "text.verticalAlign": "Vertical alignment",
  "text.fit": "Fit",
  "text.fontScaleFactor": "Text scale",
  "text.margins": "Margins",
  "text.textShadow": "Text shadow",
  "text.outline": "Text outline",
  "text.lineMask": "Line boxes",
  "text.fill": "Text fill",
  background: "Background",
  transition: "Transition",
  removed: "Deleted theme objects",
};

/** The label of an override (`bounds` → "Position and size"). */
export function overrideLabel(key: string): string {
  return OVERRIDE_LABELS[key] ?? key;
}
