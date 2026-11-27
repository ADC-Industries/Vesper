<script lang="ts">
  import Button from "../../ui/Button.svelte";
  // Inspector, Shape tab (an object is selected; under Arrange for several): geometry, opacity,
  // fill, stroke, shadow, feather. Decoration changes apply to every selected object.
  import Link2 from "lucide-svelte/icons/link-2";
  import Unlink2 from "lucide-svelte/icons/unlink-2";
  import type { Element, ElementPatch, Fill, FillKind, MediaBinItemInfo, ScaleMode, Shadow, Stroke, StrokeStyle, Feather } from "$protocol";
  import type { Connection } from "../../server";
  import type { EditState } from "../../edit/editState.svelte";
  import { boundsOf, elementName, fillDrawn, fillKindPatch, mediaFillWith } from "../../edit/elements";
  import { DEFAULT_FEATHER, DEFAULT_SHADOW, DEFAULT_STROKE, geometryPatch, setElements } from "../../edit/ops";
  import ColorIn from "./ColorIn.svelte";
  import Num from "./Num.svelte";
  import MediaPicker from "./MediaPicker.svelte";
  import { sourcesState } from "../../state/sources.svelte";
  import { inputSource, liveFillChoices, liveFillValue } from "../show/sources";

  let { st, conn }: { st: EditState; conn: Connection } = $props();
  type MediaFill = Extract<FillKind, { type: "media" }>;
  type LiveFill = Extract<FillKind, { type: "liveSource" }>;

  let sel = $derived(st.selected);
  let el = $derived<Element | null>(sel[0] ?? null);
  let one = $derived(sel.length === 1 ? el : null);
  let picking = $state(false);
  /** Geometry shown: the drag preview while dragging, else the document's. */
  let geom = $derived(one ? (st.preview?.get(one.id) ?? boundsOf(one)) : null);

  function patchAll(label: string, patch: (e: Element) => ElementPatch) {
    const slide = st.slideId;
    if (!slide) return;
    void st.run(label, setElements(slide, sel.map((e) => ({ id: e.id, patch: patch(e) }))));
  }

  function geo(field: "x" | "y" | "width" | "height" | "rotation", v: number) {
    if (!one) return;
    const slide = st.slideId!;
    void st.run(field === "rotation" ? "Rotate" : "Geometry", setElements(slide, [{ id: one.id, patch: geometryPatch(one, field, v, !!one.aspectLocked) }]));
  }

  // Fill
  let fill = $derived<Fill>(el?.fill ?? {});
  let fillOn = $derived(fillDrawn(fill) && !!fill.kind && fill.kind.type !== "unknown");
  let fillKind = $derived(fill.kind?.type === "media" ? "media" : fill.kind?.type === "liveSource" ? "live" : "color");
  /** A live network source as the fill (a camera box): the first video input (portable: each
   *  computer binds it), else the first listed source, to start with. */
  function liveFill() {
    sourcesState.ensure();
    const v = sourcesState.inputs[0];
    const kind: LiveFill = v
      ? { type: "liveSource", input: v.id, source: inputSource(v) ?? "", display: { scale: "fill" } }
      : { type: "liveSource", source: sourcesState.list[0]?.name ?? "", display: { scale: "fill" } };
    setFill({ enabled: true, kind }, "Live Source Fill");
  }
  /** The fill picker's choice: `input:<id>` (a video input, its bound source kept as the
   *  fallback) or a network source name. */
  function pickLive(value: string) {
    if (value.startsWith("input:")) {
      const id = value.slice(6);
      const v = sourcesState.input(id);
      setLive("Live Source", (l) => ({ ...l, input: id, source: (v && inputSource(v)) ?? l.source ?? "" }));
    } else {
      setLive("Live Source", (l) => {
        const { input: _input, ...rest } = l;
        return { ...rest, source: value };
      });
    }
  }
  $effect(() => {
    if (fill.kind?.type === "liveSource") sourcesState.ensure();
  });
  function setFill(f: Fill, label = "Fill") {
    patchAll(label, () => ({ fill: f }));
  }
  /** One setting of each selected object's own media fill (E10-3). */
  function setMedia(label: string, change: (k: MediaFill) => MediaFill) {
    patchAll(label, (e) => fillKindPatch(e, "media", change));
  }
  /** One setting of each selected object's own live-source fill (E10-3). */
  function setLive(label: string, change: (k: LiveFill) => LiveFill) {
    patchAll(label, (e) => fillKindPatch(e, "liveSource", change));
  }
  /** Fill on or off, each selected object keeping its own fill (white when it had none). */
  function fillSwitched(f: Fill | undefined, on: boolean): Fill {
    if (!on) return { ...(f ?? {}), enabled: false };
    return { enabled: true, kind: f?.kind && f.kind.type !== "unknown" ? f.kind : { type: "color", color: { r: 1, g: 1, b: 1 } } };
  }
  function pickImage(it: MediaBinItemInfo) {
    picking = false;
    patchAll(it.kind === "video" ? "Video Fill" : "Image Fill", (e) => ({ fill: mediaFillWith(e.fill, it.asset, it.kind) }));
  }

  let stroke = $derived<Stroke>({ ...DEFAULT_STROKE, ...(el?.stroke ?? {}) });
  function setStroke(s: Partial<Stroke>) {
    patchAll("Stroke", (e) => ({ stroke: { ...DEFAULT_STROKE, ...(e.stroke ?? {}), ...s } }));
  }
  const STROKE_STYLES: { v: StrokeStyle; label: string }[] = [
    { v: "solid", label: "Solid" },
    { v: "shortDash", label: "Dashed" },
    { v: "squareDash", label: "Dotted" },
    { v: "double", label: "Double" },
  ];
  /** Shapes the server draws a double stroke on. */
  function doubleDrawn(e: Element) {
    const t = e.kind?.type === "shape" || e.kind?.type === "text" ? e.kind.shape?.type : undefined;
    return !t || t === "rectangle" || t === "roundedRectangle" || t === "ellipse";
  }
  let shadow = $derived<Shadow>({ ...DEFAULT_SHADOW, ...(el?.shadow ?? {}) });
  function setShadow(s: Partial<Shadow>) {
    patchAll("Shadow", (e) => ({ shadow: { ...DEFAULT_SHADOW, ...(e.shadow ?? {}), ...s } }));
  }
  let feather = $derived<Feather>({ ...DEFAULT_FEATHER, ...(el?.feather ?? {}) });
  function setFeather(f: Partial<Feather>) {
    patchAll("Feather", (e) => ({ feather: { ...DEFAULT_FEATHER, ...(e.feather ?? {}), ...f } }));
  }
