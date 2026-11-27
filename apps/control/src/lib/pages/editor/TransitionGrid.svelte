<script lang="ts">
  // The transitions grid (Presentation and Slide tabs): the default, then Cut, Dissolve, Push,
  // Wipe, Zoom and Slide (plan G16). A moving kind shows its direction (arrows; zoom: in or
  // out); every kind but Cut its duration and easing. An imported transition shows read-only
  // until another is picked.
  import type { Easing, Transition } from "$protocol";
  import { shown, type Sent } from "../../edit/optimistic";
  import { transitionChoice, transitionFor, transitionLabel } from "../../edit/ops";
  import Num from "./Num.svelte";
  import { EASINGS, TRANSITION_GRID, directionOf, directionsFor, gridChoice, withDirection, type GridTransition } from "./transitions";

  let {
    transition: stored,
    owner,
    defaultLabel,
    label,
    onchange: send,
  }: {
    transition: Transition | undefined;
    /** What the transition belongs to (the slide's, presentation's or look's ID). */
    owner: string;
    defaultLabel: string;
    label: string;
    onchange: (t: Transition | null) => void | Promise<unknown>;
  } = $props();

  // The transition just sent, until that save settled: the next change (a duration committed
  // on pointerdown, then a click) builds on it (D10-2, E10-5). Kept per owner, so another slide
  // selected meanwhile shows its own (E10-6).
  let sent = $state.raw<Sent<Transition | undefined> | null>(null);
  let transition = $derived(shown(sent, owner, stored));
  function onchange(t: Transition | null) {
    const mine: Sent<Transition | undefined> = { key: owner, value: t ?? undefined };
    sent = mine;
    // A caller that answers with a promise (the save and refetch): the stored value rules after.
    void Promise.resolve(send(t)).finally(() => {
      if (sent === mine) sent = null;
    });
  }

  let cur = $derived(gridChoice(transition));
  let other = $derived(transitionChoice(transition) === "other");
  let duration = $derived(transition?.duration ?? 1);
  let dirs = $derived(directionsFor(cur));
  let dir = $derived(directionOf(transition));

  // Glyphs after the canvas's transition icons.
  const GLYPH: Record<GridTransition | "default", string> = {
    default: "M3 3h16v10H3z M7 8h8",
    cut: "M11 2v12",
    dissolve: "M2 3h7v10H2zM13 3h7v10h-7z",
    push: "M2 3h8v10H2zM12 8h8M17 5l3 3-3 3",
    wipe: "M2 3h18v10H2zM11 3v10",
    zoom: "M7 5h8v6H7zM2 2h18v12H2z",
    slide: "M8 3h12v10H8zM2 8h4",
  };

  function pick(id: GridTransition | "default") {
    if (id === "default") onchange(null);
    else if (id !== cur) onchange(transitionFor(id, duration, transition));
  }

  function setEasing(e: Easing) {
    if (transition) onchange({ ...transition, easing: e === "linear" ? undefined : e });
  }
</script>

<div class="tgrid" role="radiogroup" aria-label={label} data-transition-grid>
  {#each [{ id: "default" as const, label: defaultLabel, drawn: true }, ...TRANSITION_GRID] as t (t.id)}
    {@const on = t.id === "default" ? !transition : cur === t.id}
    <button class="cell" class:on role="radio" aria-checked={on} title={t.label} onclick={() => pick(t.id)} data-transition={t.id}>
      <svg width="22" height="16" viewBox="0 0 22 16" aria-hidden="true"><path d={GLYPH[t.id]} fill="none" stroke="currentColor" stroke-width="1.5" stroke-linejoin="round" /></svg>
      <span>{t.label}</span>
    </button>
  {/each}
</div>
{#if other}<p class="note">“{transitionLabel(transition)}” came from an import and is kept as it is; it plays as a dissolve of the same length. Pick another to replace it.</p>{/if}
{#if transition && cur && cur !== "cut"}
  {#if dirs.length}
    <div class="line">
      <span class="lab">Direction</span>
      <div class="seg" role="radiogroup" aria-label={`${label} direction`} data-transition-direction>
        {#each dirs as d (d.id)}
          <button class:on={dir === d.id} role="radio" aria-checked={dir === d.id} title={d.label} aria-label={d.label} onclick={() => onchange(withDirection(transition, d.id))} data-direction={d.id}>
            <svg width="16" height="16" viewBox="0 0 24 24" aria-hidden="true"><path d={d.glyph} fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" /></svg>
          </button>
        {/each}
      </div>
    </div>
  {:else if cur === "zoom"}
    <div class="line">
      <span class="lab">Zoom</span>
      <div class="seg words" role="radiogroup" aria-label={`${label} zoom`} data-transition-direction>
        {#each [{ id: "in" as const, label: "In" }, { id: "out" as const, label: "Out" }] as z (z.id)}
          <button class:on={dir === z.id} role="radio" aria-checked={dir === z.id} onclick={() => onchange(withDirection(transition, z.id))} data-direction={z.id}>{z.label}</button>
        {/each}
      </div>
    </div>
  {/if}
  <div class="line"><span class="lab">Duration</span><Num label={`${label} duration`} value={duration} min={0} max={10} step={0.1} unit="s" onchange={(v) => onchange({ ...transition, duration: Math.max(0, Math.round(v * 100) / 100) })} /></div>
  <div class="line">
    <span class="lab">Easing</span>
    <select value={transition.easing ?? "linear"} onchange={(e) => setEasing(e.currentTarget.value as Easing)} aria-label={`${label} easing`} data-field="transition-easing">
      {#each EASINGS as e (e.id)}<option value={e.id}>{e.label}</option>{/each}
    </select>
  </div>
{/if}

<style>
  .tgrid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(84px, 1fr));
    gap: 6px;
  }
  .cell {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 8px 4px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    font-size: 11.5px;
    text-align: center;
    min-width: 0;
  }
  .cell span {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cell:hover:not(:disabled) {
    background: var(--surface-overlay);
  }
  .cell.on {
    background: var(--accent-sel);
    border-color: var(--accent-ring);
  }
  .cell:disabled {
    opacity: 0.45;
    cursor: default;
  }
  /* The inspector's row styles, so the grid looks the same outside it (the Looks editor). */
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-top: 8px;
  }
  .lab {
    color: var(--text-label);
    font-size: 12px;
    min-width: 52px;
  }
  .seg {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--r);
    background: var(--surface-app);
    border: 1px solid var(--line);
  }
  .seg button {
    width: 30px;
    height: 26px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-label);
  }
  .seg button.on {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .seg.words button {
    width: auto;
    padding: 0 12px;
  }
</style>
