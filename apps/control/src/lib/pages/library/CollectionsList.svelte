<script lang="ts">
  // The all-collections view's list (plan 4.2 "All collections view", round 4): every collection
  // of the Codex with its colour, description and size, then the collection trash (seen only
  // here). Selecting one opens it on the right (CollectionPanel).
  import Trash2 from "lucide-svelte/icons/trash-2";
  import { libraryPage } from "../../state/library.svelte";
  import { cssColor } from "../../model";
  import { formatBytes, plural, shortTime } from "./logic";
  import { syncedText } from "./sync";
</script>

<div class="list" role="listbox" aria-label="Collections">
  {#each libraryPage.collections as c (c.collection.id)}
    {@const id = c.collection.id}
    <button type="button" class="row" class:on={libraryPage.selected === id} role="option" aria-selected={libraryPage.selected === id} onclick={() => libraryPage.select(id)}>
      <span class="swatch" style:background={cssColor(c.collection.color, 1) ?? "var(--line-strong)"} aria-hidden="true"></span>
      <span class="main">
        <span class="name">{c.collection.name ?? "Untitled"}{#if c.active}<span class="here">shown here</span>{/if}</span>
        <span class="sub">{c.collection.description || `${plural(c.counts.presentations, "song")} · ${plural(c.counts.mediaAssets, "media file")}`}</span>
        {#if syncedText(c.boxes)}<span class="synced" class:behind={!syncedText(c.boxes)?.ok}>{syncedText(c.boxes)?.text}</span>{/if}
      </span>
      <span class="size">{formatBytes(c.documentBytes + c.mediaBytes)}</span>
    </button>
  {:else}
    <p class="empty">{libraryPage.loading ? "Loading…" : "No collections."}</p>
  {/each}

  <h3 class="cap"><Trash2 size={13} strokeWidth={1.8} aria-hidden="true" /> Collection trash</h3>
  {#each libraryPage.deletedCollections as d (d.collection.id)}
    {@const id = `trash:${d.collection.id}`}
    <button type="button" class="row deleted" class:on={libraryPage.selected === id} role="option" aria-selected={libraryPage.selected === id} onclick={() => libraryPage.select(id)}>
      <span class="swatch" style:background={cssColor(d.collection.color, 1) ?? "var(--line-strong)"} aria-hidden="true"></span>
      <span class="main">
        <span class="name">{d.collection.name ?? "Untitled"}</span>
        <span class="sub">Deleted {shortTime(d.deletedAt)}{d.by.client ? ` by ${d.by.client}` : ""} · {plural(d.counts.presentations, "song")}</span>
      </span>
      <span class="size">{formatBytes(d.documentBytes + d.mediaBytes)}</span>
    </button>
  {:else}
    <p class="empty small">Deleted collections wait here, with no time limit, until an admin restores them or deletes them for good.</p>
  {/each}
</div>

<style>
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    border-top: 1px solid var(--surface-raised);
  }
  .row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 12px;
    border: 0;
    border-bottom: 1px solid #1f2126;
    background: transparent;
    color: var(--text);
    text-align: left;
  }
  .synced {
    font-size: var(--fs-xs);
    color: var(--ok-text);
  }
  .synced.behind {
    color: var(--live);
  }
  .row:hover {
    background: var(--surface-raised);
  }
  .row.on {
    background: var(--accent-sel);
  }
  .row.deleted .name {
    color: var(--text-2);
  }
  .swatch {
    width: 12px;
    height: 12px;
    border-radius: 3px;
    flex: none;
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .here {
    margin-left: 8px;
    font-size: 10.5px;
    font-weight: 600;
    color: var(--accent-link);
  }
  .sub {
    font-size: var(--fs-xs);
    color: var(--text-meta);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .size {
    font-size: var(--fs-xs);
    color: var(--text-label);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .cap {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 18px 12px 6px;
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: var(--label-tracking);
    text-transform: uppercase;
    color: var(--text-label);
  }
  .empty {
    margin: 0;
    padding: 12px;
    color: var(--text-label);
  }
  .empty.small {
    font-size: var(--fs-xs);
    color: var(--text-meta);
    line-height: 1.4;
  }
</style>
