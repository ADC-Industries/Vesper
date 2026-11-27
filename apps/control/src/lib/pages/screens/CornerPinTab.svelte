<script lang="ts">
  // The Corner Pin tab of a screen's output card (plan 4.8): Enable, a preview of the screen
  // whose four corners are dragged, ΔX / ΔY per corner in screen pixels (positive = away
  // from the centre, as the board says) and Reset. Changes apply live
  // (PUT /screens/{id}/corner-pin, throttled while dragging) to the whole screen.
  import type { CornerPin, ScreenEntry } from "$protocol";
  import { Button } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { CORNERS, cornerPoint, fieldNumber, isConvex, offsetFor, pinOf, quadOf, throttle, type Corner } from "./adjust";

  let { entry, onapplied }: { entry: ScreenEntry; onapplied: (e: ScreenEntry) => void } = $props();

  let w = $derived(entry.size?.width ?? 1920);
  let h = $derived(entry.size?.height ?? 1080);
  let pin = $state<CornerPin>(pinOf(null));
  let error = $state<string | null>(null);
  let dragging = $state<Corner | null>(null);
  /** Revisions our own changes produced: the server copy is taken over only when someone
   * else changed it (our replies arrive while later edits are still on their way). */
  const mine = new Set<string>();
  $effect(() => {
    const p = pinOf(entry);
    if (!mine.has(entry.revision) && !dragging) pin = p;
  });

  let quad = $derived(quadOf(pin, w, h));
  let convex = $derived(isConvex(quad));
  let offsets = $derived(CORNERS.some((c) => pin[c.key].x !== 0 || pin[c.key].y !== 0));
  // The preview shows a margin around the screen so corners can be pulled outwards.
  let m = $derived(Math.round(Math.max(w, h) * 0.12));
  let handle = $derived(Math.max(w, h) / 60);

  const send = throttle(async (id: string, p: CornerPin) => {
    const rest = session.conn?.rest;
    if (!rest) return;
    try {
      const r = await rest.setScreenCornerPin(id, p.enabled || offsets ? p : {});
      error = null;
      mine.add(r.revision);
      onapplied(r);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }, 120);

  function update(p: CornerPin) {
    pin = p;
    // A crossed or folded shape is shown but not sent (the server would refuse it).
    if (!p.enabled || isConvex(quadOf(p, w, h))) send(entry.id, p);
  }

  function setOffset(c: Corner, axis: "x" | "y", v: number) {
    if (!Number.isFinite(v)) return;
    const lim = axis === "x" ? w : h;
    const val = Math.max(-lim, Math.min(lim, Math.round(v)));
    update({ ...pin, [c]: { ...pin[c], [axis]: val } });
  }

  /** A ΔX / ΔY field: an emptied or invalid field changes nothing and shows the value again. */
  function typedOffset(c: Corner, axis: "x" | "y", input: HTMLInputElement) {
    const v = fieldNumber(input.value);
    if (Number.isFinite(v)) setOffset(c, axis, v);
    else input.value = String(pin[c][axis]);
  }

  function reset() {
    update({ ...pin, topLeft: { x: 0, y: 0 }, topRight: { x: 0, y: 0 }, bottomRight: { x: 0, y: 0 }, bottomLeft: { x: 0, y: 0 } });
    send.flush();
  }

  let svgEl = $state<SVGSVGElement | null>(null);
  function toScreen(e: PointerEvent): [number, number] {
    const r = svgEl!.getBoundingClientRect();
    const vw = w + 2 * m;
    const vh = h + 2 * m;
    const s = Math.min(r.width / vw, r.height / vh);
    const ox = (r.width - vw * s) / 2;
    const oy = (r.height - vh * s) / 2;
    return [(e.clientX - r.left - ox) / s - m, (e.clientY - r.top - oy) / s - m];
  }
  function down(c: Corner, e: PointerEvent) {
    if (!pin.enabled) return;
    dragging = c;
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!dragging) return;
    const [x, y] = toScreen(e);
    const cx = Math.max(-w, Math.min(2 * w, x));
    const cy = Math.max(-h, Math.min(2 * h, y));
    update({ ...pin, [dragging]: offsetFor(dragging, cx, cy, w, h) });
  }
  function up() {
    if (!dragging) return;
    dragging = null;
    send.flush();
  }
</script>

