// Text of a text box for the in-place editor: plain text, code-point ranges (what
// `setTextStyle.range` counts) versus the UTF-16 offsets the DOM uses, the effective style of
// a run (the cascade box default → paragraph → run) and the text box rebuilt from what the
// operator typed.

import type { CharStyle, ChordMark, Color, Paragraph, ParagraphStyle, TextBox, TextRange, TextRun } from "$protocol";
import { paragraphText, splitChords } from "./chords";

/** Plain text: runs joined, paragraphs separated by `\n` (as the server counts). */
export function plainText(tb: TextBox | undefined | null): string {
  return (tb?.paragraphs ?? []).map((p) => (p.runs ?? []).map((r) => r.text ?? "").join("")).join("\n");
}

/** Length in Unicode code points. */
export function cpLength(s: string): number {
  let n = 0;
  for (const _ of s) n++;
  return n;
}

/** Code points in `s` before UTF-16 offset `i` (an offset inside a surrogate pair rounds up). */
export function utf16ToCp(s: string, i: number): number {
  let cp = 0;
  let at = 0;
  for (const ch of s) {
    if (at >= i) break;
    at += ch.length;
    cp++;
  }
  return cp;
}

/** UTF-16 offset of code point `cp` in `s` (clamped to the end). */
export function cpToUtf16(s: string, cp: number): number {
  let at = 0;
  let n = 0;
  for (const ch of s) {
    if (n >= cp) break;
    at += ch.length;
    n++;
  }
  return at;
}

/** A non-empty code-point range from two UTF-16 offsets into `text` (any order), or null. */
export function rangeFromUtf16(text: string, a: number, b: number): TextRange | null {
  const s = utf16ToCp(text, Math.min(a, b));
  const e = utf16ToCp(text, Math.max(a, b));
  return e > s ? { start: s, end: e } : null;
}

/** The text a code-point range covers. */
export function rangeText(text: string, r: TextRange): string {
  return Array.from(text).slice(r.start, r.end).join("");
}

// ---- styles ---------------------------------------------------------------------------------------

/** What the renderer uses when nothing is set (CharStyle::fallback). */
export const FALLBACK_STYLE: Required<Pick<CharStyle, "fontFamily" | "fontWeight" | "size" | "italic">> & { color: Color } = {
  fontFamily: "DejaVu Sans",
  fontWeight: 400,
  size: 72,
  italic: false,
  color: { r: 1, g: 1, b: 1 },
};

/** Box default, then the paragraph's end style (for empty paragraphs) or the run's overrides. */
export function effectiveStyle(tb: TextBox | undefined, run?: CharStyle | null): CharStyle {
  return { ...FALLBACK_STYLE, ...stripUndef(tb?.defaultStyle ?? {}), ...stripUndef(run ?? {}) };
}

export function effectiveParagraph(tb: TextBox | undefined, p?: Paragraph | null): ParagraphStyle {
  return { ...stripUndef(tb?.defaultParagraph ?? {}), ...stripUndef(p?.style ?? {}) };
}

function stripUndef<T extends object>(o: T): Partial<T> {
  const out: Partial<T> = {};
  for (const [k, v] of Object.entries(o)) if (v !== undefined) (out as Record<string, unknown>)[k] = v;
  return out;
}

/** The style of the first character of a code-point range (what the inspector shows). */
export function styleAt(tb: TextBox | undefined, cp: number): CharStyle {
  let at = 0;
  const paras = tb?.paragraphs ?? [];
  for (const p of paras) {
    for (const r of p.runs ?? []) {
      const n = cpLength(r.text ?? "");
      if (cp < at + n) return effectiveStyle(tb, r.style);
      at += n;
    }
    if (cp === at) return effectiveStyle(tb, p.endStyle ?? p.runs?.[p.runs.length - 1]?.style);
    at += 1; // the paragraph break
  }
  return effectiveStyle(tb, paras[paras.length - 1]?.runs?.[0]?.style);
}

