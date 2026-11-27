// Small view helpers over the protocol/model types.

import type { Color, Presentation, ShowState, Slide } from "$protocol";

/** One slide of the grid, in document order. */
export interface GridSlide {
  id: string;
  /** 0-based index in the whole presentation (what `triggerSlide.index` and `slideIndex` use). */
  index: number;
  label: string;
  enabled: boolean;
  sectionId: string;
  sectionName: string;
  /** CSS colour of the section (group), or null when it has none. */
  color: string | null;
  /** First slide of its section (the place for a group header). */
  sectionStart: boolean;
  /** The slide's visible text, one line per paragraph (Text and List views). */
  text: string;
}

/** Plain text of a slide: every visible text element, paragraphs on separate lines. */
export function slideText(slide: Slide): string {
  const lines: string[] = [];
  for (const el of slide.elements ?? []) {
    if (el.hidden || el.kind?.type !== "text") continue;
    for (const p of el.kind.text?.paragraphs ?? []) {
      const line = (p.runs ?? [])
        .map((r) => r.text ?? "")
        .join("")
        .replace(/\u2028/g, "\n")
        .replace(/\t/g, " ");
      lines.push(line);
    }
  }
  // Drop leading/trailing blank lines, keep inner ones (stanza breaks).
  while (lines.length && !lines[0]!.trim()) lines.shift();
  while (lines.length && !lines[lines.length - 1]!.trim()) lines.pop();
  return lines.join("\n");
}

export function cssColor(c: Color | undefined | null, alpha?: number): string | null {
  if (!c) return null;
  const a = alpha ?? c.a ?? 1;
  const to = (v: number) => Math.round(Math.max(0, Math.min(1, v)) * 255);
  return `rgba(${to(c.r)}, ${to(c.g)}, ${to(c.b)}, ${a})`;
}

/** Relative luminance (0..1) of a colour, for choosing readable text on it. */
export function luminance(c: Color): number {
  const lin = (v: number) => (v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4));
  return 0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b);
}

export function flattenSlides(p: Presentation): GridSlide[] {
  const out: GridSlide[] = [];
  let index = 0;
  for (const s of p.sections ?? []) {
    let first = true;
    for (const sl of s.slides ?? []) {
      out.push({
        id: sl.id,
        index,
        label: sl.label?.text ?? "",
        enabled: sl.enabled ?? true,
        sectionId: s.id,
        sectionName: s.name ?? "",
        color: cssColor(s.color, 1),
        sectionStart: first,
        text: slideText(sl),
      });
      first = false;
      index++;
    }
  }
  return out;
}

/** Width/height of the presentation canvas (1920×1080 when not set). */
export function aspectRatio(p: Presentation | null): number {
  const w = p?.canvasSize?.width ?? 1920;
  const h = p?.canvasSize?.height ?? 1080;
  return w > 0 && h > 0 ? w / h : 16 / 9;
}

/** Layers the CLEAR row shows, in the order operators know. */
export const CLEAR_LAYERS = [
  { layer: "slide", label: "Slide", key: "F2" },
  { layer: "media", label: "Media", key: "F3" },
  { layer: "videoInput", label: "Video Input", key: null },
  // Soft hyphen: wraps as "Announce-/ments" in a narrow panel.
  { layer: "announcements", label: "Announce\u00adments", key: null },
  { layer: "props", label: "Props", key: "F5" },
  { layer: "messages", label: "Messages", key: "F6" },
  { layer: "audio", label: "Audio", key: "F4" },
] as const;

export type ClearableLayer = (typeof CLEAR_LAYERS)[number]["layer"];

export type LayerActivity = Record<ClearableLayer, boolean>;

export function layerActivity(s: ShowState | null): LayerActivity {
  const l = s?.layers;
  return {
    slide: !!l?.slide || !!l?.foreground,
    media: !!l?.media,
    videoInput: !!l?.videoInput,
    announcements: !!l?.announcements,
    props: (l?.props.length ?? 0) > 0,
    messages: (l?.messages.length ?? 0) > 0,
    audio: !!l?.audio,
  };
}

/** Thumbnail widths we request, so the server's cache is shared between grid sizes. */
const THUMB_BUCKETS = [160, 240, 320, 480, 640, 960];

export function thumbWidthFor(cssWidth: number, dpr: number): number {
  const want = cssWidth * Math.max(1, dpr);
  return THUMB_BUCKETS.find((b) => b >= want) ?? THUMB_BUCKETS[THUMB_BUCKETS.length - 1]!;
}
