// Copy / Paste Style for text objects (client-side: the server has no style clipboard). Copy
// takes the text box's look (the first character's style, the first paragraph's alignment and
// spacing, and the box settings: fit, margins, outline, text shadow, line mask, vertical
// alignment); Paste applies it to whole text boxes through `setTextStyle` and `setTextBox`.

import type { CharStyle, Element, ParagraphStyle, PresentationOp, TextBox } from "$protocol";
import { DEFAULT_OUTLINE, DEFAULT_SHADOW, textBoxOp, textStyleOps } from "../../edit/ops";
import { effectiveParagraph, styleAt } from "../../edit/text";

type BoxFields = Omit<Extract<PresentationOp, { type: "setTextBox" }>, "type" | "slide" | "element">;

export interface TextStyleClip {
  character: CharStyle;
  paragraph: ParagraphStyle;
  box: BoxFields;
}

/** The look of a text object, or null for other objects. */
export function copyTextStyle(el: Element): TextStyleClip | null {
  if (el.kind?.type !== "text") return null;
  const tb: TextBox = el.kind.text ?? {};
  const character: CharStyle = { ...styleAt(tb, 0) };
  // A paragraph's list marker belongs to its content, not its look.
  const { list: _list, ...paragraph } = effectiveParagraph(tb, tb.paragraphs?.[0] ?? null);
  const box: BoxFields = {
    outline: structuredClone(tb.outline ?? { ...DEFAULT_OUTLINE }),
    textShadow: structuredClone(tb.textShadow ?? { ...DEFAULT_SHADOW }),
  };
  if (tb.fit !== undefined) box.fit = tb.fit;
  if (tb.verticalAlign !== undefined) box.verticalAlign = tb.verticalAlign;
  if (tb.margins !== undefined) box.margins = structuredClone(tb.margins);
  if (tb.lineMask !== undefined) box.lineMask = structuredClone(tb.lineMask);
  return { character, paragraph, box };
}

/** The ops that give a text object the copied look (one batch per target), or [] for others. */
export function pasteTextStyleOps(slide: string, target: Element, clip: TextStyleClip): PresentationOp[] {
  if (target.kind?.type !== "text") return [];
  return [
    ...textStyleOps(slide, target, { character: clip.character, paragraph: clip.paragraph }, null),
    textBoxOp(slide, target.id, clip.box),
  ];
}
