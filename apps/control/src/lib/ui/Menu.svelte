<script lang="ts">
  // A menu list (in a Popover, or on its own as a context menu): ✓ marks, shortcut hints,
  // separators, disabled items. ↑ / ↓ / Home / End move, a letter jumps, Enter / Space choose,
  // Esc closes. It takes the focus when it opens and gives it back when it closes.
  import Check from "lucide-svelte/icons/check";
  import { focusable, menuMove, menuTypeahead } from "./logic";
  import { ICON_SIZE, ICON_STROKE, type MenuItem } from "./types";


  let {
    items,
    label,
    onselect,
    onclose,
  }: { items: MenuItem[]; label: string; onselect: (id: string) => void; onclose: () => void } = $props();

  let active = $state(-1);
  let list: HTMLDivElement;

  function focusItem(i: number) {
    active = i;
    list.querySelector<HTMLElement>(`[data-index="${i}"]`)?.focus();
  }

  function attach(node: HTMLDivElement) {
    const prev = document.activeElement as HTMLElement | null;
    const first = items.findIndex(focusable);
    if (first >= 0) queueMicrotask(() => focusItem(first));
    else node.focus();
    return () => prev?.focus?.();
  }

  function choose(it: MenuItem) {
    if (!focusable(it)) return;
    onselect(it.id);
  }

  function key(e: KeyboardEvent) {
    if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      e.stopPropagation();
      onclose();
      return;
    }
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      const it = items[active];
      if (it) choose(it);
      return;
    }
    const next =
      menuMove(items, active, e.key) ??
      (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey
        ? menuTypeahead(
            items.map((i) => i.label ?? null),
            items,
            active,
            e.key,
          )
        : null);
    if (next !== null) {
      e.preventDefault();
      focusItem(next);
    }
  }
</script>

<div class="ui-menu" role="menu" aria-label={label} tabindex="-1" bind:this={list} onkeydown={key} {@attach attach}>
  {#each items as it, i (it.id)}
    {#if it.separator}
      <div class="sep" role="separator"></div>
    {:else}
      <button
        type="button"
        role={it.checked === undefined ? "menuitem" : "menuitemcheckbox"}
        aria-checked={it.checked}
        aria-disabled={it.disabled || undefined}
        class:danger={it.danger}
        class:disabled={it.disabled}
        tabindex="-1"
        data-index={i}
        onclick={() => choose(it)}
        onpointermove={() => focusable(it) && active !== i && focusItem(i)}
      >
        <span class="mark" aria-hidden="true">
          {#if it.checked}<Check size={14} strokeWidth={2} />{:else if it.icon}{@const Icon = it.icon}<Icon
              size={ICON_SIZE}
              strokeWidth={ICON_STROKE}
            />{/if}
        </span>
        <span class="label">{it.label}{#if it.note}<span class="note"> · {it.note}</span>{/if}</span>
        {#if it.shortcut}<span class="kbd-hint">{it.shortcut}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .ui-menu {
    display: flex;
    flex-direction: column;
    min-width: 200px;
    padding: 4px;
    background: var(--surface-raised);
    outline: none;
  }
  button {
    display: grid;
    grid-template-columns: 20px minmax(0, 1fr) auto;
    align-items: center;
    gap: var(--sp-4);
    padding: 6px 10px 6px 6px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text);
    font-size: var(--fs);
    text-align: left;
  }
  button:hover:not(.disabled),
  button:focus-visible {
    background: var(--accent-sel);
    outline: none;
  }
  button.disabled {
    color: var(--text-disabled);
    cursor: default;
  }
  button.danger:not(.disabled) {
    color: var(--danger-text);
  }
  .mark {
    display: grid;
    place-items: center;
    color: var(--accent-ring);
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .note {
    color: var(--text-meta);
  }
  .kbd-hint {
    font-size: var(--fs-sm);
    color: var(--text-meta);
  }
  .sep {
    height: 1px;
    margin: 4px 6px;
    background: var(--line-strong);
  }
</style>
