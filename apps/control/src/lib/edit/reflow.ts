// Reflow: the presentation as one text block per slide, and the mapping from what the operator
// does in it to edit ops. Pure functions, unit tested.
//
// Which element holds a slide's text: its **primary text element**, the same rule as the
// server (crates/vesper-server/src/edit.rs): the first text element with a text placeholder slot,
// else the largest text box without a data link. A slide without one gets one from the
// server on its first edit (in the presentation's prevailing style).
//
// Slide breaks: **Ctrl+Enter** (or the Insert Slide Break button) splits the slide at the
// caret's paragraph. Enter is a new line inside the slide; blank lines are ordinary lines.

import type { Color, Paragraph, Presentation, PresentationOp, Section, Slide } from "$protocol";
import { cssColor } from "../model";
import { uuid } from "./ids";

export interface ReflowBlock {
  slideId: string;
  /** 0-based index in document order. */
  index: number;
  sectionId: string;
  sectionName: string;
  /** CSS colour of the group, or null. */
  color: string | null;
  /** First slide of its section (where the group label starts). */
  sectionStart: boolean;
  /** Slides in this block's section (the group label spans them). */
  sectionSize: number;
  /** The primary text element's text (`\n` between paragraphs), "" when there is none. */
  text: string;
  elementId: string | null;
}

/** A group the picker offers: a section name / colour, or a global group. */
export interface GroupChoice {
  name: string;
  color: Color | null;
  globalGroup?: { id: string; name?: string };
}

function paragraphText(p: Paragraph): string {
  return (p.runs ?? []).map((r) => r.text ?? "").join("");
}

/** The slide's primary text element index (see the module docs), or -1. */
export function primaryTextIndex(slide: Slide): number {
  const els = slide.elements ?? [];
  const slot = els.findIndex(
    (e) => e.kind?.type === "text" && !!e.placeholder && (e.placeholder.accepts ?? "text") !== "media",
  );
  if (slot >= 0) return slot;
  let best = -1;
  let area = -1;
  els.forEach((e, i) => {
    if (e.kind?.type !== "text" || e.dataLink) return;
    const a = Math.abs(e.bounds?.width ?? 0) * Math.abs(e.bounds?.height ?? 0);
    if (a > area) {
      area = a;
      best = i;
    }
  });
  return best;
}

/** The text Reflow edits for a slide. */
export function primaryText(slide: Slide): string {
  const i = primaryTextIndex(slide);
  const el = i >= 0 ? slide.elements?.[i] : undefined;
  if (!el || el.kind?.type !== "text") return "";
  return (el.kind.text?.paragraphs ?? []).map(paragraphText).join("\n");
}

export function reflowBlocks(p: Presentation): ReflowBlock[] {
  const out: ReflowBlock[] = [];
  let index = 0;
  for (const s of p.sections ?? []) {
    const slides = s.slides ?? [];
    slides.forEach((sl, k) => {
      const i = primaryTextIndex(sl);
      out.push({
        slideId: sl.id,
        index: index++,
        sectionId: s.id,
        sectionName: s.name ?? "",
        color: cssColor(s.color, 1),
        sectionStart: k === 0,
        sectionSize: slides.length,
        text: primaryText(sl),
        elementId: i >= 0 ? (sl.elements?.[i]?.id ?? null) : null,
      });
    });
  }
  return out;
}

/**
 * Where Ctrl+Enter at `caret` splits `text`: the paragraph index that starts the new slide,
 * and the text to save first (a `\n` is inserted when the caret is inside a line).
 */
export function splitPoint(text: string, caret: number): { text: string; atParagraph: number } {
  const c = Math.max(0, Math.min(caret, text.length));
  const before = text.slice(0, c);
  const line = before.split("\n").length - 1;
  const atLineStart = c === 0 || text[c - 1] === "\n";
  const atLineEnd = c === text.length || text[c] === "\n";
  if (atLineStart) return { text, atParagraph: line };
  if (atLineEnd) return { text, atParagraph: line + 1 };
  return { text: `${before}\n${text.slice(c)}`, atParagraph: line + 1 };
}

