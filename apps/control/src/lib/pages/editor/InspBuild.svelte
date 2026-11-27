<script lang="ts">
  // Inspector, Build tab (plan G18, docs/BUILDS-PLAN.md): the slide's build order, one row per
  // build (step, object, in / out, trigger, effect with direction, duration, delay; move up /
  // down, remove), "Build in" / "Build out" for the selected object, and a step preview that
  // shows the canvas as the audience sees it after each `next`. Every change is one
  // `setSlide {builds}`.
  import type { Build, BuildStart, Direction, Slide, TransitionKind } from "$protocol";
  import type { EditState } from "../../edit/editState.svelte";
  import { elementName } from "../../edit/elements";
  import Button from "../../ui/Button.svelte";
  import Num from "./Num.svelte";
  import { EFFECTS, TRIGGERS, buildSteps, effectOf, moveBuild, newBuild, stepCount, withEffect, type BuildEffect } from "./builds";
  import { DIRECTIONS } from "./transitions";
  import { shown, type Sent } from "../../edit/optimistic";

  let { st, slide }: { st: EditState; slide: Slide } = $props();

  // The builds just sent, until that save settled: the next change builds on them (D10-2: a
  // Num committed on pointerdown, then a click, in one gesture; E10-5).
  let sent = $state.raw<Sent<Build[]> | null>(null);
  let builds = $derived(shown(sent, slide.id, slide.builds ?? []));
  let timed = $derived(buildSteps({ ...slide, builds }));
  let steps = $derived(stepCount({ ...slide, builds }));
  let names = $derived(new Map((slide.elements ?? []).map((e) => [e.id, e.name || elementName(e)])));
  let sel = $derived(st.selected.length === 1 ? st.selected[0] : null);
  let selModes = $derived(new Set(builds.filter((b) => b.element === sel?.id).map((b) => b.mode ?? "in")));

  function save(label: string, next: Build[]) {
    const mine = { key: slide.id, value: next };
    sent = mine;
    void st.run(label, [{ type: "setSlide", slide: slide.id, builds: next }]).finally(() => {
      if (sent === mine) sent = null;
    });
  }
  function setAt(i: number, label: string, patch: Partial<Build>) {
    save(label, builds.map((b, k) => (k === i ? { ...b, ...patch } : b)));
  }
  function stepOf(i: number): number | null {
    return timed.find((t) => t.index === i)?.step ?? null;
  }
  function direction(b: Build): Direction | "in" | "out" | null {
    const k = b.transition?.kind;
    if (k?.type === "push" || k?.type === "slide" || k?.type === "wipe") return k.direction;
    if (k?.type === "zoom") return k.direction === "out" ? "out" : "in";
    return null;
  }
  function setDirection(i: number, d: string) {
    const t = builds[i]?.transition;
    const k = t?.kind;
    if (!t || !k) return;
    const direction = d as Direction;
    let kind: TransitionKind | null = null;
    if (k.type === "zoom") kind = d === "out" ? { type: "zoom", direction: "out" } : { type: "zoom" };
    else if (k.type === "push") kind = { type: "push", direction };
    else if (k.type === "slide") kind = { type: "slide", direction };
    else if (k.type === "wipe") kind = { type: "wipe", direction };
    if (kind) setAt(i, "Build Direction", { transition: { ...t, kind } });
  }
  function preview(step: number | null) {
    st.buildPreview = step === null ? null : Math.max(0, Math.min(steps, step));
  }
</script>

