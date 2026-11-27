// Dual-language slides (batch 16): a slide's second-language layer (`Slide.translation`, see
// crates/model/src/language.rs) as Oriel edits it. Pure functions, unit tested.
//
// - Reflow edits the second text of each slide's primary text element beside its primary text;
//   a change is one `setSlideTranslation` op carrying the slide's whole layer.
// - "Paste two languages" splits pasted text into slides (stanzas, blank-line separated) and,
//   within each, the two languages: alternating lines, or a separator (a line of its own, or
//   inside a line: `primary / second`). Pasted ChordPro is read by the server
//   (`POST /chordpro/parse` with the same split, or `{ci: …}` comments) so its chords stay on
//   the first language (`applyChordPro` after the text).

import type { LanguageDisplay, Paragraph, Presentation, PresentationOp, SecondSplit, Slide, SlideTranslation, TranslateJob, TranslatedSlide, TranslationModel } from "$protocol";
import { inlineChords } from "./chords";
import { primaryTextIndex } from "./reflow";

/** The display modes a look's screen entry offers, with their labels. */
export const LANGUAGE_DISPLAYS: { value: LanguageDisplay; label: string; hint: string }[] = [
  { value: "auto", label: "Auto", hint: "Both, as the slide prefers (parallel Bible: side by side), else stacked" },
  { value: "primary", label: "First only", hint: "The first language only: the slide as authored" },
  { value: "second", label: "Second only", hint: "The second language only, in the first one's place and style" },
  { value: "stacked", label: "Stacked", hint: "Both: the second text under the first, smaller" },
  { value: "sideBySide", label: "Side by side", hint: "Both: the text box split in two" },
];

/** A few language codes offered in pickers (any BCP 47 code can be typed). */
export const COMMON_LANGUAGES: { code: string; name: string }[] = [
  { code: "en", name: "English" },
  { code: "es", name: "Spanish" },
  { code: "pt", name: "Portuguese" },
  { code: "fr", name: "French" },
  { code: "de", name: "German" },
  { code: "ko", name: "Korean" },
  { code: "zh", name: "Chinese" },
  { code: "tl", name: "Tagalog" },
  { code: "vi", name: "Vietnamese" },
  { code: "uk", name: "Ukrainian" },
  { code: "ar", name: "Arabic" },
  { code: "sw", name: "Swahili" },
];

function paragraphsText(ps: Paragraph[] | undefined): string {
  return (ps ?? []).map((p) => (p.runs ?? []).map((r) => r.text ?? "").join("")).join("\n");
}

/** The second text of `elementId` on `slide` ("" when it has none). */
export function secondText(slide: Slide | undefined, elementId: string | null): string {
  if (!slide || !elementId) return "";
  const t = slide.translation?.texts?.find((x) => x.element === elementId);
  return paragraphsText(t?.paragraphs);
}

/** The slide's second language code ("" when none). */
export function slideLanguage(slide: Slide | undefined): string {
  return slide?.translation?.language ?? "";
}

/** The most common second language of a presentation's slides ("" when none has one). */
export function presentationLanguage(slides: Slide[]): string {
  const n = new Map<string, number>();
  for (const s of slides) {
    const l = s.translation?.language;
    if (l) n.set(l, (n.get(l) ?? 0) + 1);
  }
  let best = "";
  let max = 0;
  for (const [l, c] of n) if (c > max) [best, max] = [l, c];
  return best;
}

function plainParagraphs(text: string): Paragraph[] {
  return text.split("\n").map((l) => {
    const t = l.replace(/\r$/, "");
    return t ? { runs: [{ text: t }] } : {};
  });
}

/**
 * The slide's layer with `elementId`'s second text set to `text` (other elements' texts and
 * the preferred layout kept); `null` when nothing visible is left.
 */
