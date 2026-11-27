// A theme's template slides in the slide editor (docs/THEMES-PLAN.md section 9): the starter
// layouts "+" offers (built like the server's `themes::starter`, on the theme's canvas) and the
// slot roles a text box can take.

import type { Element, Placeholder, Size, Slide, TextAlign } from "$protocol";

/** A starter layout for a new template slide. */
export type TemplateKind = "lyrics" | "title" | "scripture" | "blank";

export const TEMPLATE_KINDS: { kind: TemplateKind; label: string }[] = [
  { kind: "lyrics", label: "Lyrics" },
  { kind: "title", label: "Title" },
  { kind: "scripture", label: "Scripture" },
  { kind: "blank", label: "Blank" },
];

/** Roles a theme's text box can fill; "" is none (decoration). */
export const SLOT_ROLES: { slot: string; label: string }[] = [
  { slot: "", label: "None (decoration)" },
  { slot: "lyrics", label: "Lyrics" },
  { slot: "title", label: "Title" },
  { slot: "scripture", label: "Scripture" },
  { slot: "reference", label: "Reference" },
];

/** The label of a slot role ("lyrics" → "Lyrics"; unknown names as they are). */
export function roleLabel(slot: string | undefined): string {
  if (!slot) return "None";
  return SLOT_ROLES.find((r) => r.slot === slot)?.label ?? slot;
}

/** The roles of a template slide's slots, in z order (bottom first), without repeats. */
export function slideRoles(slide: Slide): string[] {
  const out: string[] = [];
  for (const e of slide.elements ?? []) {
    const s = e.placeholder?.slot;
    if (e.placeholder && !out.includes(s ?? "")) out.push(s ?? "");
  }
  return out;
}

/** The placeholder for a role picked in the inspector (`null`: no role). Media elements take a
 * media slot; a text box keeps what its placeholder accepted. */
export function placeholderFor(el: Element, slot: string): Placeholder | null {
  if (!slot) return null;
  const accepts = el.kind?.type === "text" ? (el.placeholder?.accepts ?? "text") : "media";
  return { slot, accepts };
}

function slot(id: string, name: string, role: string, x: number, y: number, w: number, h: number, size: number, align: TextAlign, bold: boolean): Element {
  return {
    id,
    name,
    bounds: { x, y, width: w, height: h },
    placeholder: { slot: role },
    themed: true,
    kind: {
      type: "text",
      text: {
        paragraphs: [],
        defaultStyle: { fontFamily: "Arial", fontWeight: bold ? 700 : 400, size: Math.round(size), color: { r: 1, g: 1, b: 1 } },
        defaultParagraph: { alignment: align },
        verticalAlign: "middle",
        fit: "shrinkToFit",
      },
    },
  } as Element;
}

/** A new template slide of `kind` on a `canvas`-sized theme, white Arial on black, labelled
 * with its name (the template's name in the theme document). */
export function templateSlide(kind: TemplateKind, canvas: Size | undefined, uuid: () => string, name?: string): Slide {
  const w = Math.max(64, canvas?.width ?? 1920);
  const h = Math.max(64, canvas?.height ?? 1080);
  const label = name ?? TEMPLATE_KINDS.find((k) => k.kind === kind)!.label;
  const elements: Element[] =
    kind === "lyrics"
      ? [slot(uuid(), "Lyrics", "lyrics", w * 0.05, h * 0.05, w * 0.9, h * 0.9, h * 0.074, "center", false)]
      : kind === "title"
        ? [slot(uuid(), "Title", "title", w * 0.05, h * 0.3, w * 0.9, h * 0.4, h * 0.11, "center", true)]
        : kind === "scripture"
          ? [
              slot(uuid(), "Scripture", "scripture", w * 0.05, h * 0.05, w * 0.9, h * 0.72, h * 0.06, "left", false),
              slot(uuid(), "Reference", "reference", w * 0.05, h * 0.8, w * 0.9, h * 0.14, h * 0.045, "right", true),
            ]
          : [];
  return { id: uuid(), label: { text: label }, background: { r: 0, g: 0, b: 0 }, elements } as Slide;
}
