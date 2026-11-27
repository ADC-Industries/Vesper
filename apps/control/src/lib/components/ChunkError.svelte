<script lang="ts">
  // A part of Oriel that loads on first use (its own chunk) couldn't be loaded (D11-3): most
  // often a browser tab left open across a server upgrade, whose old chunk names are gone. Says
  // so and offers a reload, instead of an empty page.
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import { Button } from "../ui";

  let { error, what = "This page" }: { error: unknown; what?: string } = $props();
  let detail = $derived(error instanceof Error ? error.message : String(error));
</script>

<div class="chunk-error" role="alert" data-chunk-error>
  <TriangleAlert size={18} strokeWidth={2} aria-hidden="true" />
  <div class="txt">
    <b>{what} couldn't be loaded.</b>
    <span>The server was probably updated since Oriel opened. Reload to get the new version.</span>
    <span class="detail" title={detail}>{detail}</span>
  </div>
  <Button size="sm" variant="primary" onclick={() => location.reload()}>Reload</Button>
</div>

<style>
  .chunk-error {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    margin: 16px;
    padding: 12px 14px;
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: 6px;
    background: color-mix(in srgb, var(--danger) 10%, var(--surface-panel));
    color: var(--text);
    font-size: var(--fs-sm);
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
    color: var(--text-2);
  }
  b {
    color: var(--text);
  }
  .detail {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-label);
    font-size: 11px;
  }
</style>
