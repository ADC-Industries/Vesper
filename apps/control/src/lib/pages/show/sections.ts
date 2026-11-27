// The Show page's centre as one scrolling list of sections: one per playlist item (headers,
// presentations, media), or a single section for a library item picked outside the playlist.
// Plain TypeScript, unit tested: sectioning, which section and slide are live or next, the
// size slider and the placeholder heights that keep a long playlist cheap to scroll.

import type { PlaylistItemDetail, PresentationSummary, Presentation, SlideAction } from "$protocol";

export type ShowSection =
  | { type: "header"; key: string; itemId: string; name: string; color: string | null }
  | {
      type: "presentation";
      key: string;
      /** The playlist item (`null`: a library item shown on its own). */
      itemId: string | null;
      presentationId: string;
      name: string;
      slideCount: number;
      enabled: boolean;
      /** The item's arrangement (`null`: the presentation's selected one). */
      arrangement: string | null;
      /** Song, scripture, text or media (from the summary). */
      kind: PresentationKind;
      /** Authors, year and CCLI number from the summary ("" when unknown). */
      meta: string;
    }
  | { type: "missing"; key: string; itemId: string; name: string }
  | { type: "item"; key: string; itemId: string; name: string; kind: "media" | "videoInput" | "other"; enabled: boolean };

/** A position of the show state (live slide or cursor), as far as the sections care. */
export interface Position {
  presentation: string;
  slide: string;
  playlist: { playlist: string; item: string } | null;
  /** Its place in the play order (which occurrence of a repeated slide; absent: not known). */
  step?: number | null;
}

/** One section per playlist item, in order. */
export function playlistSections(items: readonly PlaylistItemDetail[], headerColor: (it: PlaylistItemDetail) => string | null): ShowSection[] {
  return items.map((it): ShowSection => {
    const key = `item:${it.id}`;
    switch (it.kind.type) {
      case "header":
        return { type: "header", key, itemId: it.id, name: it.displayName, color: headerColor(it) };
      case "presentation":
        if (!it.presentation) return { type: "missing", key, itemId: it.id, name: it.displayName };
        return {
          type: "presentation",
          key,
          itemId: it.id,
          presentationId: it.presentation.id,
          name: it.displayName,
          slideCount: it.presentation.slideCount,
          enabled: it.enabled,
          arrangement: it.kind.arrangement ?? null,
          kind: it.presentation.kind ?? "text",
          meta: summaryMeta(it.presentation),
        };
      case "media":
      case "videoInput":
        return { type: "item", key, itemId: it.id, name: it.displayName, kind: it.kind.type, enabled: it.enabled };
      default:
        return { type: "item", key, itemId: it.id, name: it.displayName, kind: "other", enabled: it.enabled };
    }
  });
}

/** The single section of a library item picked outside the playlist. */
export function librarySection(p: Pick<PresentationSummary, "id" | "name" | "slideCount"> & Partial<PresentationSummary>): ShowSection {
  return {
    type: "presentation",
    key: `lib:${p.id}`,
    itemId: null,
    presentationId: p.id,
    name: p.name,
    slideCount: p.slideCount,
    enabled: true,
    arrangement: null,
    kind: p.kind ?? "text",
    meta: summaryMeta(p),
  };
}

/**
 * Whether a position (the live slide or the cursor) belongs to this section. A playlist
 * section matches the playlist item the slide was triggered from (the same presentation may be
 * in the playlist twice); a slide triggered outside a playlist matches by presentation, and so
 * does a library section.
 */
export function inSection(sec: ShowSection, pos: Position | null): boolean {
  if (!pos || sec.type !== "presentation" || pos.presentation !== sec.presentationId) return false;
  if (sec.itemId === null || !pos.playlist) return true;
  return pos.playlist.item === sec.itemId;
}

export interface SlideMarks {
  live: boolean;
  next: boolean;
}

/** LIVE and NEXT for one slide of a section (NEXT never on the live slide or a disabled one). */
export function slideMarks(sec: ShowSection, slideId: string, enabled: boolean, live: Position | null, cursor: Position | null): SlideMarks {
  const isLive = inSection(sec, live) && live!.slide === slideId;
  const isNext = !isLive && enabled && inSection(sec, cursor) && cursor!.slide === slideId;
  return { live: isLive, next: isNext };
}

// ---------------------------------------------------------------- play order

/**
 * The play order of a presentation as the server plays it (`show::play_order`): the document
 * indices of its slides in the order of the item's arrangement, else the presentation's
 * selected one; repeated groups appear again, groups left out are skipped, unknown group IDs
 * are ignored. `null`: no arrangement applies (or it plays nothing), so document order.
 */
