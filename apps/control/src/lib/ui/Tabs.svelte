<script lang="ts" generics="V extends string">
  // A tab strip with the violet underline. The panel is the caller's: give it
  // `role="tabpanel"`, `id="{idPrefix}-panel-{value}"` and `aria-labelledby="{idPrefix}-tab-{value}"`.
  import Badge from "./Badge.svelte";
  import { segmentedKey } from "./logic";
  import { ICON_SIZE, ICON_STROKE, type IconComponent } from "./types";

  interface Tab {
    value: V;
    label: string;
    icon?: IconComponent;
    iconOnly?: boolean;
    badge?: number;
    disabled?: boolean;
    title?: string;
  }

  let {
    tabs,
    value = $bindable(),
    label,
    idPrefix,
    fill = false,
    onchange,
  }: { tabs: Tab[]; value: V; label: string; idPrefix: string; fill?: boolean; onchange?: (v: V) => void } = $props();

  let root: HTMLDivElement;

  // With `onchange` the caller owns the value (it may refuse, e.g. the Screens page's unsaved
  // changes guard): the control shows what the caller passes, not the click.
  function pick(v: V) {
    if (v === value) return;
    if (onchange) onchange(v);
    else value = v;
  }

  function key(e: KeyboardEvent) {
    const next = segmentedKey(tabs, value, e.key);
    if (next === null) return;
    e.preventDefault();
    pick(next);
    root.querySelector<HTMLButtonElement>(`#${CSS.escape(`${idPrefix}-tab-${next}`)}`)?.focus();
  }
</script>

<div class="ui-tabs" class:fill role="tablist" aria-label={label} bind:this={root} onkeydown={key} tabindex="-1">
  {#each tabs as t (t.value)}
    <button
      type="button"
      role="tab"
      id="{idPrefix}-tab-{t.value}"
      aria-controls="{idPrefix}-panel-{t.value}"
      aria-selected={t.value === value}
      aria-label={t.iconOnly ? t.label : undefined}
      title={t.title ?? (t.iconOnly ? t.label : undefined)}
      tabindex={t.value === value ? 0 : -1}
      class:on={t.value === value}
      disabled={t.disabled}
      onclick={() => pick(t.value)}
    >
      {#if t.icon}{@const Icon = t.icon}<Icon size={ICON_SIZE} strokeWidth={ICON_STROKE} aria-hidden="true" />{/if}
      {#if !t.iconOnly}<span>{t.label}</span>{/if}
      {#if t.badge}<Badge count={t.badge} />{/if}
    </button>
  {/each}
</div>

<style>
  .ui-tabs {
    display: flex;
    align-items: stretch;
    gap: var(--sp-1);
    border-bottom: 1px solid var(--line);
    outline: none;
    overflow-x: auto;
    scrollbar-width: none;
  }
  button {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-3);
    padding: 8px 12px;
    border: 0;
    background: transparent;
    color: var(--text-label);
    font-size: var(--fs);
    font-weight: 600;
    white-space: nowrap;
  }
  .fill button {
    flex: 1;
    padding-left: 4px;
    padding-right: 4px;
  }
  button:hover:not(:disabled):not(.on) {
    color: var(--text);
  }
  button.on {
    color: var(--text);
  }
  button.on::after {
    content: "";
    position: absolute;
    left: 6px;
    right: 6px;
    bottom: -1px;
    height: 2px;
    border-radius: 1px;
    background: var(--accent-ring);
  }
  button:disabled {
    opacity: 0.45;
  }
  button:focus-visible {
    outline-offset: -2px;
  }
</style>
