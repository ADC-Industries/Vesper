<script lang="ts" generics="V extends string | number">
  // A segmented control (Show | Edit | Reflow, Grid | Text | List): a radio group with one tab
  // stop; arrow keys, Home and End move the choice.
  import { segmentedKey } from "./logic";
  import { ICON_SIZE, ICON_STROKE, type IconComponent } from "./types";

  interface Option {
    value: V;
    label: string;
    icon?: IconComponent;
    /** Show only the icon (the label stays the accessible name). */
    iconOnly?: boolean;
    disabled?: boolean;
    title?: string;
  }

  let {
    options,
    value = $bindable(),
    label,
    size = "md",
    onchange,
  }: { options: Option[]; value: V; label: string; size?: "sm" | "md"; onchange?: (v: V) => void } = $props();

  let root: HTMLDivElement;

  // With `onchange` the caller owns the value (it may refuse, e.g. the Screens page's unsaved
  // changes guard): the control shows what the caller passes, not the click.
  function pick(v: V) {
    if (v === value) return;
    if (onchange) onchange(v);
    else value = v;
  }

  function key(e: KeyboardEvent) {
    const next = segmentedKey(options, value, e.key);
    if (next === null) return;
    e.preventDefault();
    pick(next);
    const i = options.findIndex((o) => o.value === next);
    (root.querySelectorAll("button")[i] as HTMLButtonElement | undefined)?.focus();
  }
</script>

<div class="ui-seg {size}" role="radiogroup" aria-label={label} bind:this={root} onkeydown={key} tabindex="-1">
  {#each options as o (o.value)}
    <button
      type="button"
      role="radio"
      aria-checked={o.value === value}
      aria-label={o.iconOnly ? o.label : undefined}
      title={o.title ?? (o.iconOnly ? o.label : undefined)}
      tabindex={o.value === value ? 0 : -1}
      class:on={o.value === value}
      class:icon-only={o.iconOnly}
      disabled={o.disabled}
      onclick={() => pick(o.value)}
    >
      {#if o.icon}{@const Icon = o.icon}<Icon size={ICON_SIZE} strokeWidth={ICON_STROKE} aria-hidden="true" />{/if}
      {#if !o.iconOnly}{o.label}{/if}
    </button>
  {/each}
</div>

<style>
  .ui-seg {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    background: var(--surface-app);
    border: 1px solid var(--line);
    border-radius: var(--r-card);
    outline: none;
  }
  button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-3);
    padding: 6px 14px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
    font-size: var(--fs);
    font-weight: 600;
  }
  .sm button {
    padding: 3px 9px;
    font-size: var(--fs-sm);
  }
  button.icon-only {
    padding: 5px 7px;
  }
  .sm button.icon-only {
    padding: 3px 5px;
  }
  button:hover:not(:disabled):not(.on) {
    background: var(--surface-raised);
    color: var(--text);
  }
  button.on {
    background: var(--accent);
    color: var(--text-strong);
  }
  button:disabled {
    opacity: 0.45;
  }
</style>