/** The paragraph a code point is in. */
export function paragraphAt(tb: TextBox | undefined, cp: number): Paragraph | null {
  let at = 0;
  for (const p of tb?.paragraphs ?? []) {
    const n = (p.runs ?? []).reduce((s, r) => s + cpLength(r.text ?? ""), 0);
    if (cp <= at + n) return p;
    at += n + 1;
  }
  return null;
}

// ---- rebuilding a text box from the editor ----------------------------------------------------------

/** What the in-place editor holds: paragraphs of text pieces, each tagged with its origin. */
export interface EditedParagraph {
  /** Index of the paragraph it came from (a paragraph made with Enter copies its neighbour's). */
  from: number | null;
  pieces: EditedPiece[];
}

export interface EditedPiece {
  text: string;
  /** The run it came from: [paragraph, run], or null for text typed outside any run. */
  run: [number, number] | null;
}

function sameStyle(a: CharStyle | undefined, b: CharStyle | undefined): boolean {
  return JSON.stringify(a ?? {}) === JSON.stringify(b ?? {});
}

/**
 * The text box after editing: the original's box settings, paragraph styles from the
 * paragraph each one came from (else the one before it), run styles from the run each piece
 * came from. Adjacent pieces with the same style merge; empty runs are dropped.
 */
export function rebuildTextBox(orig: TextBox, edited: EditedParagraph[]): TextBox {
  const src = orig.paragraphs ?? [];
  const paragraphs: Paragraph[] = [];
  // Paragraphs that came from the same one (Enter clones the line's div with its origin):
  // its chords are shared out among them by character, never copied onto each (H06-4).
  const texts = edited.map((ep) => ep.pieces.map((pc) => pc.text).join(""));
  const halves = new Map<number, number[]>();
  edited.forEach((ep, i) => {
    if (ep.from !== null) halves.set(ep.from, [...(halves.get(ep.from) ?? []), i]);
  });
  const chordsOf = new Map<number, ChordMark[]>();
  for (const [from, idx] of halves) {
    const base = src[from];
    if (!base?.chords?.length) continue;
    const parts = splitChords(
      paragraphText(base),
      idx.map((i) => texts[i] ?? ""),
      base.chords,
    );
    idx.forEach((i, k) => chordsOf.set(i, parts[k] ?? []));
  }
  let prevFrom: number | null = null;
  for (const [i, ep] of edited.entries()) {
    const fromIdx: number | null = ep.from ?? prevFrom ?? (src.length ? src.length - 1 : null);
    const base = fromIdx !== null ? src[fromIdx] : undefined;
    prevFrom = fromIdx;
    const runs: TextRun[] = [];
    // Text typed outside a run takes the style of the run before it (else the paragraph's first).
    let lastStyle: CharStyle | undefined = base?.runs?.[0]?.style;
    for (const piece of ep.pieces) {
      const style = piece.run ? src[piece.run[0]]?.runs?.[piece.run[1]]?.style : lastStyle;
      lastStyle = style;
      if (!piece.text) continue;
      const last = runs[runs.length - 1];
      if (last && sameStyle(last.style, style)) last.text = (last.text ?? "") + piece.text;
      else runs.push(style && Object.keys(style).length ? { text: piece.text, style: { ...style } } : { text: piece.text });
    }
    const p: Paragraph = {};
    if (base?.style) p.style = base.style;
    if (runs.length) p.runs = runs;
    if (base?.endStyle) p.endStyle = base.endStyle;
    // Chords stay on their words (a line typed new has none; a split line's chords go to
    // the half holding their word).
    const chords = chordsOf.get(i);
    if (chords?.length) p.chords = chords;
    paragraphs.push(p);
  }
  const out: TextBox = { ...orig };
  if (paragraphs.length) out.paragraphs = paragraphs;
  else delete out.paragraphs;
  return out;
}

/** The pieces of the unedited box (what the editor starts with). */
export function editedFromTextBox(tb: TextBox): EditedParagraph[] {
  return (tb.paragraphs ?? []).map((p, pi) => ({
    from: pi,
    pieces: (p.runs ?? []).map((r, ri) => ({ text: r.text ?? "", run: [pi, ri] as [number, number] })),
  }));
}

/** Text boxes equal for saving purposes. */
export function sameTextBox(a: TextBox, b: TextBox): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
