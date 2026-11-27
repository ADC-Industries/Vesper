<script lang="ts">
  // A status dot with an optional label. Green = on / healthy, grey = off, amber = live or a
  // warning, red = error or offline, blue = information.
  import type { Status } from "./types";

  let { status, label, pulse = false }: { status: Status; label?: string; pulse?: boolean } = $props();
</script>

<span class="ui-status" role={label ? undefined : "img"} aria-label={label ? undefined : status}>
  <span class="dot {status}" class:pulse aria-hidden="true"></span>
  {#if label}<span class="label">{label}</span>{/if}
</span>

<style>
  .ui-status {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
    min-width: 0;
  }
  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .ok {
    background: var(--ok);
  }
  .off {
    background: var(--off);
  }
  .live,
  .warn {
    background: var(--live);
  }
  .error {
    background: var(--danger);
  }
  .info {
    background: var(--info);
  }
  .pulse {
    animation: pulse 1.4s ease-in-out infinite;
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }
</style>
