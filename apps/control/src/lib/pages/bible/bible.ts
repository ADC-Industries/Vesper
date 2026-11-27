// The Bible page's logic (plan 4.5, docs/SCRIPTURE-PLAN.md), plain TypeScript, unit tested:
// verse keys and selection (click toggles, shift-click selects a range), the reference string a
// selection makes ("Psalm 23:1–4, 6", the server's label format), the slide-split choice, the
// recent references list and the preview URL's parameters.

import type { ReferencePlacement, ScriptureBook, ScriptureSlideRequest, ScriptureVerse } from "$protocol";

/** A verse's key in a selection: `"<chapter>:<verse>"`. */
export type VerseKey = string;

export function verseKey(v: Pick<ScriptureVerse, "chapter" | "verse">): VerseKey {
  return `${v.chapter}:${v.verse}`;
}

/** The verse a key names. */
export function parseKey(k: VerseKey): { chapter: number; verse: number } {
  const [c, v] = k.split(":").map(Number);
  return { chapter: c ?? 0, verse: v ?? 0 };
}

/**
 * A click on verse `index` of `verses`: toggles it, or with `shift` (and an anchor) selects the
 * range from the anchor to it (adding to the selection). Returns the new selection and anchor.
 */
export function clickVerse(
  verses: readonly Pick<ScriptureVerse, "chapter" | "verse">[],
  selected: ReadonlySet<VerseKey>,
  anchor: number | null,
  index: number,
  shift: boolean,
): { selected: Set<VerseKey>; anchor: number } {
  const out = new Set(selected);
  const v = verses[index];
  if (!v) return { selected: out, anchor: anchor ?? index };
  if (shift && anchor !== null && verses[anchor]) {
    const [a, b] = anchor < index ? [anchor, index] : [index, anchor];
    for (let i = a; i <= b; i++) out.add(verseKey(verses[i]!));
    return { selected: out, anchor };
  }
  const k = verseKey(v);
  if (out.has(k)) out.delete(k);
  else out.add(k);
  return { selected: out, anchor: index };
}

/** Every verse selected (Select all). */
export function selectAll(verses: readonly Pick<ScriptureVerse, "chapter" | "verse">[]): Set<VerseKey> {
  return new Set(verses.map(verseKey));
}

/** The name a reference uses for a book: "Psalm 23", not "Psalms 23". */
export function referenceBookName(name: string): string {
  return name === "Psalms" ? "Psalm" : name;
}

/**
 * The reference a selection makes, in the server's label format: runs of consecutive verses
 * joined with an en dash, runs separated by ", ", a new chapter written as `chapter:verse`
 * ("Psalm 23:1–4, 6", "John 3:16–18, 4:1"). Empty selection: "".
 */
export function referenceFromSelection(bookName: string, selected: Iterable<VerseKey>): string {
  const keys = [...selected].map(parseKey).filter((k) => k.chapter > 0 && k.verse > 0);
  if (!keys.length) return "";
  keys.sort((a, b) => a.chapter - b.chapter || a.verse - b.verse);
  const runs: { chapter: number; from: number; to: number }[] = [];
  for (const k of keys) {
    const last = runs[runs.length - 1];
    if (last && last.chapter === k.chapter && k.verse === last.to + 1) last.to = k.verse;
    else if (!(last && last.chapter === k.chapter && k.verse <= last.to)) runs.push({ chapter: k.chapter, from: k.verse, to: k.verse });
  }
  let chapter = 0;
  const parts = runs.map((r) => {
    const span = r.from === r.to ? `${r.from}` : `${r.from}–${r.to}`;
    const text = r.chapter === chapter ? span : `${r.chapter}:${span}`;
    chapter = r.chapter;
    return text;
  });
  return `${referenceBookName(bookName)} ${parts.join(", ")}`;
}

/** The books of a testament, in order. */
export function booksOf(books: readonly ScriptureBook[], testament: ScriptureBook["testament"]): ScriptureBook[] {
  return books.filter((b) => b.testament === testament);
}

