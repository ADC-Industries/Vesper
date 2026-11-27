<script lang="ts">
  // The 11 px uppercase section label, with optional controls on the right.
  import type { Snippet } from "svelte";

  let {
    level = 2,
    id,
    children,
    actions,
  }: { level?: 2 | 3 | 4; id?: string; children: Snippet; actions?: Snippet } = $props();
</script>

<div class="ui-section">
  <svelte:element this={`h${level}`} class="section-label" {id}>{@render children()}</svelte:element>
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  .ui-section {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    min-height: 24px;
  }
  .section-label {
    flex: 1;
    min-width: 0;
    margin: 0;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
  }
</style>
