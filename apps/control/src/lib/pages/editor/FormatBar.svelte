<script lang="ts">
  // The text formatting bar above the canvas: font, size, bold / italic / underline /
  // strikethrough, all caps, alignment and colour for the selected text objects (the selected
  // characters while one is edited in place, as the Text tab does); then Snap to centre, Rulers
  // and the canvas size.
  import AlignLeft from "lucide-svelte/icons/align-left";
  import AlignCenter from "lucide-svelte/icons/align-center";
  import AlignRight from "lucide-svelte/icons/align-right";
  import AlignJustify from "lucide-svelte/icons/align-justify";
  import type { CharStyle, Color, Element, ParagraphStyle, TextAlign } from "$protocol";
  import type { EditState } from "../../edit/editState.svelte";
  import { effectiveParagraph, paragraphAt, styleAt } from "../../edit/text";
  import { textStyleOps } from "../../edit/ops";
  import { cssColor } from "../../model";
  import ColorIn from "./ColorIn.svelte";
  import Num from "./Num.svelte";

  let { st }: { st: EditState } = $props();

  let texts = $derived(st.selected.filter((e) => e.kind?.type === "text" && !e.locked));
  let first = $derived<Element | null>(texts[0] ?? null);
  let tb = $derived(first?.kind?.type === "text" ? (first.kind.text ?? {}) : {});
  let range = $derived(first && st.editingText === first.id ? st.textRange : null);
  let cur = $derived<CharStyle>(first ? styleAt(tb, range?.start ?? 0) : {});
  let para = $derived<ParagraphStyle>(first ? effectiveParagraph(tb, paragraphAt(tb, range?.start ?? 0)) : {});
  let off = $derived(!texts.length);
  let families = $derived(st.fonts?.families ?? []);
  let size = $derived(st.canvasSize());

  /** One undo step: the one text object (its selected characters while edited), or all of them. */
  function apply(label: string, style: { character?: CharStyle; paragraph?: ParagraphStyle }) {
    const slide = st.slideId;
    if (!slide || !texts.length) return;
    if (texts.length === 1) void st.textStyle(texts[0]!, label, style);
    else void st.run(label, texts.flatMap((el) => textStyleOps(slide, el, style, null)));
  }
  function char(label: string, character: CharStyle) {
    apply(label, { character });
  }
  function paragraph(label: string, p: ParagraphStyle) {
    apply(label, { paragraph: p });
  }

  let bold = $derived((cur.fontWeight ?? 400) >= 600);
  let under = $derived(!!cur.underline && cur.underline !== "none");
  let strike = $derived(!!cur.strikethrough && cur.strikethrough !== "none");
  let caps = $derived(cur.capitalization === "allCaps");

  const ALIGNS: { v: TextAlign; icon: typeof AlignLeft; label: string }[] = [
    { v: "left", icon: AlignLeft, label: "Align left" },
    { v: "center", icon: AlignCenter, label: "Centre" },
    { v: "right", icon: AlignRight, label: "Align right" },
    { v: "justified", icon: AlignJustify, label: "Justify" },
  ];
  const SWATCHES: { label: string; c: Color }[] = [
    { label: "White", c: { r: 1, g: 1, b: 1 } },
    { label: "Warm yellow", c: { r: 1, g: 0.85, b: 0.4 } },
    { label: "Light violet", c: { r: 0.79, g: 0.75, b: 1 } },
    { label: "Black", c: { r: 0, g: 0, b: 0 } },
  ];
  const same = (a: Color | undefined, b: Color) => !!a && Math.abs(a.r - b.r) < 0.01 && Math.abs(a.g - b.g) < 0.01 && Math.abs(a.b - b.b) < 0.01;
</script>