/**
 * Ops for Ctrl+Enter / Insert Slide Break (the current text is saved in the same batch). The
 * edit queue fills in `expectText` when it sends, after the typing queued before it.
 */
export function splitOps(slideId: string, text: string, serverText: string | undefined, caret: number): PresentationOp[] {
  const sp = splitPoint(text, caret);
  const ops: PresentationOp[] = [];
  if (sp.text !== serverText) ops.push({ type: "setSlideText", slide: slideId, text: sp.text });
  ops.push({ type: "splitSlide", slide: slideId, atParagraph: sp.atParagraph });
  return ops;
}

/** Backspace at the start of a slide: append it to the previous slide. */
export function mergeOps(previousId: string, slideId: string): PresentationOp[] {
  return [{ type: "mergeSlides", first: previousId, second: slideId }];
}

/**
 * Assigns a group to a slide, as the Reflow group picker does: the slide starts a group with
 * that name and colour. The first slide of a section renames its section; a slide inside a
 * section splits it, and the slides from it to the section's end move into a new section.
 */
export function groupOps(p: Presentation, slideId: string, g: GroupChoice, newId: () => string = uuid): PresentationOp[] {
  const sections = p.sections ?? [];
  const si = sections.findIndex((s) => (s.slides ?? []).some((x) => x.id === slideId));
  if (si < 0) return [];
  const sec = sections[si]!;
  const slides = sec.slides ?? [];
  const k = slides.findIndex((x) => x.id === slideId);
  const gg = g.globalGroup ? { id: g.globalGroup.id, ...(g.globalGroup.name ? { name: g.globalGroup.name } : {}) } : null;
  if (k === 0) {
    return [{ type: "setSection", section: sec.id, name: g.name, color: g.color, globalGroup: gg }];
  }
  const section: Section = { id: newId(), name: g.name, ...(g.color ? { color: g.color } : {}), ...(gg ? { globalGroup: gg } : {}) };
  const ops: PresentationOp[] = [{ type: "addSection", after: sec.id, section }];
  let after: string | undefined;
  for (const s of slides.slice(k)) {
    ops.push({ type: "moveSlide", slide: s.id, toSection: section.id, ...(after ? { after } : {}) });
    after = s.id;
  }
  return ops;
}

/** The groups a picker offers: the presentation's sections (by name), then global groups. */
export function groupChoices(p: Presentation | null, global: GroupChoice[]): { sections: GroupChoice[]; global: GroupChoice[] } {
  const seen = new Set<string>();
  const sections: GroupChoice[] = [];
  for (const s of p?.sections ?? []) {
    const name = s.name ?? "";
    if (!name || seen.has(name.toLowerCase())) continue;
    seen.add(name.toLowerCase());
    sections.push({ name, color: s.color ?? null, ...(s.globalGroup ? { globalGroup: s.globalGroup } : {}) });
  }
  return { sections, global: global.filter((g) => !seen.has(g.name.toLowerCase())) };
}

/** Built-in groups when the server has none (typical song parts, in familiar colours). */
export const DEFAULT_GROUPS: GroupChoice[] = [
  { name: "Verse 1", color: { r: 0.18, g: 0.45, b: 0.9, a: 1 } },
  { name: "Verse 2", color: { r: 0.18, g: 0.45, b: 0.9, a: 1 } },
  { name: "Verse 3", color: { r: 0.18, g: 0.45, b: 0.9, a: 1 } },
  { name: "Pre-Chorus", color: { r: 0.62, g: 0.35, b: 0.85, a: 1 } },
  { name: "Chorus", color: { r: 0.85, g: 0.2, b: 0.25, a: 1 } },
  { name: "Bridge", color: { r: 0.55, g: 0.3, b: 0.85, a: 1 } },
  { name: "Tag", color: { r: 0.95, g: 0.6, b: 0.1, a: 1 } },
  { name: "Intro", color: { r: 0.2, g: 0.7, b: 0.45, a: 1 } },
  { name: "Outro", color: { r: 0.2, g: 0.7, b: 0.45, a: 1 } },
];

/** A short content hash of a slide: changes whenever the slide does (thumbnail cache-buster). */
export function slideHash(slide: Slide): string {
  const s = JSON.stringify(slide);
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return (h >>> 0).toString(36);
}