export function withSecondText(slide: Slide, elementId: string, language: string, text: string): SlideTranslation | null {
  const old = slide.translation ?? {};
  const texts = (old.texts ?? []).filter((t) => t.element !== elementId);
  if (text.trim()) texts.push({ element: elementId, paragraphs: plainParagraphs(text) });
  const visible = texts.some((t) => paragraphsText(t.paragraphs).trim());
  if (!visible) return null;
  const lang = language.trim() || old.language || "";
  return { ...(lang ? { language: lang } : {}), texts, ...(old.layout ? { layout: old.layout } : {}) };
}

/** The op that sets `elementId`'s second text on `slide`. */
export function secondTextOp(slide: Slide, elementId: string, language: string, text: string): PresentationOp {
  return { type: "setSlideTranslation", slide: slide.id, translation: withSecondText(slide, elementId, language, text) };
}

/** How pasted two-language text is split. */
export type TwoLanguageSplit = { type: "alternating" } | { type: "separator"; separator: string };

/**
 * Splits pasted text into slides of `[primary, second]`: stanzas (one slide each) separated by
 * blank lines, the languages by `split`. Lines are trimmed; a stanza without second text gives
 * "" as its second.
 */
export function splitTwoLanguages(text: string, split: TwoLanguageSplit): [string, string][] {
  const stanzas: string[][] = [[]];
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (!line) {
      if (stanzas[stanzas.length - 1]!.length) stanzas.push([]);
    } else stanzas[stanzas.length - 1]!.push(line);
  }
  return stanzas
    .filter((s) => s.length)
    .map((lines) => {
      const a: string[] = [];
      const b: string[] = [];
      const sep = split.type === "separator" ? split.separator.trim() : "";
      if (sep) {
        let below = false;
        for (const l of lines) {
          const at = l.indexOf(sep);
          if (l === sep) below = true;
          else if (at >= 0) {
            a.push(l.slice(0, at).trim());
            b.push(l.slice(at + sep.length).trim());
          } else (below ? b : a).push(l);
        }
      } else lines.forEach((l, i) => (i % 2 === 0 ? a : b).push(l));
      return [a.join("\n"), b.filter((l) => l).join("\n")] as [string, string];
    });
}

