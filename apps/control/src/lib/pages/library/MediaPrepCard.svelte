<script lang="ts">
  // A video's or sound's preparation in the Library's item panel (docs/MEDIA-PREP-PLAN.md): its
  // state (a running job's step and progress, or what the last preparation found: loudness, the
  // level at playback, the poster frame, why it was converted and from what), Prepare / Prepare
  // again / Use the original, and the trim: in and out points on the clip's timeline (drag the
  // handles or type the times), kept with the file and used by every cue that doesn't set its own.
  import type { CollectionMedia } from "$protocol";
  import { Button } from "../../ui";
  import { mediaPrep } from "../../state/mediaPrep.svelte";
  import { formatTime, parseTime, prepSummary, trimProblem, trimSpan } from "./prep";

  let { media, readOnly }: { media: CollectionMedia; readOnly: boolean } = $props();

  let asset = $derived(media.asset);
  let job = $derived(mediaPrep.jobFor(asset.id));
  let s = $derived(prepSummary(asset, job));
  let duration = $derived(asset.probe?.duration ?? null);
  let running = $derived(!!job && job.step !== "done" && job.step !== "failed");

  // The trim being edited (null: as saved).
  let draft = $state<{ a: number | null; b: number | null } | null>(null);
  $effect(() => {
    void asset.id;
    draft = null;
  });
  let tin = $derived(draft ? draft.a : (asset.prep?.trimIn ?? null));
  let tout = $derived(draft ? draft.b : (asset.prep?.trimOut ?? null));
  let span = $derived(trimSpan(duration ?? 0, tin, tout));
  let problem = $derived(trimProblem(duration, tin, tout));
  let dirty = $derived(!!draft && (draft.a !== (asset.prep?.trimIn ?? null) || draft.b !== (asset.prep?.trimOut ?? null)));

  let bar = $state<HTMLDivElement | null>(null);
  let dragging = $state<"a" | "b" | null>(null);
  function at(e: PointerEvent): number {
    const r = bar!.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    return Math.round(f * (duration ?? 0) * 10) / 10;
  }
  function down(which: "a" | "b", e: PointerEvent) {
    if (readOnly || !duration) return;
    dragging = which;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }
  function move(e: PointerEvent) {
    if (!dragging || !duration) return;
    const t = at(e);
    const cur = { a: tin, b: tout };
    if (dragging === "a") cur.a = Math.min(t, (cur.b ?? duration) - 0.1) || null;
    else {
      const b = Math.max(t, (cur.a ?? 0) + 0.1);
      cur.b = b >= duration ? null : b;
    }
    draft = cur;
  }
  function up() {
    dragging = null;
  }
  function typed(which: "a" | "b", text: string) {
    const v = text.trim() === "" ? null : parseTime(text);
    if (text.trim() !== "" && v == null) return;
    draft = { a: which === "a" ? v : tin, b: which === "b" ? v : tout };
  }
  async function save() {
    if (!draft || problem) return;
    if (await mediaPrep.trim(asset.id, { trimIn: draft.a ?? undefined, trimOut: draft.b ?? undefined })) draft = null;
  }
  async function clear() {
    if (await mediaPrep.trim(asset.id, {})) draft = null;
  }
</script>