export function playOrder(p: Pick<Presentation, "sections" | "arrangements" | "selectedArrangement">, itemArrangement: string | null): number[] | null {
  const find = (id: string | null | undefined) => (id ? p.arrangements?.find((a) => a.id === id) : undefined);
  const arr = find(itemArrangement) ?? find(p.selectedArrangement);
  if (!arr) return null;
  const starts = new Map<string, [number, number]>();
  let at = 0;
  for (const sec of p.sections ?? []) {
    const n = sec.slides?.length ?? 0;
    starts.set(sec.id, [at, n]);
    at += n;
  }
  const order: number[] = [];
  for (const id of arr.sections ?? []) {
    const r = starts.get(id);
    if (r) for (let i = r[0]; i < r[0] + r[1]; i++) order.push(i);
  }
  return order.length ? order : null;
}

/** One tile of a section: a slide at its place in the play order. */
export interface Occurrence<S> {
  slide: S;
  /** Place in the play order (`triggerSlide.step`); `null` for a slide left out of it. */
  step: number | null;
  /** The number shown on the tile (1-based place in the play order; `null`: left out). */
  n: number | null;
  /** Unique within the section (`{#each}` key, selection). */
  key: string;
}

/** A section's tiles: the play order (`order`, `null` = document order) with repeats, then the
 *  slides it leaves out (`left`, in document order). */
export function arrangeSlides<S extends { id: string; index: number }>(slides: readonly S[], order: readonly number[] | null): { played: Occurrence<S>[]; left: Occurrence<S>[]; ids: string[] } {
  const steps = order ?? slides.map((_, i) => i);
  const played: Occurrence<S>[] = [];
  const used = new Set<number>();
  steps.forEach((d, k) => {
    const s = slides[d];
    if (!s) return;
    used.add(d);
    played.push({ slide: s, step: k, n: k + 1, key: `${s.id}#${k}` });
  });
  const left = slides.filter((_, d) => !used.has(d)).map((s) => ({ slide: s, step: null, n: null, key: `${s.id}#-` }));
  return { played, left, ids: steps.map((d) => slides[d]?.id ?? "") };
}

/** Whether a position is at this occurrence of a slide: its own step when the position's step
 *  holds that slide in `ids` (the section's play order as slide IDs), else the slide's first
 *  occurrence (a position without a step, or from another order). A left-out slide (`step`
 *  `null`) matches by ID. */
export function atOccurrence(pos: Position | null, slideId: string, step: number | null, ids: readonly string[]): boolean {
  if (!pos || pos.slide !== slideId) return false;
  if (step === null) return true;
  const ps = pos.step;
  if (ps != null && ids[ps] === slideId) return ps === step;
  return ids.indexOf(slideId) === step;
}

/** LIVE and NEXT for one occurrence (`slideMarks` for a section in play order). `announcement`
 *  is what the announcements layer shows: LIVE as well. */
export function occurrenceMarks(sec: ShowSection, slideId: string, enabled: boolean, step: number | null, ids: readonly string[], live: Position | null, cursor: Position | null, announcement: Position | null = null): SlideMarks {
  const isLive = (inSection(sec, live) && atOccurrence(live, slideId, step, ids)) || (inSection(sec, announcement) && atOccurrence(announcement, slideId, step, ids));
  const isNext = !isLive && enabled && inSection(sec, cursor) && atOccurrence(cursor, slideId, step, ids);
  return { live: isLive, next: isNext };
}

/** Key of the section that holds the live slide (the first match), or null. */
export function liveSectionKey(sections: readonly ShowSection[], live: Position | null): string | null {
  return sections.find((s) => inSection(s, live))?.key ?? null;
}

/** Presentation items and their slides, for the toolbar ("12 items · 148 slides"). */
export function playlistCounts(sections: readonly ShowSection[]): { items: number; slides: number } {
  let items = 0;
  let slides = 0;
  for (const s of sections) {
    if (s.type === "header") continue;
    items++;
    if (s.type === "presentation") slides += s.slideCount;
  }
  return { items, slides };
}

// ---------------------------------------------------------------- sizes

/** The size slider: tile width in CSS pixels. */
export const THUMB_MIN = 120;
export const THUMB_MAX = 420;
export const THUMB_STEP = 10;
export const THUMB_DEFAULT = 190;
/** One press of Ctrl+= / Ctrl+− or one Ctrl+wheel notch. */
export const THUMB_NUDGE = 30;

