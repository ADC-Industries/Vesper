<script lang="ts">
  // An empty state: icon, title, a line of text and an optional action (e.g. a NewButton).
  import type { Snippet } from "svelte";
  import type { IconComponent } from "./types";

  let {
    icon,
    title,
    children,
    action,
  }: { icon?: IconComponent; title: string; children?: Snippet; action?: Snippet } = $props();
</script>

<div class="ui-empty">
  {#if icon}{@const Icon = icon}<span class="icon" aria-hidden="true"><Icon size={28} strokeWidth={1.6} /></span>{/if}
  <p class="title">{title}</p>
  {#if children}<p class="text">{@render children()}</p>{/if}
  {#if action}<div class="action">{@render action()}</div>{/if}
</div>

<style>
  .ui-empty {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--sp-3);
    padding: 32px 16px;
    text-align: center;
    color: var(--text-label);
  }
  .icon {
    display: grid;
    place-items: center;
    width: 52px;
    height: 52px;
    margin-bottom: 4px;
    border-radius: var(--r-card);
    background: var(--surface-raised);
    color: var(--text-2);
  }
  p {
    margin: 0;
    max-width: 42ch;
  }
  .title {
    font-size: var(--fs-md);
    font-weight: 600;
    color: var(--text);
  }
  .text {
    font-size: var(--fs-sm);
    line-height: 1.5;
  }
  .action {
    margin-top: var(--sp-4);
  }
</style>
