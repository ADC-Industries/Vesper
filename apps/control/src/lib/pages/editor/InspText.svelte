<script lang="ts">
  // Inspector, Text tab (a text object is selected). Character and paragraph styles apply to
  // the characters selected in the in-place editor, else to the whole text box; box settings
  // (fit, margins, outline, text shadow, line mask, vertical alignment) to the box.
  import AlignLeft from "lucide-svelte/icons/align-left";
  import { shown, type Sent } from "../../edit/optimistic";
  import AlignCenter from "lucide-svelte/icons/align-center";
  import AlignRight from "lucide-svelte/icons/align-right";
  import AlignJustify from "lucide-svelte/icons/align-justify";
  import Italic from "lucide-svelte/icons/italic";
  import type { CharStyle, Element, FitMode, LineMask, LineMaskStyle, ParagraphStyle, ScrollDirection, ScrollStart, Shadow, TextAlign, TextBox, TextOutline, TextScroll, VerticalAlign, Capitalization } from "$protocol";
  import type { EditState } from "../../edit/editState.svelte";
  import { DEFAULT_OUTLINE, DEFAULT_SCROLL, DEFAULT_SHADOW, setElements, textBoxOp, weightName } from "../../edit/ops";
  import { cpLength, effectiveParagraph, paragraphAt, plainText, rangeText, styleAt } from "../../edit/text";
  import ColorIn from "./ColorIn.svelte";
  import Num from "./Num.svelte";

  let { st, el }: { st: EditState; el: Element } = $props();

  let tb = $derived<TextBox>(el.kind?.type === "text" ? (el.kind.text ?? {}) : {});
  let range = $derived(st.editingText === el.id ? st.textRange : null);
  let cur = $derived<CharStyle>(styleAt(tb, range?.start ?? 0));
  let para = $derived<ParagraphStyle>(effectiveParagraph(tb, paragraphAt(tb, range?.start ?? 0)));
  let scope = $derived(range ? `Selected text: “${short(rangeText(plainText(tb), range))}”` : "Whole text box");

  function short(s: string) {
    return s.length > 24 ? `${s.slice(0, 22)}…` : s;
  }

  let families = $derived(st.fonts?.families ?? []);
  let family = $derived(families.find((f) => f.family.toLowerCase() === (cur.fontFamily ?? "").toLowerCase()) ?? null);
  let weights = $derived([...new Set((family?.faces ?? []).map((f) => f.weight))].sort((a, b) => a - b));
  let substitute = $derived(
    !family && cur.fontFamily ? (st.fonts?.substitutions.find((s) => s.family?.toLowerCase() === cur.fontFamily!.toLowerCase())?.replacementFamily ?? st.fonts?.fallbackFamily ?? null) : null,
  );

  function char(label: string, character: CharStyle) {
    void st.textStyle(el, label, { character });
  }
  function paragraph(label: string, p: ParagraphStyle) {
    void st.textStyle(el, label, { paragraph: p });
  }
  function box(label: string, fields: Parameters<typeof textBoxOp>[2]) {
    void st.editText(el, label, () => [textBoxOp(st.slideId!, el.id, fields)]);
  }

  function setFamily(name: string) {
    const f = families.find((x) => x.family === name);
    // Keep the weight when the family has it, else its nearest face.
    const want = cur.fontWeight ?? 400;
    const ws = (f?.faces ?? []).map((x) => x.weight);
    const w = ws.length ? ws.reduce((a, b) => (Math.abs(b - want) < Math.abs(a - want) ? b : a)) : want;
    char("Font", { fontFamily: name, fontWeight: w });
  }

  // Scrolling text (an element setting, not the text box's).
  // The scrolling just sent, until that save settled: the next change builds on it (D10-2, E10-5).
  let sentScroll = $state.raw<Sent<TextScroll> | null>(null);
  let scroll = $derived<TextScroll>(shown(sentScroll, `${st.slideId}/${el.id}`, { ...DEFAULT_SCROLL, ...(el.scroll ?? {}) }));
  function setScroll(patch: Partial<TextScroll>) {
    const slide = st.slideId;
    if (!slide) return;
    const next = { ...scroll, ...patch };
    const mine = { key: `${slide}/${el.id}`, value: next };
    sentScroll = mine;
    void st.run("Scrolling", setElements(slide, [{ id: el.id, patch: { scroll: next } }])).finally(() => {
      if (sentScroll === mine) sentScroll = null;
    });
  }
  const SCROLL_DIRS: { v: ScrollDirection; label: string; glyph: string }[] = [
    { v: "left", label: "Scroll left", glyph: "M19 12H5M11 6l-6 6 6 6" },
    { v: "right", label: "Scroll right", glyph: "M5 12h14M13 6l6 6-6 6" },
    { v: "up", label: "Scroll up", glyph: "M12 19V5M6 11l6-6 6 6" },
    { v: "down", label: "Scroll down", glyph: "M12 5v14M6 13l6 6 6-6" },
  ];

  let outline = $derived<TextOutline>({ ...DEFAULT_OUTLINE, ...(tb.outline ?? {}) });
  let tshadow = $derived<Shadow>({ ...DEFAULT_SHADOW, ...(tb.textShadow ?? {}) });
  let mask = $derived<LineMask>(tb.lineMask ?? {});
  let margins = $derived(tb.margins ?? {});
  const ALIGNS: { v: TextAlign; icon: typeof AlignLeft; label: string }[] = [
    { v: "left", icon: AlignLeft, label: "Align left" },
    { v: "center", icon: AlignCenter, label: "Centre" },
    { v: "right", icon: AlignRight, label: "Align right" },
    { v: "justified", icon: AlignJustify, label: "Justify" },
  ];
