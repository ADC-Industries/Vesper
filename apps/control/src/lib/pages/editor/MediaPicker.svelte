<script lang="ts">
  // Picks an image (or a video, or either: `kind`) from the media bins (insert toolbar, media fills).
  import type { MediaBinDetail, MediaBinItemInfo } from "$protocol";
  import type { Connection } from "../../server";
  import { flattenBins, mediaState } from "../../state/media.svelte";

  let {
    conn,
    onpick,
    onclose,
    align = "left",
    kind = "image",
  }: { conn: Connection; onpick: (it: MediaBinItemInfo) => void; onclose: () => void; align?: "left" | "right"; kind?: "image" | "video" | "media" } = $props();

  let bins = $derived(flattenBins(mediaState.mediaBins).filter((b) => b.browser === "media"));
  let binId = $state<string | null>(mediaState.mediaBinId);
  let detail = $state.raw<MediaBinDetail | null>(null);
  let loading = $state(false);

  $effect(() => {
    const id = binId ?? bins[0]?.id ?? null;
    if (!id) return;
    loading = true;
    conn.rest
      .mediaBin(id)
      .then((d) => {
        if ((binId ?? bins[0]?.id) === id) detail = d;
      })
      .catch(() => (detail = null))
      .finally(() => (loading = false));
  });

  let images = $derived((detail?.items ?? []).filter((i) => (kind === "media" ? i.kind === "image" || i.kind === "video" : i.kind === kind)));
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && onclose()} />

<div class="picker" class:right={align === "right"} role="dialog" aria-label={kind === "video" ? "Choose a video" : kind === "media" ? "Choose an image or video" : "Choose an image"}>
  <div class="head">
    <select value={binId ?? bins[0]?.id ?? ""} onchange={(e) => (binId = (e.currentTarget as HTMLSelectElement).value)} aria-label="Media bin">
      {#each bins as b (b.id)}<option value={b.id}>{[...b.path, b.name].join(" / ")}</option>{/each}
    </select>
  </div>
  <div class="grid">
    {#each images as it (it.id)}
      <button class="it" onclick={() => onpick(it)} title={it.name} disabled={it.availability !== "online"} data-media-item={it.name}>
        <img src={conn.rest.mediaThumbnailUrl(it.asset, 160)} alt="" draggable="false" loading="lazy" />
        <span>{it.name}</span>
      </button>
    {:else}
      <p class="none">{loading ? "Loading…" : bins.length ? `No ${kind === "video" ? "videos" : kind === "media" ? "images or videos" : "images"} in this bin.` : "No media bins."}</p>
    {/each}
  </div>
</div>

<style>
  .picker {
    position: absolute;
    top: 40px;
    left: -100px;
    width: 340px;
    max-height: 360px;
    display: flex;
    flex-direction: column;
    background: var(--surface-raised);
    border: 1px solid var(--line-strong);
    border-radius: 4px;
    box-shadow: 0 8px 22px rgba(0, 0, 0, 0.55);
    z-index: 20;
  }
  .picker.right {
    left: auto;
    right: 0;
    top: 30px;
  }
  .head {
    padding: 6px;
    border-bottom: 1px solid var(--line);
  }
  select {
    width: 100%;
    height: 24px;
    background: var(--surface-panel);
    border: 1px solid var(--line-strong);
    border-radius: 2px;
  }
  .grid {
    overflow-y: auto;
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 6px;
    padding: 6px;
  }
  .it {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 2px;
    border: 1px solid transparent;
    border-radius: 3px;
    background: var(--surface-panel);
    color: var(--text);
    font-size: 11px;
    min-width: 0;
  }
  .it:hover:not(:disabled) {
    border-color: var(--accent-bright);
  }
  .it img {
    width: 100%;
    aspect-ratio: 16 / 9;
    object-fit: contain;
    background: #000;
  }
  .it span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .none {
    grid-column: 1 / -1;
    color: var(--text-faint);
    margin: 8px;
  }
</style>