</script>

{#if el}
  {#if one}
    <section class="sec">
      <h4 class="ename">{elementName(one)}</h4>
      <div class="geo">
        <span>X</span><Num label="X" value={geom?.x} onchange={(v) => geo("x", v)} />
        <span>Y</span><Num label="Y" value={geom?.y} onchange={(v) => geo("y", v)} />
        <span>W</span><Num label="Width" value={geom?.width} min={1} onchange={(v) => geo("width", v)} />
        <span>H</span><Num label="Height" value={geom?.height} min={1} onchange={(v) => geo("height", v)} />
        <button
          class="lockar"
          class:on={!!one.aspectLocked}
          onclick={() => patchAll(one.aspectLocked ? "Unlock Aspect" : "Lock Aspect", () => ({ aspectLocked: !one.aspectLocked }))}
          title={one.aspectLocked ? "Aspect ratio locked" : "Lock aspect ratio"}
          aria-label="Lock aspect ratio"
          aria-pressed={!!one.aspectLocked}>{#if one.aspectLocked}<Link2 size={15} />{:else}<Unlink2 size={15} />{/if}</button
        >
      </div>
      <div class="line"><span class="lab">Rotation</span><Num label="Rotation" value={one.rotation ?? 0} unit="°" onchange={(v) => geo("rotation", v)} /></div>
    </section>
  {/if}

  <section class="sec">
    <div class="line">
      <span class="lab">Opacity</span>
      <input type="range" min="0" max="100" value={Math.round((el.opacity ?? 1) * 100)} onchange={(e) => patchAll("Opacity", () => ({ opacity: Number(e.currentTarget.value) / 100 }))} aria-label="Opacity" data-field="opacity-range" />
      <Num label="Opacity" value={Math.round((el.opacity ?? 1) * 100)} min={0} max={100} digits={0} unit="%" width={48} onchange={(v) => patchAll("Opacity", () => ({ opacity: v / 100 }))} />
    </div>
  </section>

  <section class="sec">
    <label class="check"><input type="checkbox" checked={fillOn} onchange={(e) => { const on = e.currentTarget.checked; patchAll("Fill", (x) => ({ fill: fillSwitched(x.fill, on) })); }} data-field="fill-enabled" /><h4>Fill</h4></label>
    {#if fillOn}
      <div class="line">
        <select value={fillKind} onchange={(e) => (e.currentTarget.value === "media" ? (picking = true) : e.currentTarget.value === "live" ? liveFill() : setFill({ enabled: true, kind: { type: "color", color: { r: 1, g: 1, b: 1 } } }))} aria-label="Fill type" data-field="fill-kind">
          <option value="color">Colour</option>
          <option value="media">Image or video</option>
          <option value="live">Live source</option>
        </select>
        {#if fill.kind?.type === "color"}
          <ColorIn color={fill.kind.color} label="Fill colour" onchange={(c) => setFill({ enabled: true, kind: { type: "color", color: c } })} />
        {:else if fill.kind?.type === "media"}
          {@const k = fill.kind}
          <div class="pop-anchor">
            <Button size="sm" onclick={() => (picking = !picking)}>Choose…</Button>
          </div>
          <select value={k.display?.scale ?? "fit"} onchange={(e) => { const scale = e.currentTarget.value as ScaleMode; setMedia("Fill", (m) => ({ ...m, display: { ...(m.display ?? {}), scale } })); }} aria-label="Image scaling">
            <option value="fit">Fit</option>
            <option value="fill">Fill</option>
            <option value="stretch">Stretch</option>
          </select>
        {:else if fill.kind?.type === "liveSource"}
          {@const k = fill.kind}
          {@const choices = liveFillChoices(sourcesState.inputs, sourcesState.list, k)}
          <select value={liveFillValue(k)} onchange={(e) => pickLive(e.currentTarget.value)} aria-label="Live source" data-field="live-source">
            {#if !k.source && !k.input}<option value="">{sourcesState.loaded ? "Choose an input or a source" : "Looking for sources…"}</option>{/if}
            {#if choices.some((c) => c.group === "inputs")}
              <optgroup label="Video inputs">
                {#each choices.filter((c) => c.group === "inputs") as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
              </optgroup>
            {/if}
            <optgroup label="Sources on the network">
              {#each choices.filter((c) => c.group === "sources") as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
            </optgroup>
          </select>
          <select value={k.display?.scale ?? "fit"} onchange={(e) => { const scale = e.currentTarget.value as ScaleMode; setLive("Fill", (l) => ({ ...l, display: { ...(l.display ?? {}), scale } })); }} aria-label="Live source scaling">
            <option value="fit">Fit</option>
            <option value="fill">Fill</option>
            <option value="stretch">Stretch</option>
          </select>
        {:else}
          <span class="note">{fill.kind?.type} (kept from the import)</span>
        {/if}
      </div>
      {#if fill.kind?.type === "media"}
        {@const k = fill.kind}
        <div class="line" data-video-playback>
          <span class="lab">Video</span>
          <label class="check"><input type="checkbox" checked={k.playback?.repeat?.type === "loop"} onchange={(e) => { const loop = e.currentTarget.checked; setMedia("Video Loop", (m) => ({ ...m, playback: { ...(m.playback ?? {}), repeat: { type: loop ? "loop" : "once" } } })); }} data-field="video-loop" />Loop</label>
          <span class="lab">Start at</span>
          <Num label="Video start offset" value={k.playback?.inPoint ?? 0} min={0} max={36000} step={0.5} unit="s" onchange={(v) => setMedia("Video Start", (m) => ({ ...m, playback: { ...(m.playback ?? {}), inPoint: v || undefined } }))} />
        </div>
        <div class="line" data-video-sound>
          <span class="lab">Sound</span>
          <label class="check"><input type="checkbox" checked={!!k.audio?.muted} onchange={(e) => { const muted = e.currentTarget.checked || undefined; setMedia("Video Mute", (m) => ({ ...m, audio: { ...(m.audio ?? {}), muted } })); }} data-field="video-mute" />Mute</label>
          <span class="lab">Level</span>
          <Num label="Video sound level" value={k.audio?.volumeDb ?? 0} min={-60} max={12} step={1} unit="dB" onchange={(v) => setMedia("Video Level", (m) => ({ ...m, audio: { ...(m.audio ?? {}), volumeDb: v || undefined } }))} />
        </div>
      {/if}
      {#if picking}<div class="pop-anchor"><MediaPicker {conn} align="right" kind="media" onpick={pickImage} onclose={() => (picking = false)} /></div>{/if}
      {#if fill.kind?.type === "liveSource"}<p class="note" data-live-fill-note>{fill.kind.input ? "The video input shows the source each computer binds it to. " : ""}The source shows in the box on the screens; this canvas leaves the box empty.</p>{/if}
      {#if fill.kind?.type === "media"}<p class="note">A video plays in the box on the screens from when the slide goes live, with its sound on the slide layer's level (muted on props, messages and announcements); this canvas shows it still.</p>{/if}
    {/if}
  </section>

  <section class="sec">
    <label class="check"><input type="checkbox" checked={!!stroke.enabled} onchange={(e) => setStroke({ enabled: e.currentTarget.checked })} data-field="stroke-enabled" /><h4>Stroke</h4></label>
    {#if stroke.enabled}
      <div class="line">
        <span class="lab">Width</span><Num label="Stroke width" value={stroke.width} min={0} max={500} onchange={(v) => setStroke({ width: v })} />
        <ColorIn color={stroke.color} label="Stroke colour" onchange={(c) => setStroke({ color: c })} />
      </div>
      <div class="line">
        <span class="lab">Style</span>
        <select value={stroke.style ?? "solid"} onchange={(e) => setStroke({ style: e.currentTarget.value as StrokeStyle })} aria-label="Stroke style" data-field="stroke-style">
          {#each STROKE_STYLES as o (o.v)}<option value={o.v}>{o.label}</option>{/each}
          {#if !STROKE_STYLES.some((o) => o.v === (stroke.style ?? "solid"))}<option value={stroke.style}>Imported dashes</option>{/if}
        </select>
      </div>
      {#if stroke.style === "double" && sel.some((e) => !doubleDrawn(e))}<p class="note">Double draws on rectangles and ellipses; other shapes show it solid.</p>{/if}
    {/if}
  </section>

  <section class="sec">
    <label class="check"><input type="checkbox" checked={!!shadow.enabled} onchange={(e) => setShadow({ enabled: e.currentTarget.checked })} data-field="shadow-enabled" /><h4>Shadow</h4></label>
    {#if shadow.enabled}
      <div class="grid2">
        <span class="lab">Angle</span><Num label="Shadow angle" value={shadow.angle} unit="°" onchange={(v) => setShadow({ angle: ((v % 360) + 360) % 360 })} />
        <span class="lab">Offset</span><Num label="Shadow offset" value={shadow.offset} min={0} onchange={(v) => setShadow({ offset: v })} />
        <span class="lab">Radius</span><Num label="Shadow radius" value={shadow.radius} min={0} onchange={(v) => setShadow({ radius: v })} />
        <span class="lab">Opacity</span><Num label="Shadow opacity" value={Math.round((shadow.opacity ?? 0.75) * 100)} min={0} max={100} digits={0} unit="%" onchange={(v) => setShadow({ opacity: v / 100 })} />
        <span class="lab">Colour</span><ColorIn color={shadow.color} label="Shadow colour" onchange={(c) => setShadow({ color: c })} />
      </div>
    {/if}
  </section>

  <section class="sec">
    <label class="check"><input type="checkbox" checked={!!feather.enabled} onchange={(e) => setFeather({ enabled: e.currentTarget.checked })} data-field="feather-enabled" /><h4>Feather</h4></label>
    {#if feather.enabled}
      <div class="line">
        <span class="lab">Radius</span><Num label="Feather radius" value={Math.round((feather.radius ?? 0.05) * 100)} min={0} max={100} digits={0} unit="%" onchange={(v) => setFeather({ radius: v / 100 })} />
        <select value={feather.style ?? "inside"} onchange={(e) => setFeather({ style: e.currentTarget.value as Feather["style"] })} aria-label="Feather style">
          <option value="inside">Inside</option>
          <option value="center">Centre</option>
          <option value="outside">Outside</option>
        </select>
      </div>
    {/if}
  </section>
{/if}