<h3 class="cap prep-cap">Preparation</h3>
<div class="prep" data-testid="media-prep">
  <div class="state {s.tone}">
    <span class="dot"></span>
    <span class="label">{s.label}</span>
  </div>
  {#if job?.step === "converting"}
    <div class="progress" role="progressbar" aria-label="Converting" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(job.progress * 100)}>
      <span style:width={`${Math.round(job.progress * 100)}%`}></span>
    </div>
  {/if}
  {#if s.error}<p class="err">{s.error}</p>{/if}
  {#each s.facts as f (f.k)}
    <div class="kv"><span>{f.k}</span><span title={f.v}>{f.v}</span></div>
  {/each}
  {#if s.reasons.length}
    <p class="why">Converted because: {s.reasons.join("; ")}. The original is kept.</p>
  {/if}
  {#each s.notes as n (n)}<p class="note">{n}</p>{/each}
  {#if mediaPrep.elsewhere}<p class="note">Preparation runs on {mediaPrep.elsewhere}.</p>{/if}
  <div class="actions">
    {#if !asset.prep?.analysed && !s.error}
      <Button size="sm" disabled={readOnly || running || mediaPrep.busy} onclick={() => mediaPrep.prepare(asset.id)}>Prepare</Button>
    {:else}
      <Button size="sm" disabled={readOnly || running || mediaPrep.busy} onclick={() => mediaPrep.prepare(asset.id, true)}>Prepare again</Button>
    {/if}
    {#if s.converted}
      <Button size="sm" disabled={readOnly || running || mediaPrep.busy} onclick={() => mediaPrep.useOriginal(asset.id)}>Use the original</Button>
    {/if}
  </div>
</div>

{#if duration}
  <h3 class="cap prep-cap">Trim</h3>
  <div class="trim" data-testid="media-trim">
    <div class="bar" bind:this={bar} onpointermove={move} onpointerup={up} onpointercancel={up} role="group" aria-label="Trim on the clip's timeline">
      <span class="cut" style:left="0" style:width={`${span.a * 100}%`}></span>
      <span class="keep" style:left={`${span.a * 100}%`} style:width={`${(span.b - span.a) * 100}%`}></span>
      <span class="cut" style:left={`${span.b * 100}%`} style:right="0"></span>
      {#if asset.prep?.posterAt != null}<span class="poster" style:left={`${(asset.prep.posterAt / duration) * 100}%`} title="Poster frame"></span>{/if}
      <button type="button" class="handle" style:left={`${span.a * 100}%`} aria-label="In point" disabled={readOnly} onpointerdown={(e) => down("a", e)}></button>
      <button type="button" class="handle" style:left={`${span.b * 100}%`} aria-label="Out point" disabled={readOnly} onpointerdown={(e) => down("b", e)}></button>
    </div>
    <div class="times">
      <label>In <input type="text" value={tin != null ? formatTime(tin) : ""} placeholder="0:00.0" disabled={readOnly} onchange={(e) => typed("a", e.currentTarget.value)} /></label>
      <label>Out <input type="text" value={tout != null ? formatTime(tout) : ""} placeholder={formatTime(duration)} disabled={readOnly} onchange={(e) => typed("b", e.currentTarget.value)} /></label>
      <span class="len">Plays {formatTime((tout ?? duration) - (tin ?? 0))}</span>
    </div>
    {#if problem}<p class="err">{problem}</p>{/if}
    <div class="actions">
      <Button size="sm" variant="primary" disabled={readOnly || !dirty || !!problem || mediaPrep.busy} onclick={save}>Save trim</Button>
      {#if dirty}<Button size="sm" onclick={() => (draft = null)}>Cancel</Button>{/if}
      {#if !dirty && (asset.prep?.trimIn != null || asset.prep?.trimOut != null)}<Button size="sm" disabled={readOnly || mediaPrep.busy} onclick={clear}>Clear trim</Button>{/if}
    </div>
    <p class="note">Cues that set their own in or out point keep them.</p>
  </div>
{/if}

<style>
  .prep-cap {
    margin-top: 14px;
  }
  .prep,
  .trim {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .state {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-sm);
    font-weight: 600;
    color: var(--text-strong);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-label);
    flex: none;
  }
  .state.ok .dot {
    background: var(--ok, #3fb37f);
  }
  .state.busy .dot {
    background: var(--accent);
  }
  .state.warn .dot {
    background: var(--live);
  }
  .progress {
    height: 4px;
    border-radius: 2px;
    background: var(--surface-raised);
    overflow: hidden;
  }
  .progress span {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .kv {
    display: flex;
    justify-content: space-between;
    gap: 10px;
    font-size: var(--fs-sm);
  }
  .kv > span:first-child {
    color: var(--text-meta);
    flex: none;
  }
  .kv > span:last-child {
    color: var(--text-2);
    text-align: right;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .why,
  .note,
  .err {
    margin: 0;
    font-size: var(--fs-xs);
    line-height: 1.45;
    color: var(--text-meta);
  }
  .err {
    color: var(--live);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 2px;
  }
  .bar {
    position: relative;
    height: 26px;
    margin: 4px 7px;
    border-radius: 4px;
    background: var(--surface-raised);
    touch-action: none;
  }
  .cut,
  .keep {
    position: absolute;
    top: 0;
    bottom: 0;
  }
  .cut {
    background: repeating-linear-gradient(135deg, transparent 0 5px, rgba(255, 255, 255, 0.05) 5px 10px);
  }
  .keep {
    background: var(--accent-sel-2);
    border-top: 2px solid var(--accent);
    border-bottom: 2px solid var(--accent);
  }
  .poster {
    position: absolute;
    top: -3px;
    bottom: -3px;
    width: 2px;
    margin-left: -1px;
    background: var(--text-2);
    opacity: 0.6;
  }
  .handle {
    position: absolute;
    top: -3px;
    bottom: -3px;
    width: 12px;
    margin-left: -6px;
    padding: 0;
    border: 0;
    border-radius: 3px;
    background: var(--accent);
    cursor: ew-resize;
  }
  .handle:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .times {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: var(--fs-sm);
    color: var(--text-meta);
  }
  .times input {
    width: 62px;
    margin-left: 4px;
  }
  .len {
    margin-left: auto;
  }
</style>
