<script lang="ts">
  // Small state chips: LIVE (amber), NEXT, DISABLED, EDITED, TEST, IN SET.
  import type { Snippet } from "svelte";
  import type { ChipKind } from "./types";

  const LABELS: Record<ChipKind, string> = {
    live: "LIVE",
    next: "NEXT",
    disabled: "DISABLED",
    edited: "EDITED",
    test: "TEST",
    inset: "IN SET",
    neutral: "",
  };

  let { kind = "neutral", title, children }: { kind?: ChipKind; title?: string; children?: Snippet } = $props();
</script>

<span class="ui-chip {kind}" {title}>
  {#if children}{@render children()}{:else}{LABELS[kind]}{/if}
</span>

<style>
  .ui-chip {
    display: inline-flex;
    align-items: center;
    flex: none;
    padding: 1px 5px;
    border-radius: var(--r-xs);
    font-size: var(--fs-2xs);
    font-weight: 700;
    letter-spacing: 0.04em;
    line-height: 1.4;
    white-space: nowrap;
  }
  .live {
    background: var(--live);
    color: var(--live-ink);
  }
  .next {
    background: #000;
    color: var(--text);
    box-shadow: inset 0 0 0 1px var(--next-ring);
  }
  .disabled {
    background: var(--surface-overlay);
    color: var(--text-label);
  }
  .edited {
    background: var(--accent-sel);
    color: var(--accent-link);
  }
  .test {
    background: var(--live-soft);
    color: var(--live);
  }
  .inset {
    background: var(--ok-bg);
    color: var(--ok-text);
  }
  .neutral {
    background: var(--surface-overlay);
    color: var(--text-2);
  }
</style>