<section class="sec builds" data-builds>
  <div class="line between">
    <h4>Build order</h4>
    <span class="count">{steps ? `${steps} step${steps === 1 ? "" : "s"} on Next` : "No steps"}</span>
  </div>
  {#if !builds.length}
    <p class="note">No builds: every object shows when the slide goes live. Select an object and add a build to bring it in (or take it out) on Next.</p>
  {/if}
  <ol class="rows">
    {#each builds as b, i (i)}
      {@const step = stepOf(i)}
      {@const eff = effectOf(b.transition)}
      {@const dir = direction(b)}
      <li class="row" class:dangling={step === null} class:cur={sel?.id === b.element} data-build-row={i}>
        <div class="head">
          <span class="step" title={step === null ? "Its object was deleted: the build is skipped" : step === 0 ? "Plays with the slide" : `Plays on Next ${step}`}>{step === null ? "–" : step === 0 ? "S" : step}</span>
          <button class="name" onclick={() => st.select([b.element])} title="Select the object">{names.get(b.element) ?? "Deleted object"}</button>
          <select class="mode" value={b.mode ?? "in"} onchange={(e) => setAt(i, "Build In or Out", { mode: e.currentTarget.value === "out" ? "out" : undefined })} aria-label="In or out" data-field="build-mode">
            <option value="in">In</option>
            <option value="out">Out</option>
          </select>
          <button class="ic" disabled={i === 0} onclick={() => save("Move Build", moveBuild(builds, i, -1))} aria-label="Move up" title="Move up">↑</button>
          <button class="ic" disabled={i === builds.length - 1} onclick={() => save("Move Build", moveBuild(builds, i, 1))} aria-label="Move down" title="Move down">↓</button>
          <button class="ic" onclick={() => save("Remove Build", builds.filter((_, k) => k !== i))} aria-label="Remove build" title="Remove">✕</button>
        </div>
        <div class="line">
          <select value={b.start ?? "onNext"} onchange={(e) => setAt(i, "Build Trigger", { start: e.currentTarget.value === "onNext" ? undefined : (e.currentTarget.value as BuildStart) })} aria-label="Starts" data-field="build-start">
            {#each TRIGGERS as t (t.id)}<option value={t.id}>{t.label}</option>{/each}
          </select>
          <select value={eff} onchange={(e) => setAt(i, "Build Effect", { transition: withEffect(b.transition, e.currentTarget.value as BuildEffect) })} aria-label="Effect" data-field="build-effect">
            {#each EFFECTS as x (x.id)}<option value={x.id}>{x.label}</option>{/each}
          </select>
          {#if dir}
            <select value={dir} onchange={(e) => setDirection(i, e.currentTarget.value)} aria-label="Direction" data-field="build-direction">
              {#if eff === "zoom"}
                <option value="in">In</option><option value="out">Out</option>
              {:else}
                {#each DIRECTIONS.filter((d) => eff === "wipe" || d.id !== "center") as d (d.id)}<option value={d.id}>{d.label}</option>{/each}
              {/if}
            </select>
          {/if}
        </div>
        <!-- Each label stays with its field: at 1366 the two pairs share one line; narrower,
             Delay moves down whole instead of leaving its field behind. -->
        <div class="line times">
          {#if eff !== "cut"}
            <span class="pair">
              <span class="lab">Duration</span>
              <Num value={b.transition?.duration ?? 0} min={0} max={10} step={0.1} unit="s" width={48} label="Build duration" onchange={(v) => b.transition && setAt(i, "Build Duration", { transition: { ...b.transition, duration: v } })} />
            </span>
          {/if}
          <span class="pair">
            <span class="lab">Delay</span>
            <Num value={b.delay ?? 0} min={0} max={60} step={0.1} unit="s" width={48} label="Build delay" onchange={(v) => setAt(i, "Build Delay", { delay: v || undefined })} />
          </span>
        </div>
      </li>
    {/each}
  </ol>
  <div class="line">
    <Button size="sm" disabled={!sel || selModes.has("in")} onclick={() => sel && save("Add Build", [...builds, newBuild(sel.id, "in")])} data-action="build-in">Build in</Button>
    <Button size="sm" disabled={!sel || selModes.has("out")} onclick={() => sel && save("Add Build", [...builds, newBuild(sel.id, "out")])} data-action="build-out">Build out</Button>
    {#if !sel}<span class="note">Select one object to add a build.</span>{/if}
  </div>
</section>

<section class="sec" data-build-preview>
  <div class="line between">
    <h4>Preview</h4>
    {#if st.buildPreview !== null}<span class="count">{st.buildPreview === 0 ? "As the slide goes live" : `After Next ${st.buildPreview}`}</span>{/if}
  </div>
  <div class="line">
    <Button size="sm" disabled={!steps && !timed.length} onclick={() => preview(st.buildPreview === null ? 0 : null)} data-action="build-preview">{st.buildPreview === null ? "Preview steps" : "Show all"}</Button>
    {#if st.buildPreview !== null}
      <Button size="sm" disabled={st.buildPreview <= 0} onclick={() => preview((st.buildPreview ?? 0) - 1)} aria-label="Previous step">◀</Button>
      <span class="count">{st.buildPreview} / {steps}</span>
      <Button size="sm" disabled={st.buildPreview >= steps} onclick={() => preview((st.buildPreview ?? 0) + 1)} aria-label="Next step">▶</Button>
    {/if}
  </div>
  <p class="note">The canvas shows each step settled; the Show page plays the motion.</p>
</section>

<style>
  .count {
    color: var(--text-meta);
    font-size: 12px;
  }
  .note {
    margin: 0;
    color: var(--text-meta);
    font-size: 11.5px;
    line-height: 1.45;
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
  }
  .row.cur {
    border-color: var(--accent-ring);
  }
  .row.dangling {
    opacity: 0.6;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .step {
    flex: none;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--accent-sel);
    color: var(--accent-link);
    font-size: 11px;
    font-weight: 700;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: left;
    border: 0;
    background: none;
    color: var(--text-strong);
    font-weight: 600;
    padding: 0;
    cursor: pointer;
  }
  .mode {
    width: 62px;
  }
  .ic {
    flex: none;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-panel);
    color: var(--text);
  }
  .ic:disabled {
    opacity: 0.4;
  }
  .row select {
    min-width: 0;
  }
  .times {
    column-gap: 14px;
  }
  .pair {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  .pair .lab {
    min-width: 0;
  }
</style>