/** Pasted text that looks like ChordPro: a `{directive}` line or a chord in brackets. */
export function looksLikeChordPro(text: string): boolean {
  return /^\s*\{[^{}]+\}\s*$/m.test(text) || /\[[A-G][#b♯♭]?(?:maj|min|m|dim|aug|sus|add|[0-9]|\/[A-G][#b♯♭]?|[#b♯♭+°ø()])*\]/.test(text);
}

/** How "Paste two languages" separates the languages ("comments": ChordPro `{ci: …}` lines). */
export type PasteMode = "alternating" | "line" | "inline" | "comments";

/** The ChordPro reader's split for a paste mode. */
export function chordProSplit(mode: PasteMode, separator: string): SecondSplit {
  switch (mode) {
    case "alternating":
      return { type: "alternating" };
    case "line":
      return { type: "line" };
    case "inline":
      return { type: "inline", separator };
    case "comments":
      return { type: "comments" };
  }
}

/**
 * A ChordPro song read by the server (`POST /chordpro/parse` with a split), as pasted slides:
 * `[first, second]` per slide in order, each slide's first-language lines with their chords
 * inline (for `applyChordPro`), and the second language's code.
 */
export function chordProPaste(p: Presentation): { pairs: [string, string][]; chords: string[]; language: string } {
  const pairs: [string, string][] = [];
  const chords: string[] = [];
  let language = "";
  for (const sl of (p.sections ?? []).flatMap((s) => s.slides ?? [])) {
    const i = primaryTextIndex(sl);
    const el = i >= 0 ? sl.elements?.[i] : undefined;
    const ps = el?.kind?.type === "text" ? (el.kind.text?.paragraphs ?? []) : [];
    pairs.push([paragraphsText(ps), el ? secondText(sl, el.id) : ""]);
    chords.push(ps.filter((x) => x.chords?.length).map((x) => inlineChords(paragraphsText([x]), x.chords)).join("\n"));
    language ||= sl.translation?.language ?? "";
  }
  return { pairs, chords, language };
}

/**
 * The ChordPro paste's chords for the slides it filled (the first `used`): one `applyChordPro`
 * after the text, so each first-language line takes its chords. Only when the paste replaced
 * the first-language text (else the lines may match nothing, and the edit would be refused).
 */
export function chordOps(chords: string[], used: number, withPrimary: boolean): PresentationOp[] {
  const text = chords.slice(0, used).filter((c) => c).join("\n\n");
  return withPrimary && text ? [{ type: "applyChordPro", text }] : [];
}

/** A Reflow block as the paste needs it. */
export interface PasteTarget {
  slideId: string;
  elementId: string | null;
  text: string;
}

/**
 * The ops that put pasted `[primary, second]` slides onto `targets` in order: the second text
 * always, the primary text too when `withPrimary`. Slides without a text element get only the
 * primary (the server adds the element). `leftover` counts pasted slides with no slide to go
 * to.
 */
export function pasteOps(
  targets: PasteTarget[],
  slides: Map<string, Slide>,
  pairs: [string, string][],
  language: string,
  withPrimary: boolean,
): { ops: PresentationOp[]; used: number; leftover: number } {
  const ops: PresentationOp[] = [];
  const n = Math.min(targets.length, pairs.length);
  for (let i = 0; i < n; i++) {
    const t = targets[i]!;
    const [primary, second] = pairs[i]!;
    const slide = slides.get(t.slideId);
    if (withPrimary && primary !== t.text) ops.push({ type: "setSlideText", slide: t.slideId, text: primary, expectText: t.text });
    if (slide && t.elementId) ops.push(secondTextOp(slide, t.elementId, language, second));
  }
  return { ops, used: n, leftover: Math.max(0, pairs.length - targets.length) };
}

// ---- machine translation (docs/DUAL-LANGUAGE-PLAN.md §5) ------------------------------------

/** The banner shown with every machine translation (the server sends the same text). */
export const MACHINE_NOTICE = "Machine translation: check it, and check your licence allows a translation of this song";

/** A target language the box's models offer from a source language. */
export type TranslationTarget = { code: string; name: string; pair: string; ready: boolean };

/** The languages `models` translate `from` into (by primary tag: `en-US` uses `en` models). */
export function translationTargets(models: TranslationModel[], from: string): TranslationTarget[] {
  const src = (from || "en").split(/[-_]/)[0]!.toLowerCase();
  const out: TranslationTarget[] = [];
  for (const m of models) {
    if (m.from.toLowerCase() !== src) continue;
    for (const code of m.to) out.push({ code, name: languageName(code), pair: m.id, ready: m.status === "ready" });
  }
  return out;
}

/** A language's name for pickers ("Spanish", "Portuguese (pt-BR)", else the code). */
export function languageName(code: string): string {
  const name = COMMON_LANGUAGES.find((l) => l.code === code.split("-")[0]?.toLowerCase())?.name;
  if (!name) return code;
  return code.includes("-") ? `${name} (${code})` : name;
}

/** The source languages `models` translate from, by name (English first). */
export function translationSources(models: TranslationModel[]): { code: string; name: string }[] {
  const codes = [...new Set(models.map((m) => m.from.toLowerCase()))];
  return codes
    .map((code) => ({ code, name: languageName(code) }))
    .sort((a, b) => (a.code === "en" ? -1 : b.code === "en" ? 1 : a.name.localeCompare(b.name)));
}

/**
 * The source language a translation starts from: the first of `known` that some model
 * translates from (by primary tag), else English, else the first one offered. The Translate
 * dialog passes `translateSourceOrder`.
 */
export function defaultSource(sources: { code: string }[], known: (string | null | undefined)[]): string {
  const offered = (c: string | null | undefined) => {
    const tag = (c ?? "").split(/[-_]/)[0]!.toLowerCase();
    return tag && sources.some((s) => s.code === tag) ? tag : null;
  };
  for (const k of known) {
    const c = offered(k);
    if (c) return c;
  }
  return offered("en") ?? sources[0]?.code ?? "en";
}

/** The proposed second text of `elementId` in a preview's slide ("" when none). */
export function proposedText(p: TranslatedSlide | undefined, elementId: string | null): string {
  if (!p || !elementId) return "";
  return paragraphsText(p.translation.texts?.find((t) => t.element === elementId)?.paragraphs);
}

/** Keep: one `setSlideTranslation` per proposed slide (the layer keeps its `machine` mark). */
export function keepTranslationOps(slides: TranslatedSlide[]): PresentationOp[] {
  return slides.map((s) => ({ type: "setSlideTranslation", slide: s.slide, translation: s.translation }));
}

/** The slide's second text is a kept machine translation nobody has edited since. */
export function isMachineTranslation(slide: Slide | undefined): boolean {
  return !!slide?.translation?.machine;
}

const mb = (b: number) => Math.round(b / 1_000_000);

/** What a preview is doing, for its progress line. */
export function translateJobText(job: TranslateJob): string {
  switch (job.status) {
    case "queued":
      return "Waiting for the translation engine…";
    case "downloading":
      return job.downloadSize ? `Downloading the ${job.model} model (first use): ${mb(job.downloaded ?? 0)} of ${mb(job.downloadSize)} MB` : `Downloading the ${job.model} model (first use)…`;
    case "translating":
      return `Translating: ${job.done} of ${job.total} ${job.total === 1 ? "slide" : "slides"}`;
    case "done":
      return `${job.slides.length} ${job.slides.length === 1 ? "slide" : "slides"} translated${job.seconds != null ? ` in ${job.seconds.toFixed(1)} s` : ""}`;
    case "failed":
      return `Translation failed: ${job.detail ?? "unknown error"}`;
    case "cancelled":
      return "Translation cancelled";
  }
}

/**
 * A BCP 47 language code in its usual case (`pt_br` → `pt-BR`, `zh-hant-tw` → `zh-Hant-TW`), or
 * null when `code` isn't one: the server's check (`model::language_code`), so a typed code is
 * refused before it is sent.
 */
export function languageCode(code: string): string | null {
  const c = code.trim();
  if (!c || c.length > 35) return null;
  const out: string[] = [];
  for (const [i, tag] of c.split(/[-_]/).entries()) {
    const letters = /^[A-Za-z]+$/.test(tag);
    const ok = i === 0 ? letters && tag.length >= 2 && tag.length <= 8 : /^[A-Za-z0-9]{1,8}$/.test(tag);
    if (!ok) return null;
    const lower = tag.toLowerCase();
    out.push(i > 0 && letters && tag.length === 2 ? tag.toUpperCase() : i > 0 && letters && tag.length === 4 ? lower[0]!.toUpperCase() + lower.slice(1) : lower);
  }
  return out.join("-");
}

/**
 * Where the Translate dialog looks for the source language, in order (owner, 2026-11-26): the
 * presentation's own language, then what the operator last translated it from (this browser),
 * then its Scripture translation's language; `defaultSource` falls back to English.
 */
export function translateSourceOrder(known: { presentation?: string | null; remembered?: string | null; scripture?: string | null }): (string | null | undefined)[] {
  return [known.presentation, known.remembered, known.scripture];
}

/** The Language field's choices: the common languages, plus `current` when it isn't one of them. */
export function languageChoices(current: string | null | undefined): { code: string; name: string }[] {
  const list = COMMON_LANGUAGES.map((l) => ({ code: l.code, name: l.name }));
  if (current && !list.some((l) => l.code === current)) list.push({ code: current, name: languageName(current) });
  return list;
}
