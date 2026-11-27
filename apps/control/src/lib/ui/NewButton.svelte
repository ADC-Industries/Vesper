<script lang="ts">
  // The dashed "+ New …" button that ends lists and grids.
  import Plus from "lucide-svelte/icons/plus";
  import type { HTMLButtonAttributes } from "svelte/elements";
  import { ICON_SIZE, ICON_STROKE } from "./types";

  let { label, ...rest }: HTMLButtonAttributes & { label: string } = $props();
</script>

<button type="button" class="ui-new" {...rest}>
  <Plus size={ICON_SIZE} strokeWidth={ICON_STROKE} aria-hidden="true" />
  {label}
</button>

<style>
  .ui-new {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-3);
    width: 100%;
    min-height: var(--control-h);
    padding: 6px 10px;
    border: 1px dashed var(--line-strong);
    border-radius: var(--r);
    background: transparent;
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .ui-new:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--text);
    background: var(--accent-sel-2);
  }
  .ui-new:disabled {
    opacity: 0.45;
  }
</style>
