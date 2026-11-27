<script lang="ts">
  // The live picture of the picked screen (`subscribePreview`), with LIVE / STAND-BY in the
  // corner (amber ring while it is on air) and the caller's overlays (the stereo meter).
  import { onDestroy, type Snippet } from "svelte";
  import type { Connection, PreviewImage } from "../../server";
  import { screensState } from "../../state/screens.svelte";
  import { useLive } from "../../live";

  let { conn, live: onAir, children }: { conn: Connection; live: boolean; children?: Snippet } = $props();

  const live = useLive();
  const screens = live.screens;
  // svelte-ignore state_referenced_locally
  const status = conn.status;

  const FPS = 10;
  const WIDTH = 480;

  let img = $state.raw<PreviewImage | null>(null);
  let screenId = $derived(screensState.previewScreen ?? $screens[0]?.id ?? null);
  let screen = $derived($screens.find((s) => s.id === screenId) ?? null);
  let online = $derived($status.phase === "online");

  // (Re)subscribe when the chosen screen changes. The socket re-sends the subscription
  // after every reconnect by itself.
  let subscribed: string | null = null;
  $effect(() => {
    const id = screenId;
    if (id === subscribed) return;
    if (subscribed) conn.socket.unsubscribePreview(subscribed);
    subscribed = id;
    img = id ? (conn.previews.get(id) ?? null) : null;
    if (id) conn.socket.subscribePreview(id, FPS, WIDTH);
  });

  // The view is re-created for a new connection, so `conn` never changes here.
  // svelte-ignore state_referenced_locally
  const off = conn.onPreview((p) => {
    if (p.screen === subscribed) img = p;
  });
  onDestroy(() => {
    off();
    if (subscribed) conn.socket.unsubscribePreview(subscribed);
  });

  let aspect = $derived(screen ? screen.width / screen.height : 16 / 9);
</script>

<div class="preview" class:stale={!online}>
  <div class="frame" class:onair={onAir} style:aspect-ratio={aspect}>
    {#if img}
      <img src={img.url} alt={`Live picture of ${screen?.name ?? "the screen"}`} />
    {:else}
      <span class="none">{online ? "Waiting for picture…" : "No picture while offline"}</span>
    {/if}
    <span class="badge" class:onair={onAir} data-testid="preview-badge">{onAir ? "LIVE" : "STAND-BY"}</span>
    {#if children}{@render children()}{/if}
    {#if !online && img}<span class="tag">OFFLINE · last picture</span>{/if}
  </div>
</div>

<style>
  .preview {
    position: relative;
    width: 100%;
  }
  .frame {
    position: relative;
    width: 100%;
    border-radius: var(--r);
    overflow: hidden;
    background: #000;
    outline: 2px solid var(--surface-overlay);
  }
  .frame.onair {
    outline-color: var(--live);
  }
  .badge {
    position: absolute;
    left: 8px;
    top: 8px;
    z-index: 3;
    padding: 1px 5px;
    border-radius: 3px;
    background: var(--surface-overlay);
    color: var(--text);
    font-size: 10px;
    font-weight: 700;
  }
  .badge.onair {
    background: var(--live);
    color: var(--live-ink);
  }
  .frame img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .stale .frame img {
    filter: grayscale(1) brightness(0.5);
  }
  .none {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-meta);
    font-size: 12px;
  }
  .tag {
    position: absolute;
    right: 6px;
    bottom: 6px;
    padding: 1px 6px;
    border-radius: 2px;
    background: var(--danger-bg);
    color: var(--text-strong);
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: 0.04em;
  }
</style>