export function clampThumb(px: number): number {
  if (!Number.isFinite(px)) return THUMB_DEFAULT;
  return Math.max(THUMB_MIN, Math.min(THUMB_MAX, Math.round(px / THUMB_STEP) * THUMB_STEP));
}

/** Gap between tiles: tighter for small tiles, as drawn. */
export function thumbGap(px: number): number {
  return px < 160 ? 10 : 14;
}

/** Tiles per row in a grid of `width` (auto-fill of `px` columns with `gap`). */
export function perRow(width: number, px: number, gap: number): number {
  if (width <= 0) return 1;
  return Math.max(1, Math.floor((width + gap) / (Math.min(px, width) + gap)));
}

/** Height of a tile: the picture at `aspect` plus the group band. */
export const BAND_H = 22;

/** Estimated height of a section's slides before its document is loaded, so the scroll bar and
 *  "scroll into view" are right while the sections further down are still placeholders. */
export function placeholderHeight(view: "grid" | "text" | "list", slides: number, width: number, px: number, aspect = 16 / 9): number {
  if (slides <= 0) return 0;
  if (view === "list") return slides * 31;
  if (view === "text") return slides * 70 + (slides - 1) * 6;
  const gap = thumbGap(px);
  const n = perRow(width, px, gap);
  const rows = Math.ceil(slides / n);
  const tile = Math.min(px, width) / aspect + BAND_H + 4;
  return rows * tile + (rows - 1) * gap;
}

// ---------------------------------------------------------------- media cues

export interface MediaCue {
  action: string;
  name: string;
  asset: string;
}

/** The slide's media actions (its media cues), in order. */
export function mediaCues(actions: readonly SlideAction[] | undefined): MediaCue[] {
  const out: MediaCue[] = [];
  for (const a of actions ?? []) {
    if (a.kind.type !== "media" || a.enabled === false) continue;
    out.push({ action: a.id, name: a.name || "Media", asset: a.kind.asset });
  }
  return out;
}

/** Media cues of every slide of a presentation, by slide ID. */
export function cuesBySlide(p: Presentation): Map<string, MediaCue[]> {
  const m = new Map<string, MediaCue[]>();
  for (const sec of p.sections ?? []) for (const s of sec.slides ?? []) {
    const c = mediaCues(s.actions);
    if (c.length) m.set(s.id, c);
  }
  return m;
}

/** A version for each slide's thumbnail URL (`&v=`), so the grid loads a new picture after an
 *  edit: the server's thumbnail draws the slide over the most recent background media cue at or
 *  before it, so it changes with the slide itself, with any slide's actions (a cue added or
 *  removed earlier in the presentation) and with the presentation's own settings. */
export function thumbVersions(p: Presentation): Map<string, string> {
  const { sections, modifiedAt: _m, createdAt: _c, ...own } = p;
  const slides = (sections ?? []).flatMap((s) => s.slides ?? []);
  const base = JSON.stringify(own) + JSON.stringify(slides.map((s) => s.actions ?? null));
  return new Map(slides.map((s) => [s.id, fnv(base + JSON.stringify(s))]));
}

function fnv(s: string): string {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return (h >>> 0).toString(36);
}

// ---------------------------------------------------------------- the presentation's kind

export type PresentationKind = "song" | "scripture" | "text" | "media";

/** A summary's second line: authors and year, CCLI number ("" when the server knows none). */
export function summaryMeta(p: Partial<Pick<PresentationSummary, "authors" | "copyrightYear" | "ccliNumber">> | null | undefined): string {
  const parts: string[] = [];
  if (p?.authors) parts.push(p.authors + (p.copyrightYear ? `, ${p.copyrightYear}` : ""));
  else if (p?.copyrightYear) parts.push(String(p.copyrightYear));
  if (p?.ccliNumber) parts.push(`CCLI ${p.ccliNumber}`);
  return parts.join(" · ");
}

/** Song, scripture or plain text, read off a loaded document (the summaries carry the server's
 *  kind, media included: prefer that). */
export function presentationKind(p: Presentation | null | undefined): PresentationKind {
  if (!p) return "text";
  if (p.scripture) return "scripture";
  if (p.songInfo || (p.arrangements?.length ?? 0) > 0) return "song";
  return "text";
}

/** The kind's dot colour (as on the board). */
export const KIND_DOT: Record<PresentationKind, string> = {
  song: "#B48CF0",
  scripture: "#4FB487",
  text: "#5B8DEF",
  media: "#8A8E9A",
};

