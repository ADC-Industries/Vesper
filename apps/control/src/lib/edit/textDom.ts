// The in-place text editor's DOM: a contenteditable made of one <div data-p> per paragraph and
// one <span data-r="p:r"> per run, styled with an approximation of the renderer's text (the
// server's render is what counts). Reads it back as edited paragraphs (see text.ts) and maps
// DOM selections to plain-text offsets and back.

import type { CharStyle, ParagraphStyle, TextBox } from "$protocol";
import { cssColor } from "../model";
import { effectiveParagraph, effectiveStyle, type EditedParagraph, type EditedPiece } from "./text";

/** CSS for a run at `zoom` (font size scaled by `--fs`, the shrink-to-fit factor). */
export function runCss(s: CharStyle, tb: TextBox, zoom: number): string {
  const parts: string[] = [];
  parts.push(`font-family: ${JSON.stringify(s.fontFamily ?? "sans-serif")}, sans-serif`);
  parts.push(`font-weight: ${s.fontWeight ?? 400}`);
  parts.push(`font-style: ${s.italic ? "italic" : "normal"}`);
  parts.push(`font-size: calc(var(--fs, 1) * ${((s.size ?? 72) * zoom).toFixed(3)}px)`);
  parts.push(`color: ${cssColor(s.color) ?? "#fff"}`);
  if (s.letterSpacing) parts.push(`letter-spacing: ${(s.letterSpacing * zoom).toFixed(3)}px`);
  const deco = [s.underline && s.underline !== "none" ? "underline" : "", s.strikethrough && s.strikethrough !== "none" ? "line-through" : ""].filter(Boolean);
  if (deco.length) parts.push(`text-decoration: ${deco.join(" ")}`);
  switch (s.capitalization) {
    case "allCaps":
      parts.push("text-transform: uppercase");
      break;
    case "titleCase":
    case "startCase":
      parts.push("text-transform: capitalize");
      break;
    case "smallCaps":
      parts.push("font-variant: small-caps");
      break;
  }
  if (s.baseline === "superscript") parts.push("vertical-align: super; font-size: calc(var(--fs, 1) * " + ((s.size ?? 72) * zoom * 0.6).toFixed(3) + "px)");
  if (s.baseline === "subscript") parts.push("vertical-align: sub; font-size: calc(var(--fs, 1) * " + ((s.size ?? 72) * zoom * 0.6).toFixed(3) + "px)");
  if (s.highlight) parts.push(`background: ${cssColor(s.highlight)}`);
  const o = tb.outline;
  if (o?.enabled && (o.width ?? 3) > 0) {
    // The renderer draws the outline centred on the glyph edge, under the fill; so does this.
    parts.push(`-webkit-text-stroke: ${((o.width ?? 3) * zoom).toFixed(3)}px ${cssColor(o.color ?? { r: 0, g: 0, b: 0 })}`);
    parts.push("paint-order: stroke fill");
  }
  const sh = tb.textShadow;
  if (sh?.enabled) {
    const a = ((sh.angle ?? 315) * Math.PI) / 180;
    const off = (sh.offset ?? 5) * zoom;
    const c = cssColor(sh.color ?? { r: 0, g: 0, b: 0 }, (sh.color?.a ?? 1) * (sh.opacity ?? 0.75));
    parts.push(`text-shadow: ${(off * Math.cos(a)).toFixed(2)}px ${(-off * Math.sin(a)).toFixed(2)}px ${((sh.radius ?? 5) * zoom).toFixed(2)}px ${c}`);
  }
  return parts.join("; ");
}

export function paragraphCss(p: ParagraphStyle, zoom: number): string {
  const parts: string[] = [];
  const align = p.alignment === "justified" ? "justify" : p.alignment === "natural" ? "left" : !p.alignment || p.alignment === "unknown" ? "center" : p.alignment;
  parts.push(`text-align: ${align}`);
  parts.push(`line-height: ${p.lineHeightMultiple && p.lineHeightMultiple > 0 ? p.lineHeightMultiple * 1.17 : 1.17}`);
  if (p.spaceBefore) parts.push(`margin-top: ${(p.spaceBefore * zoom).toFixed(2)}px`);
  if (p.spaceAfter) parts.push(`margin-bottom: ${(p.spaceAfter * zoom).toFixed(2)}px`);
  if (p.indentStart) parts.push(`padding-left: ${(p.indentStart * zoom).toFixed(2)}px`);
  if (p.indentEnd) parts.push(`padding-right: ${(p.indentEnd * zoom).toFixed(2)}px`);
  if (p.indentFirstLine) parts.push(`text-indent: ${(p.indentFirstLine * zoom).toFixed(2)}px`);
  return parts.join("; ");
}

/** Fills `root` with the text box's paragraphs. */
export function renderTextBox(root: HTMLElement, tb: TextBox, zoom: number) {
  root.replaceChildren();
  const paras = tb.paragraphs?.length ? tb.paragraphs : [{}];
  paras.forEach((p, pi) => {
    const div = document.createElement("div");
    div.dataset.p = String(pi);
    div.setAttribute("style", paragraphCss(effectiveParagraph(tb, p), zoom));
    const runs = p.runs ?? [];
    if (!runs.some((r) => r.text)) {
      // An empty paragraph: its end style sets the caret's size.
      const span = document.createElement("span");
      span.dataset.r = `${pi}:0`;
      span.setAttribute("style", runCss(effectiveStyle(tb, p.endStyle ?? runs[0]?.style), tb, zoom));
      span.appendChild(document.createElement("br"));
      div.appendChild(span);
    }
    runs.forEach((r, ri) => {
      if (!r.text) return;
      const span = document.createElement("span");
      span.dataset.r = `${pi}:${ri}`;
      span.setAttribute("style", runCss(effectiveStyle(tb, r.style), tb, zoom));
      span.textContent = r.text.replace(/\u2028/g, "\n");
      div.appendChild(span);
    });
    root.appendChild(div);
  });
}

