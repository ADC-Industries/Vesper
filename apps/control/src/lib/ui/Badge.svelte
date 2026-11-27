<script lang="ts">
  // A count badge (tabs, list headers). Hidden when the count is 0 unless `showZero`.
  let {
    count,
    tone = "neutral",
    label,
    showZero = false,
  }: { count: number; tone?: "neutral" | "accent" | "live" | "error"; label?: string; showZero?: boolean } = $props();
</script>

{#if count > 0 || showZero}
  <span class="ui-badge {tone}" aria-label={label ? `${count} ${label}` : undefined}>{count > 99 ? "99+" : count}</span>
{/if}

<style>
  .ui-badge {
    display: inline-grid;
    place-items: center;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    border-radius: 9px;
    font-size: var(--fs-2xs);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .neutral {
    background: var(--surface-overlay);
    color: var(--text-2);
  }
  .accent {
    background: var(--accent);
    color: var(--text-strong);
  }
  .live {
    background: var(--live);
    color: var(--live-ink);
  }
  .error {
    background: var(--danger-bg);
    color: var(--text-strong);
  }
</style>
