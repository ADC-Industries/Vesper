<script lang="ts">
  // The Library's middle column for a media bin or a media playlist (G17): its toolbar
  // (rename, move up / down, delete) and its items as tiles, each with "Remove". Images and
  // videos are added by dragging their tiles onto the bin in the tree, or with "Add to" in the
  // Images / Videos views. A watched folder's bin is read only (its items follow the folder).
  import Pencil from "lucide-svelte/icons/pencil";
  import ArrowUp from "lucide-svelte/icons/arrow-up";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import X from "lucide-svelte/icons/x";
  import type { Connection } from "../../server";
  import { Button } from "../../ui";
  import { binsPage } from "../../state/bins.svelte";
  import { libraryPage } from "../../state/library.svelte";
  import { session } from "../../state/session.svelte";
  import { binMove, mediaPlaylists } from "./bins";
  import { formatDuration } from "./logic";

  let { conn, readOnly }: { conn: Connection; readOnly: boolean } = $props();

  let view = $derived(libraryPage.view);
  let row = $derived(view.kind === "bin" ? (binsPage.rows.find((r) => r.id === view.id) ?? null) : null);
  let pl = $derived(view.kind === "mediaPlaylist" ? (mediaPlaylists(session.playlistTree).find((p) => p.id === view.id) ?? null) : null);
  let watched = $derived(row?.kind === "watched");
  let locked = $derived(readOnly || binsPage.busy);

  interface Tile {
    id: string;
    asset: string | null;
    name: string;
    kind: string;
    missing: boolean;
    enabled: boolean;
    duration: number | null;
  }
  let tiles = $derived.by((): Tile[] => {
    if (view.kind === "bin" && binsPage.bin?.id === view.id)
      return binsPage.bin.items.map((i) => ({ id: i.id, asset: i.asset, name: i.name, kind: i.kind, missing: i.availability === "missing", enabled: i.enabled, duration: i.duration ?? null }));
    if (view.kind === "mediaPlaylist" && binsPage.playlist?.id === view.id)
      return binsPage.playlist.items.map((i) => ({
        id: i.id,
        asset: i.kind.type === "media" ? i.kind.asset : null,
        name: i.displayName,
        kind: i.kind.type,
        missing: false,
        enabled: i.enabled,
        duration: null,
      }));
    return [];
  });
  let loaded = $derived((view.kind === "bin" && binsPage.bin?.id === view.id) || (view.kind === "mediaPlaylist" && binsPage.playlist?.id === view.id));
</script>

<div class="bar">
  {#if row}
    <Button size="sm" variant="ghost" icon={Pencil} disabled={locked} onclick={() => binsPage.renameBin(row)}>Rename</Button>
    <Button size="sm" variant="ghost" icon={ArrowUp} iconOnly aria-label="Move up" title="Move up" disabled={locked || !binMove(binsPage.rows, row.id, -1)} onclick={() => binsPage.moveBin(row, -1)} />
    <Button size="sm" variant="ghost" icon={ArrowDown} iconOnly aria-label="Move down" title="Move down" disabled={locked || !binMove(binsPage.rows, row.id, 1)} onclick={() => binsPage.moveBin(row, 1)} />
    <span class="flex"></span>
    <Button size="sm" variant="ghost" icon={Trash2} disabled={locked} onclick={() => binsPage.deleteBin(row)}>Delete bin</Button>
  {:else if pl}
    <Button size="sm" variant="ghost" icon={Pencil} disabled={locked} onclick={() => binsPage.renameMediaPlaylist(pl)}>Rename</Button>
    <span class="flex"></span>
    <Button size="sm" variant="ghost" icon={Trash2} disabled={locked} onclick={() => binsPage.deleteMediaPlaylist(pl)}>Delete playlist</Button>
  {/if}
</div>
{#if watched}
  <p class="note">A watched folder: its items follow the folder on disk, so they aren't edited here.</p>
{:else}
  <p class="note">Drag images or videos onto a bin or media playlist in the list on the left to add them.</p>
{/if}
<div class="grid-wrap">
  <div class="grid">
    {#each tiles as t (t.id)}
      <div class="tile" class:off={!t.enabled}>
        <span class="thumb">
          {#if t.asset && !t.missing}<img src={conn.rest.mediaThumbnailUrl(t.asset, 320)} alt="" loading="lazy" draggable="false" />{/if}
          {#if t.missing}<span class="badge warn">MISSING</span>
          {:else if t.duration}<span class="badge">{formatDuration(t.duration)}</span>{/if}
        </span>
        <span class="tname" title={t.name}>{t.name}</span>
        <span class="tsub">{t.kind === "video" ? "Video" : t.kind === "audio" ? "Audio" : t.kind === "media" ? "Media" : t.kind === "image" ? "Image" : t.kind}{t.enabled ? "" : " · disabled"}</span>
        {#if !watched}
          <button type="button" class="remove" disabled={locked} aria-label={`Remove ${t.name}`} title="Remove (the file stays in the collection)" onclick={() => binsPage.removeItem(t.id)}>
            <X size={13} strokeWidth={2} aria-hidden="true" />
          </button>
        {/if}
      </div>
    {:else}
      <p class="empty">{loaded ? (watched ? "Nothing in this folder yet." : "Nothing here yet. Drag images or videos here from Images or Videos.") : "Loading…"}</p>
    {/each}
  </div>
</div>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 12px;
    border-top: 1px solid var(--surface-raised);
  }
  .flex {
    flex: 1;
  }
  .note {
    margin: 0;
    padding: 2px 16px 6px;
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
  .grid-wrap {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 12px 12px;
    border-top: 1px solid var(--surface-raised);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
    padding-top: 8px;
  }
  .empty {
    grid-column: 1 / -1;
    margin: 0;
    padding: 16px;
    color: var(--text-label);
  }
  .tile {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px;
    border-radius: 7px;
    border: 2px solid transparent;
    background: var(--surface-panel);
    color: var(--text);
    min-width: 0;
  }
  .tile:hover {
    border-color: var(--line-strong);
  }
  .tile.off .thumb {
    opacity: 0.45;
  }
  .thumb {
    position: relative;
    display: block;
    aspect-ratio: 16 / 9;
    border-radius: 4px;
    overflow: hidden;
    background: var(--surface-well);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .badge {
    position: absolute;
    right: 4px;
    bottom: 4px;
    padding: 0 5px;
    border-radius: 3px;
    background: rgba(0, 0, 0, 0.65);
    font-size: 10px;
    font-weight: 800;
  }
  .badge.warn {
    background: var(--live);
    color: var(--live-ink);
  }
  .tname,
  .tsub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tname {
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .tsub {
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
  .remove {
    position: absolute;
    top: 8px;
    right: 8px;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: rgba(0, 0, 0, 0.7);
    color: #fff;
    opacity: 0;
  }
  .tile:hover .remove,
  .remove:focus-visible {
    opacity: 1;
  }
  .remove:disabled {
    display: none;
  }
</style>
