<script lang="ts">
  // A card: raised surface, 1 px line, 8 px radius. `selected` gives it the violet ring.
  import type { Snippet } from "svelte";
  import type { HTMLAttributes } from "svelte/elements";

  let {
    selected = false,
    padding = "md",
    children,
    class: cls = "",
    ...rest
  }: HTMLAttributes<HTMLDivElement> & { selected?: boolean; padding?: "none" | "sm" | "md"; children: Snippet } = $props();
</script>

<div class="ui-card pad-{padding} {cls}" class:selected {...rest}>
  {@render children()}
</div>

<style>
  .ui-card {
    background: var(--surface-raised);
    border: 1px solid var(--line);
    border-radius: var(--r-card);
  }
  .pad-sm {
    padding: var(--sp-4);
  }
  .pad-md {
    padding: var(--sp-6);
  }
  .selected {
    border-color: var(--accent);
    box-shadow: inset 0 0 0 1px var(--accent);
  }
</style>