/** The slide split choice: a verse count (1–3) or "auto" (as many as fit). */
export type Split = "auto" | "1" | "2" | "3";
export const SPLITS: { value: Split; label: string }[] = [
  { value: "1", label: "1 verse" },
  { value: "2", label: "2 verses" },
  { value: "3", label: "3 verses" },
  { value: "auto", label: "Auto" },
];

export function parseSplit(s: string | null | undefined): Split {
  return s === "1" || s === "2" || s === "3" ? s : "auto";
}

/** The slide request for a reference and the page's options. */
export function slideRequest(
  translation: string,
  reference: string,
  opts: {
    split: Split;
    showReference: boolean;
    verseNumbers: boolean;
    theme: string | null;
    template?: string | null;
    /** A second translation (parallel Bible) and how its slides prefer to show it. */
    second?: string | null;
    secondLayout?: SecondLayout;
  },
): ScriptureSlideRequest {
  const referenceOn: ReferencePlacement = opts.showReference ? "each" : "none";
  const req: ScriptureSlideRequest = { translation, reference, verseNumbers: opts.verseNumbers, referenceOn };
  if (opts.split !== "auto") req.versesPerSlide = Number(opts.split);
  if (opts.theme) req.theme = opts.theme;
  if (opts.theme && opts.template) req.template = opts.template;
  if (opts.second && opts.second !== translation) {
    req.secondTranslation = opts.second;
    if (opts.secondLayout && opts.secondLayout !== "sideBySide") req.secondLayout = opts.secondLayout;
  }
  return req;
}

/** How parallel Bible slides show the second translation by default (a screen's look can
 * still choose otherwise). */
export type SecondLayout = "sideBySide" | "stacked";

export const SECOND_LAYOUTS: { value: SecondLayout; label: string }[] = [
  { value: "sideBySide", label: "Side by side" },
  { value: "stacked", label: "Stacked" },
];

/** Reads the second-layout preference (side by side unless it says stacked). */
export function parseSecondLayout(v: string | null): SecondLayout {
  return v === "stacked" ? "stacked" : "sideBySide";
}

/** The query parameters of `GET /scripture/preview` for slide `slide` of a request. */
export function previewParams(req: ScriptureSlideRequest, slide: number, w: number): Record<string, string | number> {
  const p: Record<string, string | number> = {
    translation: req.translation,
    reference: req.reference,
    verseNumbers: req.verseNumbers ? "true" : "false",
    referenceOn: req.referenceOn,
    slide,
    w,
  };
  if (req.versesPerSlide) p.versesPerSlide = req.versesPerSlide;
  if (req.maxChars) p.maxChars = req.maxChars;
  if (req.theme) p.theme = req.theme;
  if (req.template) p.template = req.template;
  if (req.secondTranslation) p.secondTranslation = req.secondTranslation;
  if (req.secondLayout) p.secondLayout = req.secondLayout;
  return p;
}

/** Recent references: newest first, no duplicates (case-insensitive), at most `max`. */
export const RECENT_MAX = 8;

export interface Recent {
  translation: string;
  reference: string;
}

export function pushRecent(list: readonly Recent[], item: Recent, max = RECENT_MAX): Recent[] {
  const same = (r: Recent) => r.reference.toLowerCase() === item.reference.toLowerCase() && r.translation === item.translation;
  return [item, ...list.filter((r) => !same(r))].slice(0, max);
}

/** The recent list from its saved text (anything unreadable: empty). */
export function parseRecent(text: string | null): Recent[] {
  if (!text) return [];
  try {
    const v = JSON.parse(text) as unknown;
    if (!Array.isArray(v)) return [];
    return v
      .filter((r): r is Recent => !!r && typeof r === "object" && typeof r.translation === "string" && typeof r.reference === "string")
      .slice(0, RECENT_MAX);
  } catch {
    return [];
  }
}