interface Seg {
  node: Node;
  /** UTF-16 offset in the plain text. */
  start: number;
  /** UTF-16 length (text nodes; 0 for paragraph markers). */
  len: number;
}

function runOf(node: Node, para: Element): [number, number] | null {
  let el: Element | null = node.nodeType === Node.ELEMENT_NODE ? (node as Element) : node.parentElement;
  while (el && el !== para) {
    const r = (el as HTMLElement).dataset?.r;
    if (r) {
      const [a, b] = r.split(":").map(Number);
      if (Number.isFinite(a) && Number.isFinite(b)) return [a!, b!];
    }
    el = el.parentElement;
  }
  return null;
}

/** Paragraph containers: the root's element children (bare text at the root is one paragraph). */
function paragraphsOf(root: HTMLElement): Node[] {
  return [...root.childNodes].filter((n) => n.nodeType === Node.ELEMENT_NODE || (n.nodeType === Node.TEXT_NODE && n.textContent));
}

/** Reads the editor: edited paragraphs plus segments for selection mapping. */
export function readEditor(root: HTMLElement): { paragraphs: EditedParagraph[]; segs: Seg[]; text: string } {
  const paragraphs: EditedParagraph[] = [];
  const segs: Seg[] = [];
  let at = 0;
  let text = "";
  paragraphsOf(root).forEach((para, i) => {
    if (i > 0) {
      at += 1;
      text += "\n";
    }
    segs.push({ node: para, start: at, len: 0 });
    const from = para instanceof HTMLElement && para.dataset.p !== undefined ? Number(para.dataset.p) : null;
    const pieces: EditedPiece[] = [];
    const walker = document.createTreeWalker(para, NodeFilter.SHOW_TEXT | NodeFilter.SHOW_ELEMENT);
    const nodes: Node[] = para.nodeType === Node.TEXT_NODE ? [para] : [];
    for (let n = walker.nextNode(); n; n = walker.nextNode()) nodes.push(n);
    const hasText = nodes.some((n) => n.nodeType === Node.TEXT_NODE && n.textContent);
    nodes.forEach((n, k) => {
      const run = para.nodeType === Node.ELEMENT_NODE ? runOf(n, para as Element) : null;
      if (n.nodeType === Node.TEXT_NODE) {
        const t = (n.textContent ?? "").replace(/\n/g, "\u2028");
        segs.push({ node: n, start: at, len: t.length });
        pieces.push({ text: t, run });
        at += t.length;
        text += t;
      } else if ((n as Element).tagName === "BR") {
        // A <br> is a soft line break, except the placeholder of an empty paragraph and the
        // one the browser keeps at the very end.
        const last = !nodes.slice(k + 1).some((m) => m.nodeType === Node.TEXT_NODE && m.textContent);
        if (hasText && !last) {
          segs.push({ node: n, start: at, len: 1 });
          pieces.push({ text: "\u2028", run });
          at += 1;
          text += "\u2028";
        }
      }
    });
    paragraphs.push({ from: Number.isFinite(from) ? from : null, pieces });
  });
  return { paragraphs, segs, text };
}

/** UTF-16 offset in the plain text of a DOM point inside the editor. */
export function offsetOf(root: HTMLElement, node: Node, offset: number): number {
  const { segs, text } = readEditor(root);
  if (node.nodeType === Node.TEXT_NODE) {
    const s = segs.find((x) => x.node === node);
    if (s) return s.start + Math.min(offset, s.len);
  }
  const r = document.createRange();
  try {
    r.setStart(node, offset);
  } catch {
    return text.length;
  }
  let best = 0;
  for (const s of segs) {
    // The segment starts at or before the point: the point is at or after its start.
    const cmp = r.comparePoint(s.node, 0);
    if (cmp <= 0) best = s.node.nodeType === Node.TEXT_NODE ? s.start + s.len : s.start;
    else break;
  }
  return best;
}

/** Selects UTF-16 offsets [a, b) of the plain text in the editor. */
export function selectOffsets(root: HTMLElement, a: number, b: number) {
  const { segs } = readEditor(root);
  const point = (off: number): [Node, number] => {
    let found: [Node, number] = [root, root.childNodes.length];
    for (const s of segs) {
      if (s.node.nodeType === Node.TEXT_NODE) {
        if (off >= s.start && off <= s.start + s.len) return [s.node, off - s.start];
      } else if (off === s.start && s.len === 0) found = [s.node, 0];
    }
    return found;
  };
  const [sn, so] = point(a);
  const [en, eo] = point(b);
  const sel = window.getSelection();
  if (!sel) return;
  const r = document.createRange();
  r.setStart(sn, so);
  r.setEnd(en, eo);
  sel.removeAllRanges();
  sel.addRange(r);
}
