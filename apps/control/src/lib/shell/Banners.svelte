<script lang="ts">
  // The connection banners under the header (docs/ORIEL-REDESIGN-PLAN.md 3.4): red while the
  // server can't be reached (attempt count, Retry now), green for 3.5 s once it is back.
  // Commands are never queued (a slide must not fire late), so the banner says they wait.
  import { onDestroy } from "svelte";
  import Alert from "lucide-svelte/icons/triangle-alert";
  import type { Connection } from "../server";

  let { conn }: { conn: Connection } = $props();
  // svelte-ignore state_referenced_locally
  const status = conn.status;

  const BACK_MS = 3500;

  // The first attempt right after the connect screen is not "offline" yet.
  let offline = $derived(
    $status.phase === "reconnecting" || $status.phase === "incompatible" || ($status.phase === "connecting" && $status.attempt > 0),
  );
  let back = $state(false);
  let wasOffline = false;
  let timer: ReturnType<typeof setTimeout> | null = null;

  $effect(() => {
    if (offline) {
      wasOffline = true;
      back = false;
    } else if (wasOffline && $status.phase === "online") {
      wasOffline = false;
      back = true;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => (back = false), BACK_MS);
    }
  });
  onDestroy(() => timer && clearTimeout(timer));
</script>

{#if offline}
  <div class="banner off" role="alert" aria-live="assertive">
    <Alert size={16} strokeWidth={1.8} aria-hidden="true" />
    {#if $status.phase === "incompatible"}
      <strong>This server runs an incompatible version.</strong>
      <span class="detail">Update Oriel or the server.</span>
    {:else}
      <strong>Not connected to the Vesper server — reconnecting…</strong>
      <span class="detail">Attempt {Math.max(1, $status.attempt)} · Slides and edits wait until the connection is back.</span>
      <span class="fill"></span>
      <button onclick={() => conn.socket.retryNow()}>Retry now</button>
    {/if}
  </div>
{:else if back}
  <div class="banner ok" role="status">
    <span class="dot" aria-hidden="true"></span>Reconnected — everything is back in sync.
  </div>
{/if}

<style>
  .banner {
    flex: none;
    position: relative;
    z-index: 40;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 14px;
    padding: 8px 14px;
    font-size: var(--fs);
  }
  .off {
    background: var(--danger-bg);
    color: var(--text-strong);
  }
  .ok {
    gap: 8px;
    padding: 6px 14px;
    background: var(--ok-bg);
    color: var(--ok-bg-text);
    font-size: 12.5px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--ok);
  }
  strong {
    font-weight: 700;
  }
  .detail {
    font-size: 12.5px;
    opacity: 0.9;
  }
  .fill {
    flex: 1 1 10px;
  }
  button {
    padding: 5px 12px;
    border: 1px solid rgba(255, 255, 255, 0.6);
    border-radius: var(--r);
    background: transparent;
    color: var(--text-strong);
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  button:hover {
    background: rgba(255, 255, 255, 0.12);
    border-color: var(--text-strong);
  }
</style>
