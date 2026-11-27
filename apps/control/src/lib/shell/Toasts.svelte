<script lang="ts">
  // Notices as toasts, bottom centre (rules in toasts.ts). "N more" opens Diagnostics, which
  // keeps the full picture.
  import type { Connection } from "../server";
  import Toast from "../ui/Toast.svelte";
  import { shell } from "../state/shell.svelte";
  import { toastLifetime, visibleToasts } from "./toasts";

  let { conn }: { conn: Connection } = $props();
  // svelte-ignore state_referenced_locally
  const notices = conn.notices;

  const timers = new Map<number, ReturnType<typeof setTimeout>>();

  let view = $derived(visibleToasts($notices));

  $effect(() => {
    for (const n of $notices) {
      const ms = toastLifetime(n.level);
      if (ms !== null && !timers.has(n.id)) {
        timers.set(
          n.id,
          setTimeout(() => {
            timers.delete(n.id);
            conn.dismissNotice(n.id);
          }, ms),
        );
      }
    }
  });

  function more() {
    if (shell.closeDialogs()) shell.showDiagnostics = true;
  }
</script>

<div class="toasts" aria-live="polite">
  {#if view.folded > 0}
    <button class="more" onclick={more} title="Open Diagnostics">
      {view.folded} more notice{view.folded === 1 ? "" : "s"}
    </button>
  {/if}
  {#each view.shown as n (n.id)}
    <div class="slot"><Toast level={n.level} message={n.message} count={n.count} ondismiss={() => conn.dismissNotice(n.id)} /></div>
  {/each}
</div>

<style>
  /* Bottom centre of the window, above the status bar, as in the canvas. */
  .toasts {
    position: fixed;
    left: 50%;
    bottom: 44px;
    transform: translateX(-50%);
    z-index: 70;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    pointer-events: none;
  }
  .slot,
  .more {
    pointer-events: auto;
  }
  .more {
    padding: 4px 12px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r-pill);
    background: var(--surface-raised);
    color: var(--text-2);
    font-size: var(--fs-sm);
    box-shadow: var(--shadow-toast);
  }
  .more:hover {
    color: var(--text);
    border-color: var(--line-hover);
  }
</style>