</script>

<p class="scope" data-text-scope>{scope}</p>

<section class="sec">
  <h4>Font</h4>
  <select class="wide" value={family?.family ?? "__missing"} onchange={(e) => setFamily(e.currentTarget.value)} aria-label="Font family" data-field="font-family">
    {#if !family}<option value="__missing" disabled>{cur.fontFamily ?? "?"} (not installed{substitute ? `: renders as ${substitute}` : ""})</option>{/if}
    {#each families as f (f.family)}<option value={f.family}>{f.family}</option>{/each}
  </select>
  <div class="line">
    <select value={cur.fontWeight ?? 400} onchange={(e) => char("Font Weight", { fontWeight: Number(e.currentTarget.value) })} aria-label="Font weight" data-field="font-weight">
      {#each weights.length ? weights : [cur.fontWeight ?? 400] as w (w)}<option value={w}>{weightName(w)}</option>{/each}
    </select>
    <button class="tg" class:on={!!cur.italic} onclick={() => char("Italic", { italic: !cur.italic })} aria-pressed={!!cur.italic} title="Italic" aria-label="Italic" data-field="italic"><Italic size={15} /></button>
    <Num label="Font size" value={cur.size ?? 72} min={1} max={2000} width={56} onchange={(v) => char("Font Size", { size: v })} />
    <ColorIn color={cur.color} label="Text colour" onchange={(c) => char("Text Colour", { color: c })} />
  </div>
  <div class="line">
    <span class="lab">Caps</span>
    <select value={cur.capitalization === "allCaps" ? "allCaps" : cur.capitalization === "titleCase" || cur.capitalization === "startCase" ? "titleCase" : "none"} onchange={(e) => char("Capitalization", { capitalization: e.currentTarget.value as Capitalization })} aria-label="Capitalization" data-field="caps">
      <option value="none">None</option>
      <option value="allCaps">ALL CAPS</option>
      <option value="titleCase">Title</option>
    </select>
  </div>
</section>

<section class="sec">
  <h4>Paragraph</h4>
  <div class="line">
    <div class="seg" role="radiogroup" aria-label="Alignment">
      {#each ALIGNS as a (a.v)}
        {@const Icon = a.icon}
        <button class:on={(para.alignment ?? "center") === a.v || (a.v === "left" && para.alignment === "natural")} onclick={() => paragraph("Alignment", { alignment: a.v })} title={a.label} aria-label={a.label}><Icon size={15} /></button>
      {/each}
    </div>
    <span class="lab">Line</span>
    <Num label="Line height" value={para.lineHeightMultiple ?? 1} min={0.3} max={5} step={0.05} width={50} unit="×" onchange={(v) => paragraph("Line Height", { lineHeightMultiple: v })} />
  </div>
</section>

<section class="sec">
  <h4>Box</h4>
  <div class="line">
    <span class="lab">Vertical</span>
    <select value={tb.verticalAlign ?? "middle"} onchange={(e) => box("Vertical Alignment", { verticalAlign: e.currentTarget.value as VerticalAlign })} aria-label="Vertical alignment" data-field="valign">
      <option value="top">Top</option>
      <option value="middle">Middle</option>
      <option value="bottom">Bottom</option>
    </select>
  </div>
  <div class="line">
    <span class="lab">Fit</span>
    <select value={tb.fit ?? "none"} onchange={(e) => box("Fit", { fit: e.currentTarget.value as FitMode })} aria-label="Fit mode" data-field="fit">
      <option value="none">None</option>
      <option value="shrinkToFit">Shrink to fit</option>
      <option value="scaleToFit">Scale to fit</option>
      {#if tb.fit && !["none", "shrinkToFit", "scaleToFit"].includes(tb.fit)}<option value={tb.fit} disabled>{tb.fit} (imported)</option>{/if}
    </select>
  </div>
  <div class="grid4">
    <span class="lab">Margins</span>
    <Num label="Left margin" value={margins.left ?? 0} min={0} width={44} digits={0} onchange={(v) => box("Margins", { margins: { ...margins, left: v } })} />
    <Num label="Right margin" value={margins.right ?? 0} min={0} width={44} digits={0} onchange={(v) => box("Margins", { margins: { ...margins, right: v } })} />
    <Num label="Top margin" value={margins.top ?? 0} min={0} width={44} digits={0} onchange={(v) => box("Margins", { margins: { ...margins, top: v } })} />
    <Num label="Bottom margin" value={margins.bottom ?? 0} min={0} width={44} digits={0} onchange={(v) => box("Margins", { margins: { ...margins, bottom: v } })} />
  </div>
</section>

<section class="sec">
  <label class="check"><input type="checkbox" checked={!!outline.enabled} onchange={(e) => box("Outline", { outline: { ...outline, enabled: e.currentTarget.checked } })} data-field="outline-enabled" /><h4>Outline</h4></label>
  {#if outline.enabled}
    <div class="line">
      <span class="lab">Width</span><Num label="Outline width" value={outline.width} min={0} max={100} onchange={(v) => box("Outline", { outline: { ...outline, width: v } })} />
      <ColorIn color={outline.color} label="Outline colour" onchange={(c) => box("Outline", { outline: { ...outline, color: c } })} />
    </div>
  {/if}
</section>

<section class="sec">
  <label class="check"><input type="checkbox" checked={!!tshadow.enabled} onchange={(e) => box("Text Shadow", { textShadow: { ...tshadow, enabled: e.currentTarget.checked } })} data-field="tshadow-enabled" /><h4>Text Shadow</h4></label>
  {#if tshadow.enabled}
    <div class="grid2">
      <span class="lab">Angle</span><Num label="Text shadow angle" value={tshadow.angle} unit="°" onchange={(v) => box("Text Shadow", { textShadow: { ...tshadow, angle: ((v % 360) + 360) % 360 } })} />
      <span class="lab">Offset</span><Num label="Text shadow offset" value={tshadow.offset} min={0} onchange={(v) => box("Text Shadow", { textShadow: { ...tshadow, offset: v } })} />
      <span class="lab">Radius</span><Num label="Text shadow radius" value={tshadow.radius} min={0} onchange={(v) => box("Text Shadow", { textShadow: { ...tshadow, radius: v } })} />
      <span class="lab">Opacity</span><Num label="Text shadow opacity" value={Math.round((tshadow.opacity ?? 0.75) * 100)} min={0} max={100} digits={0} unit="%" onchange={(v) => box("Text Shadow", { textShadow: { ...tshadow, opacity: v / 100 } })} />
      <span class="lab">Colour</span><ColorIn color={tshadow.color} label="Text shadow colour" onchange={(c) => box("Text Shadow", { textShadow: { ...tshadow, color: c } })} />
    </div>
  {/if}
</section>

<section class="sec">
  <label class="check"><input type="checkbox" checked={!!mask.enabled} onchange={(e) => box("Line Mask", { lineMask: { fill: { r: 0, g: 0, b: 0, a: 0.6 }, ...mask, enabled: e.currentTarget.checked } })} data-field="linemask-enabled" /><h4>Line Mask</h4></label>
  {#if mask.enabled}
    <div class="line">
      <select value={mask.style ?? "fullWidth"} onchange={(e) => box("Line Mask", { lineMask: { ...mask, style: e.currentTarget.value as LineMaskStyle } })} aria-label="Line mask style">
        <option value="fullWidth">Full width</option>
        <option value="lineWidth">Line width</option>
        <option value="maxLineWidth">Widest line</option>
      </select>
      <ColorIn color={mask.fill ?? { r: 0, g: 0, b: 0 }} label="Line mask colour" onchange={(c) => box("Line Mask", { lineMask: { ...mask, fill: { ...c, a: mask.fill?.a ?? c.a } } })} />
    </div>
  {/if}
</section>

<section class="sec" data-scroll-section>
  <label class="check"><input type="checkbox" checked={!!scroll.enabled} onchange={(e) => setScroll({ enabled: e.currentTarget.checked })} data-field="scroll-enabled" /><h4>Scrolling</h4></label>
  {#if scroll.enabled}
    <div class="line">
      <span class="lab">Direction</span>
      <div class="seg" role="radiogroup" aria-label="Scroll direction">
        {#each SCROLL_DIRS as d (d.v)}
          <button class:on={scroll.direction === d.v} role="radio" aria-checked={scroll.direction === d.v} title={d.label} aria-label={d.label} onclick={() => setScroll({ direction: d.v })} data-scroll-dir={d.v}>
            <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true"><path d={d.glyph} fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>
          </button>
        {/each}
      </div>
    </div>
    <div class="line">
      <span class="lab">Start</span>
      <select value={scroll.start} onchange={(e) => setScroll({ start: e.currentTarget.value as ScrollStart })} aria-label="Scroll start position" data-field="scroll-start">
        <option value="auto">Automatic</option>
        <option value="offscreen">Start offscreen</option>
        <option value="onscreen">Start onscreen</option>
      </select>
    </div>
    <div class="line"><span class="lab">Speed</span><Num label="Scroll speed" value={scroll.speed} min={5} max={2000} step={10} digits={0} unit="px/s" onchange={(v) => setScroll({ speed: v })} /></div>
    <div class="line">
      <label class="check"><input type="checkbox" checked={scroll.repeat !== false} onchange={(e) => setScroll({ repeat: e.currentTarget.checked })} data-field="scroll-repeat" />Repeat</label>
      {#if scroll.repeat !== false}<span class="lab">Gap</span><Num label="Scroll gap" value={scroll.gap} min={0} max={4000} step={10} digits={0} unit="px" onchange={(v) => setScroll({ gap: v })} />{/if}
    </div>
    <p class="note">Moves on the screens from when the slide goes live; thumbnails and this canvas show it still.</p>
  {/if}
</section>

<p class="hint">{cpLength(plainText(tb))} characters. Double-click the text box (or press Enter) to edit its text; select characters to style only those.</p>
