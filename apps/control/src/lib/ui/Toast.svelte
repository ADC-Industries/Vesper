<script lang="ts">
  // One toast (the canvas's: overlay surface, 8 px radius, toast shadow). The stack and its
  // timing are the caller's (components/Toasts).
  import CircleX from "lucide-svelte/icons/circle-x";
  import Info from "lucide-svelte/icons/info";
  import Alert from "lucide-svelte/icons/triangle-alert";
  import X from "lucide-svelte/icons/x";

  let {
    level = "info",
    message,
    count = 1,
    ondismiss,
  }: { level?: "info" | "warning" | "error"; message: string; count?: number; ondismiss?: () => void } = $props();

  const Icon = $derived(level === "error" ? CircleX : level === "warning" ? Alert : Info);
</script>

<div class="ui-toast {level}" role={level === "error" ? "alert" : "status"}>
  <span class="icon" aria-hidden="true"><Icon size={16} strokeWidth={1.8} /></span>
  <p title={message}>
    {message}
    {#if count > 1}<span class="count">×{count}</span>{/if}
  </p>
  {#if ondismiss}
    <button type="button" onclick={ondismiss} aria-label="Dismiss"><X size={14} strokeWidth={1.8} /></button>
  {/if}
</div>

<style>
  .ui-toast {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-4);
    max-width: min(560px, calc(100vw - 32px));
    padding: 9px 8px 9px 12px;
    background: var(--surface-overlay);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-card);
    box-shadow: var(--shadow-toast);
    color: var(--text);
    font-size: 12.5px;
    pointer-events: auto;
  }
  .icon {
    display: grid;
    padding-top: 1px;
    color: var(--info);
  }
  .warning .icon {
    color: var(--live);
  }
  .error {
    border-color: var(--danger);
  }
  .error .icon {
    color: var(--danger);
  }
  p {
    flex: 1;
    margin: 0;
    line-height: 1.45;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .count {
    margin-left: 4px;
    color: var(--text-meta);
    font-variant-numeric: tabular-nums;
  }
  button {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-label);
  }
  button:hover {
    background: var(--line-strong);
    color: var(--text);
  }
</style>
