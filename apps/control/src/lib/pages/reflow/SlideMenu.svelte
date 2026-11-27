<script lang="ts">
  // A Reflow slide's "…" menu (entries from slideMenu.ts): group swatches with a ✓ on the
  // slide's own group, a heading, separators, disabled entries. ↑ / ↓ / Home / End move, a
  // letter jumps, Enter / Space choose, Esc / Tab close. It takes the focus when it opens; the
  // page puts it back in the slide's text when it closes.
  import Check from "lucide-svelte/icons/check";
  import { menuMove, menuTypeahead } from "../../ui/logic";
  import { cssColor } from "../../model";
  import { entryFocusable, type SlideMenuEntry } from "./slideMenu";

  let {
    entries,
    label,
    anchor,
    onselect,
    onclose,
  }: {
    entries: SlideMenuEntry[];
    label: string;
    /** The button that toggles this menu (a press on it is not "outside"). */
    anchor: HTMLElement | null;
    onselect: (e: SlideMenuEntry) => void;
    onclose: () => void;
  } = $props();

  let active = $state(-1);
  let list: HTMLDivElement;
  // menuMove / menuTypeahead skip disabled entries: headings count as disabled here.
  let nav = $derived(entries.map((e) => ({ disabled: !entryFocusable(e) })));

  function focusItem(i: number) {
    active = i;
    const el = list.querySelector<HTMLElement>(`[data-index="${i}"]`);
    el?.focus();
    el?.scrollIntoView({ block: "nearest" });
  }

  function attach(node: HTMLDivElement) {
    // The slide's own group first (when it has one), else the first entry.
    const own = entries.findIndex((e) => e.checked);
    const first = own >= 0 ? own : entries.findIndex(entryFocusable);
    queueMicrotask(() => {
      // The whole menu in view (the editor scrolls), then the entry.
      node.scrollIntoView({ block: "nearest" });
      if (first >= 0) focusItem(first);
      else node.focus({ preventScroll: true });
    });
    const down = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!node.contains(t) && !anchor?.contains(t)) onclose();
    };
    window.addEventListener("pointerdown", down, true);
    return () => window.removeEventListener("pointerdown", down, true);
  }

  function choose(e: SlideMenuEntry) {
    if (entryFocusable(e)) onselect(e);
  }

  function key(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      onclose();
      return;
    }
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      const it = entries[active];
      if (it) choose(it);
      return;
    }
    const next =
      menuMove(nav, active, e.key) ??
      (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey
        ? menuTypeahead(
            entries.map((x) => (x.heading ? null : (x.label ?? null))),
            nav,
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

<div class="slide-menu" role="menu" aria-label={label} tabindex="-1" bind:this={list} onkeydown={key} {@attach attach}>
  {#each entries as it, i (it.id)}
    {#if it.separator}
      <div class="sep" role="separator"></div>
    {:else if it.heading}
      <div class="heading" role="presentation">{it.label}</div>
    {:else}
      <button
        type="button"
        role={it.group ? "menuitemcheckbox" : "menuitem"}
        aria-checked={it.group ? !!it.checked : undefined}
        aria-disabled={it.disabled || undefined}
        class:danger={it.danger}
        class:disabled={it.disabled}
        tabindex="-1"
        data-index={i}
        data-entry={it.id}
        onclick={() => choose(it)}
        onpointermove={() => entryFocusable(it) && active !== i && focusItem(i)}
      >
        {#if it.group}
          <span class="swatch" style:background={cssColor(it.group.color, 1) ?? "var(--group-neutral)"}></span>
          <span class="label">{it.label}</span>
          <span class="mark" aria-hidden="true">{#if it.checked}<Check size={14} strokeWidth={2} />{/if}</span>
        {:else}
          <span class="label wide">{it.label}</span>
        {/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .slide-menu {
    position: absolute;
    right: 4px;
    top: 24px;
    z-index: 30;
    width: 220px;
    max-height: min(480px, calc(100vh - 140px));
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    padding: 4px;
    background: var(--surface-raised);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-menu);
    outline: none;
  }
  .heading {
    padding: 6px 8px 4px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-label);
  }
  button {
    display: grid;
    grid-template-columns: 10px minmax(0, 1fr) 14px;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text);
    font-size: 12.5px;
    text-align: left;
  }
  button:hover:not(.disabled),
  button:focus-visible,
  button:focus {
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
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 2px;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .label.wide {
    grid-column: 1 / -1;
  }
  .mark {
    display: grid;
    place-items: center;
    color: var(--accent-ring);
  }
  .sep {
    height: 1px;
    margin: 4px 0;
    background: var(--line-strong);
  }
</style>
