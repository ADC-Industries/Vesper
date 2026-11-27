<script lang="ts">
  // The bottom bin's Media tab, left side: the media bins (and their folders). Picking one
  // shows it in the bin.
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import Folder from "lucide-svelte/icons/folder";
  import FolderOpen from "lucide-svelte/icons/folder-open";
  import Images from "lucide-svelte/icons/images";
  import FolderSync from "lucide-svelte/icons/folder-sync";
  import Music from "lucide-svelte/icons/music";
  import type { MediaBinNode } from "$protocol";
  import { mediaState } from "../../state/media.svelte";
  import { showView } from "../../state/show.svelte";

  /** Media bins only (audio bins are the Audio tab's); folders left empty are dropped. */
  function mediaOnly(nodes: MediaBinNode[]): MediaBinNode[] {
    return nodes.flatMap((n) => {
      if (n.kind === "folder") {
        const children = mediaOnly(n.children);
        return children.length ? [{ ...n, children }] : [];
      }
      return n.browser === "audio" ? [] : [n];
    });
  }
  let nodes = $derived(mediaOnly(mediaState.mediaBins));
</script>

{#snippet tree(nodes: MediaBinNode[], depth: number)}
  {#each nodes as n (n.id ?? `f:${n.path.join("/")}`)}
    {#if n.kind === "folder"}
      {@const key = n.path.join("/")}
      {@const open = !!showView.expandedBinFolders[key]}
      <li>
        <button class="row" style:padding-left={`${6 + depth * 14}px`} aria-expanded={open} onclick={() => showView.toggleBinFolder(key)}>
          <span class="tw">{#if open}<ChevronDown size={12} />{:else}<ChevronRight size={12} />{/if}</span>
          {#if open}<FolderOpen size={15} />{:else}<Folder size={15} />{/if}
          <span class="label">{n.name}</span>
        </button>
        {#if open}<ul>{@render tree(n.children, depth + 1)}</ul>{/if}
      </li>
    {:else}
      {@const Icon = n.browser === "audio" ? Music : n.kind === "watched" ? FolderSync : Images}
      <li>
        <button
          class="row"
          class:selected={mediaState.mediaBinId === n.id}
          style:padding-left={`${6 + depth * 14}px`}
          onclick={() => {
            if (n.id) void mediaState.selectMediaBin(n.id);
          }}
          title={`${n.name} (${n.itemCount} item${n.itemCount === 1 ? "" : "s"}${n.kind === "watched" ? ", watched folder" : ""})`}
        >
          <span class="tw"></span>
          <Icon size={15} />
          <span class="label">{n.name}</span>
          <span class="count">{n.itemCount}</span>
        </button>
      </li>
    {/if}
  {/each}
{/snippet}

<ul class="list" aria-label="Media bins">
  {@render tree(nodes, 0)}
  {#if !nodes.length}<li class="empty">No media bins</li>{/if}
</ul>

<style>
  .list,
  .list :global(ul) {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .list {
    padding: 0 6px;
  }
  .row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding-right: 8px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
    text-align: left;
    font-size: 12px;
  }
  .row :global(svg) {
    flex: none;
    color: var(--text-label);
  }
  .row:hover {
    background: var(--surface-raised);
  }
  .row.selected {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .row.selected :global(svg) {
    color: var(--text-strong);
  }
  .tw {
    width: 12px;
    flex: none;
    display: grid;
    color: var(--text-label);
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    flex: none;
    color: var(--text-meta);
    font-size: 11px;
  }
  .empty {
    padding: 6px 10px;
    color: var(--text-label);
    font-size: 12px;
  }
</style>