/** The section header's second line: authors and year for songs, else the slide count. */
export function sectionMeta(p: Presentation | null | undefined): string {
  const si = p?.songInfo;
  const parts: string[] = [];
  if (si?.authors) parts.push(si.authors + (si.copyrightYear ? `, ${si.copyrightYear}` : ""));
  else if (si?.copyrightYear) parts.push(String(si.copyrightYear));
  if (si?.ccliNumber) parts.push(`CCLI ${si.ccliNumber}`);
  if (si?.originalKey) parts.push(`Key of ${si.originalKey}`);
  const w = p?.canvasSize?.width;
  const h = p?.canvasSize?.height;
  if (w && h && Math.abs(w / h - 16 / 9) > 0.01) parts.push(`${w}×${h}`);
  return parts.join(" · ");
}

// ---------------------------------------------------------------- the playlist tree

export interface TreeNodeLike {
  id: string;
  kind: "folder" | "playlist";
  children: TreeNodeLike[];
}

/** Where a node sits: its parent folder (`null` = top level) and the sibling before it. */
export function nodePlace(tree: readonly TreeNodeLike[], id: string, parent: string | null = null): { parent: string | null; after: string | null } | null {
  for (let i = 0; i < tree.length; i++) {
    const n = tree[i]!;
    if (n.id === id) return { parent, after: i > 0 ? tree[i - 1]!.id : null };
    const sub = nodePlace(n.children, id, n.id);
    if (sub) return sub;
  }
  return null;
}

/** True when `id` is `ancestor` or inside it (a folder can't move into itself). */
export function isWithin(tree: readonly TreeNodeLike[], ancestor: string, id: string): boolean {
  const find = (nodes: readonly TreeNodeLike[]): TreeNodeLike | null => {
    for (const n of nodes) {
      if (n.id === ancestor) return n;
      const f = find(n.children);
      if (f) return f;
    }
    return null;
  };
  const a = find(tree);
  if (!a) return false;
  const walk = (n: TreeNodeLike): boolean => n.id === id || n.children.some(walk);
  return walk(a);
}

export type TreeDrop = { into: string } | { before: string } | { root: true };

/**
 * The `PUT /playlists/{id}` move for a drop: into a folder (at its end), before a node (in its
 * folder), or at the end of the top level. `null` when the drop changes nothing or isn't allowed.
 */
export function treeMove(tree: readonly TreeNodeLike[], id: string, drop: TreeDrop): { parent: string | null; after: string | null } | null {
  const from = nodePlace(tree, id);
  if (!from) return null;
  let to: { parent: string | null; after: string | null };
  if ("root" in drop) {
    const last = tree[tree.length - 1];
    to = { parent: null, after: last && last.id !== id ? last.id : (tree[tree.length - 2]?.id ?? null) };
  } else if ("into" in drop) {
    if (isWithin(tree, id, drop.into)) return null;
    const folder = findNode(tree, drop.into);
    if (!folder || folder.kind !== "folder") return null;
    const kids = folder.children.filter((c) => c.id !== id);
    to = { parent: folder.id, after: kids[kids.length - 1]?.id ?? null };
  } else {
    if (drop.before === id) return null;
    const place = nodePlace(tree, drop.before);
    if (!place || (place.parent !== null && isWithin(tree, id, place.parent))) return null;
    // The sibling before the target, skipping the moved node itself.
    const siblings = place.parent === null ? tree : (findNode(tree, place.parent)?.children ?? []);
    const i = siblings.findIndex((s) => s.id === drop.before);
    let after: string | null = null;
    for (let j = i - 1; j >= 0; j--) {
      if (siblings[j]!.id !== id) {
        after = siblings[j]!.id;
        break;
      }
    }
    to = { parent: place.parent, after };
  }
  return to.parent === from.parent && to.after === from.after ? null : to;
}

function findNode(tree: readonly TreeNodeLike[], id: string): TreeNodeLike | null {
  for (const n of tree) {
    if (n.id === id) return n;
    const f = findNode(n.children, id);
    if (f) return f;
  }
  return null;
}

// ---------------------------------------------------------------- keys

/** The menu key or Shift+F10: opens a slide's context menu from the keyboard. */
export function slideContextKey(e: Pick<KeyboardEvent, "key" | "shiftKey" | "ctrlKey" | "altKey" | "metaKey">): boolean {
  if (e.key === "ContextMenu") return true;
  return e.key === "F10" && e.shiftKey && !e.ctrlKey && !e.altKey && !e.metaKey;
}