<div class="fbar" role="toolbar" aria-label="Text formatting" data-format-bar>
  <select class="font" value={cur.fontFamily ?? ""} disabled={off} onchange={(e) => char("Font", { fontFamily: e.currentTarget.value })} aria-label="Font">
    {#if cur.fontFamily && !families.some((f) => f.family === cur.fontFamily)}<option value={cur.fontFamily} disabled>{cur.fontFamily} (not installed)</option>{/if}
    {#if off}<option value="">Font</option>{/if}
    {#each families as f (f.family)}<option value={f.family}>{f.family}</option>{/each}
  </select>
  <Num label="Font size" value={off ? null : (cur.size ?? 72)} min={1} max={2000} width={58} digits={0} disabled={off} onchange={(v) => char("Font Size", { size: v })} />
  <div class="grp">
    <button class="tb b" class:on={bold} disabled={off} aria-pressed={bold} aria-label="Bold" title="Bold" onclick={() => char("Bold", { fontWeight: bold ? 400 : 700 })}>B</button>
    <button class="tb i" class:on={!!cur.italic} disabled={off} aria-pressed={!!cur.italic} aria-label="Italic" title="Italic" onclick={() => char("Italic", { italic: !cur.italic })}>I</button>
    <button class="tb u" class:on={under} disabled={off} aria-pressed={under} aria-label="Underline" title="Underline" onclick={() => char("Underline", { underline: under ? "none" : "single" })}>U</button>
    <button class="tb s" class:on={strike} disabled={off} aria-pressed={strike} aria-label="Strikethrough" title="Strikethrough" onclick={() => char("Strikethrough", { strikethrough: strike ? "none" : "single" })}>S</button>
    <button class="tb" class:on={caps} disabled={off} aria-pressed={caps} aria-label="All caps" title="All caps" onclick={() => char("Capitalization", { capitalization: caps ? "none" : "allCaps" })}>AA</button>
  </div>
  <span class="sep"></span>
  <div class="grp" role="group" aria-label="Alignment">
    {#each ALIGNS as a (a.v)}
      {@const Icon = a.icon}
      <button class="tb" class:on={!off && (para.alignment ?? "center") === a.v} disabled={off} aria-pressed={!off && (para.alignment ?? "center") === a.v} aria-label={a.label} title={a.label} onclick={() => paragraph(a.label, { alignment: a.v })}><Icon size={16} strokeWidth={1.8} /></button>
    {/each}
  </div>
  <span class="sep"></span>
  <div class="grp sw" role="group" aria-label="Text colour">
    {#each SWATCHES as s (s.label)}
      <button class="dot" class:on={!off && same(cur.color, s.c)} disabled={off} style:background={cssColor(s.c)} aria-label={`Text colour: ${s.label}`} title={s.label} onclick={() => char("Text Colour", { color: s.c })}></button>
    {/each}
    <ColorIn color={off ? null : cur.color} label="Custom text colour" disabled={off} onchange={(c) => char("Text Colour", { color: c })} />
  </div>
  <span class="grow"></span>
  <label class="opt"><input type="checkbox" checked={st.snap} onchange={(e) => (st.snap = e.currentTarget.checked)} />Snap to centre</label>
  <label class="opt"><input type="checkbox" checked={st.rulers} onchange={(e) => (st.rulers = e.currentTarget.checked)} />Rulers</label>
  <span class="size">{size.width} × {size.height}</span>
</div>

<style>
  .fbar {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--line);
  }
  .font {
    width: 170px;
    height: 30px;
    padding: 0 8px;
    background: var(--surface-raised);
    color: var(--text);
    border: 1px solid var(--line);
    border-radius: var(--r);
    font: inherit;
    font-size: 12.5px;
  }
  .font:disabled {
    opacity: 0.45;
  }
  .grp {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .grp.sw {
    gap: 6px;
  }
  .tb {
    min-width: 30px;
    height: 30px;
    padding: 0 6px;
    display: grid;
    place-items: center;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    font-size: 13px;
  }
  .tb:hover:not(:disabled) {
    background: var(--surface-overlay);
  }
  .tb.on {
    background: var(--accent-sel);
    border-color: var(--accent-ring);
  }
  .tb:disabled,
  .dot:disabled {
    opacity: 0.45;
  }
  .b {
    font-weight: 700;
  }
  .i {
    font-style: italic;
    font-family: Georgia, serif;
  }
  .u {
    text-decoration: underline;
  }
  .s {
    text-decoration: line-through;
  }
  .dot {
    width: 22px;
    height: 22px;
    padding: 0;
    border-radius: 50%;
    border: 2px solid var(--line-strong);
  }
  .dot.on {
    border-color: var(--accent-ring);
  }
  .sep {
    width: 1px;
    height: 22px;
    background: var(--line);
  }
  .grow {
    flex: 1 1 10px;
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-label);
    font-size: 12.5px;
  }
  .opt input {
    accent-color: var(--accent);
    margin: 0;
  }
  .size {
    color: var(--text-meta);
    font-size: 12px;
  }
</style>
