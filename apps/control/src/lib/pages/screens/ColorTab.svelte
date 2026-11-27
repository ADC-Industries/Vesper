<script lang="ts">
  // The Color tab of a screen's output card (plan 4.8): brightness, contrast, gamma,
  // saturation, R/G/B gain and black level, each a slider with a number field, plus Reset.
  // Changes apply live (PUT /screens/{id}/color, throttled while dragging) to the whole
  // screen, so every output of the screen gets them.
  import type { ColorCorrection, ScreenEntry } from "$protocol";
  import { Button } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { COLOR_FIELDS, IDENTITY_COLOR, clampField, colorOf, fieldNumber, isIdentityColor, throttle, type ColorField } from "./adjust";

  let { entry, onapplied }: { entry: ScreenEntry; onapplied: (e: ScreenEntry) => void } = $props();

  let value = $state<ColorCorrection>(colorOf(null));
  let error = $state<string | null>(null);
  /** Revisions our own changes produced: the server copy is taken over only when someone
   * else changed it (our replies arrive while later edits are still on their way). */
  const mine = new Set<string>();
  $effect(() => {
    const c = colorOf(entry);
    if (!mine.has(entry.revision)) value = c;
  });

  const send = throttle(async (id: string, c: ColorCorrection) => {
    const rest = session.conn?.rest;
    if (!rest) return;
    try {
      const r = await rest.setScreenColor(id, isIdentityColor(c) ? {} : c);
      error = null;
      mine.add(r.revision);
      onapplied(r);
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }, 120);

  function set(f: ColorField, v: number) {
    value = { ...value, [f.key]: clampField(f, v) };
    send(entry.id, value);
  }

  function reset() {
    value = { ...IDENTITY_COLOR };
    send(entry.id, value);
    send.flush();
  }

  const shown = (f: ColorField, v: number) => (f.step < 1 ? v.toFixed(2) : String(v));

  /** The number field: an emptied or invalid field changes nothing and shows the value again. */
  function typed(f: ColorField, input: HTMLInputElement) {
    const v = fieldNumber(input.value);
    if (Number.isNaN(v)) input.value = shown(f, value[f.key]);
    else set(f, v);
  }
</script>

<div class="ctab" data-testid="color-tab">
  <p class="note">Applies to the whole screen, so every output of “{entry.name}” gets it. Changes go live at once.</p>
  <div class="rows">
    {#each COLOR_FIELDS as f (f.key)}
      <label class="row">
        <span class="name">{f.label}</span>
        <input type="range" min={f.min} max={f.max} step={f.step} value={value[f.key]} oninput={(e) => set(f, Number(e.currentTarget.value))} data-testid={`color-${f.key}`} />
        <input class="num" type="number" min={f.min} max={f.max} step={f.step} value={shown(f, value[f.key])} aria-label={`${f.label} value`} onchange={(e) => typed(f, e.currentTarget)} data-testid={`color-${f.key}-value`} />
      </label>
    {/each}
  </div>
  <div class="foot">
    {#if error}<span class="err" role="alert">{error}</span>{/if}
    <span class="grow"></span>
    <Button size="sm" onclick={reset} disabled={isIdentityColor(value)} title="Back to no correction" data-testid="color-reset">Reset</Button>
  </div>
</div>

<style>
  .ctab {
    max-width: 560px;
    margin: 0 auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .note {
    margin: 0;
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .rows {
    display: grid;
    grid-template-columns: 1fr;
    gap: 4px;
  }
  .row {
    display: grid;
    grid-template-columns: 96px 1fr 72px;
    align-items: center;
    gap: 10px;
    color: var(--text-2);
  }
  .row input[type="range"] {
    width: 100%;
    accent-color: var(--accent-ring);
  }
  .num {
    width: 100%;
    box-sizing: border-box;
    padding: 4px 6px;
    border-radius: 6px;
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    color: var(--text);
    font: inherit;
    font-variant-numeric: tabular-nums;
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
</style>