<div class="ptab" data-testid="pin-tab">
  <div class="head">
    <label class="chk"><input type="checkbox" checked={pin.enabled} onchange={(e) => update({ ...pin, enabled: e.currentTarget.checked })} data-testid="pin-enabled" /> Enable</label>
    <span class="note">Positive values move away from the center of the screen. Applies to every output of “{entry.name}”.</span>
  </div>
  <div class="body">
    <svg
      bind:this={svgEl}
      class="preview"
      class:off={!pin.enabled}
      viewBox={`${-m} ${-m} ${w + 2 * m} ${h + 2 * m}`}
      preserveAspectRatio="xMidYMid meet"
      role="img"
      aria-label="Corner pin preview: drag the corners"
      onpointermove={move}
      onpointerup={up}
      onpointercancel={up}
      data-testid="pin-preview"
    >
      <rect x={0} y={0} width={w} height={h} class="frame" />
      <polygon points={quad.map((p) => p.join(",")).join(" ")} class="quad" class:bad={!convex} />
      <line x1={quad[0][0]} y1={quad[0][1]} x2={quad[2][0]} y2={quad[2][1]} class="diag" />
      <line x1={quad[1][0]} y1={quad[1][1]} x2={quad[3][0]} y2={quad[3][1]} class="diag" />
      {#each CORNERS as c (c.key)}
        {@const p = cornerPoint(pin, c.key, w, h)}
        <circle cx={p[0]} cy={p[1]} r={handle} class="handle" class:on={dragging === c.key} role="button" tabindex="-1" aria-label={`Drag the ${c.label.toLowerCase()} corner`} onpointerdown={(e) => down(c.key, e)} data-testid={`pin-handle-${c.key}`}>
          <title>{c.label}</title>
        </circle>
      {/each}
    </svg>
    <div class="fields">
      {#each CORNERS as c (c.key)}
        <div class="corner">
          <span class="cname">{c.label}</span>
          <label>ΔX <input class="num" type="number" min={-w} max={w} step="1" value={pin[c.key].x} disabled={!pin.enabled} onchange={(e) => typedOffset(c.key, "x", e.currentTarget)} data-testid={`pin-${c.key}-x`} /></label>
          <label>ΔY <input class="num" type="number" min={-h} max={h} step="1" value={pin[c.key].y} disabled={!pin.enabled} onchange={(e) => typedOffset(c.key, "y", e.currentTarget)} data-testid={`pin-${c.key}-y`} /></label>
        </div>
      {/each}
    </div>
  </div>
  <div class="foot">
    {#if !convex && pin.enabled}<span class="err" role="alert" data-testid="pin-invalid">The corners cross or fold over: this shape is not sent. Move a corner back.</span>
    {:else if error}<span class="err" role="alert">{error}</span>{/if}
    <span class="grow"></span>
    <Button size="sm" onclick={reset} disabled={!offsets} title="All corners back to the screen's corners" data-testid="pin-reset">Reset</Button>
  </div>
</div>

<style>
  .ptab {
    max-width: 880px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px 14px;
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text);
  }
  .chk input {
    accent-color: var(--accent-ring);
  }
  .note {
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .body {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 280px;
    gap: 12px;
    align-items: start;
  }
  .preview {
    width: 100%;
    aspect-ratio: 16 / 10;
    background: var(--work-2);
    border-radius: 6px;
    touch-action: none;
  }
  .frame {
    fill: none;
    stroke: var(--line-hover);
    stroke-width: 1.5;
    stroke-dasharray: 6 5;
    vector-effect: non-scaling-stroke;
  }
  .quad {
    fill: color-mix(in srgb, var(--accent-ring) 22%, transparent);
    stroke: var(--accent-ring);
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
  }
  .quad.bad {
    fill: color-mix(in srgb, var(--danger) 25%, transparent);
    stroke: var(--danger);
  }
  .diag {
    stroke: var(--accent-ring);
    stroke-opacity: 0.4;
    stroke-width: 1;
    vector-effect: non-scaling-stroke;
  }
  .handle {
    fill: var(--text-strong);
    stroke: var(--accent-ring);
    stroke-width: 2;
    vector-effect: non-scaling-stroke;
    cursor: grab;
  }
  .handle.on {
    cursor: grabbing;
    fill: var(--accent-ring);
  }
  .off .quad,
  .off .handle,
  .off .diag {
    opacity: 0.45;
  }
  .off .handle {
    cursor: default;
  }
  .fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px 10px;
  }
  .corner {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .cname {
    color: var(--text);
    font-weight: 600;
  }
  .corner label {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .num {
    width: 100%;
    min-width: 0;
    box-sizing: border-box;
    padding: 4px 6px;
    border-radius: 6px;
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    color: var(--text);
    font: inherit;
    font-variant-numeric: tabular-nums;
  }
  .num:disabled {
    opacity: 0.5;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .grow {
    flex: 1;
  }
  .err {
    color: var(--danger-text);
    font-size: var(--fs-sm);
  }
  @media (max-width: 900px) {
    .body {
      grid-template-columns: 1fr;
    }
  }
</style>
