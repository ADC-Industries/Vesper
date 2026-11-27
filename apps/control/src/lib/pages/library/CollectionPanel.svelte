<script lang="ts">
  // The all-collections view's right part: a collection's name, colour and description (each
  // saved as it changes), its contents and size, "Show on this box" and "Delete" (into the
  // collection trash); a deleted collection's details with Restore and "Delete for good" (two
  // confirmations, the second naming it). Admin actions: the site's admins only (403 says so);
  // on a show box they go to the Codex box (plan 2.5).
  import Layers from "lucide-svelte/icons/layers";
  import RotateCcw from "lucide-svelte/icons/rotate-ccw";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Plus from "lucide-svelte/icons/plus";
  import type { Color } from "$protocol";
  import { Button, EmptyState } from "../../ui";
  import { cssColor } from "../../model";
  import { libraryPage } from "../../state/library.svelte";
  import { formatBytes, plural, shortTime } from "./logic";

  let { readOnly }: { readOnly: boolean } = $props();

  let sel = $derived(libraryPage.selected);
  let live = $derived(sel && !sel.startsWith("trash:") ? (libraryPage.collections.find((c) => c.collection.id === sel) ?? null) : null);
  let gone = $derived(sel?.startsWith("trash:") ? (libraryPage.deletedCollections.find((d) => `trash:${d.collection.id}` === sel) ?? null) : null);

  const SWATCHES = ["#6E5CE6", "#4FB487", "#F5A524", "#5B8DEF", "#E0607E", "#2FB8C6", "#B27BE0", "#8A8E9A"];
  const toColor = (hex: string): Color => ({ r: parseInt(hex.slice(1, 3), 16) / 255, g: parseInt(hex.slice(3, 5), 16) / 255, b: parseInt(hex.slice(5, 7), 16) / 255 });
  const same = (a: Color | undefined, hex: string) => !!a && cssColor(a, 1) === cssColor(toColor(hex), 1);

  // The fields keep what is being typed while another field's change is saved (no rebuild
  // when the revision changes); a change that failed shows the stored value again.
  async function rename(input: HTMLInputElement) {
    const name = input.value.trim();
    if (!live || !name || name === live.collection.name) return;
    if (!(await libraryPage.updateCollection(live, { name }, `Renaming ${live.collection.name ?? "the collection"}`))) input.value = live?.collection.name ?? "";
  }
  async function describe(input: HTMLTextAreaElement) {
    const v = input.value;
    if (!live || v === (live.collection.description ?? "")) return;
    if (!(await libraryPage.updateCollection(live, { description: v.trim() ? v : null }, "Changing the description"))) input.value = live?.collection.description ?? "";
  }
  function colour(hex: string | null) {
    if (!live) return;
    void libraryPage.updateCollection(live, { color: hex ? toColor(hex) : null }, "Changing the colour");
  }

  let counts = $derived(live?.counts ?? gone?.counts ?? null);
  let facts = $derived(
    counts
      ? [
          ["Songs", counts.presentations],
          ["Shelves", counts.libraries],
          ["Playlists", counts.playlists],
          ["Media files", counts.mediaAssets],
          ["Themes", counts.themes],
          ["Props", counts.props],
          ["Stage layouts", counts.stageLayouts],
        ]
      : [],
  );
  let storage = $derived(libraryPage.storage?.collections.find((c) => c.collection === (live?.collection.id ?? gone?.collection.id)) ?? null);
</script>

<main class="item" aria-label="Collection">
  <div class="head">
    <span class="kind">{gone ? "Deleted collection" : "Collection"}</span>
    <h2>{live?.collection.name ?? gone?.collection.name ?? "Collections"}</h2>
    <span class="meta">{live?.active ? "Shown on this box" : gone ? `Deleted ${shortTime(gone.deletedAt)}` : ""}</span>
    {#if !live && !gone}
      <Button size="sm" variant="outline" icon={Plus} disabled={readOnly || libraryPage.busy} onclick={() => libraryPage.createCollection()}>New collection</Button>
    {:else if live && !live.active}
      <Button size="sm" variant="outline" disabled={readOnly || libraryPage.busy} onclick={() => libraryPage.switchCollection(live!)}>Show on this box</Button>
    {:else if gone}
      <Button size="sm" variant="primary" icon={RotateCcw} disabled={readOnly || libraryPage.busy} onclick={() => libraryPage.restoreCollection(gone!)}>Restore</Button>
    {/if}
  </div>

  <div class="body">
    {#if live}
      {#key live.collection.id}
        <div class="form">
          <label class="kv">
            <span>Name</span>
            <input type="text" value={live.collection.name ?? ""} maxlength="200" disabled={readOnly || live.readOnly} onchange={(e) => void rename(e.currentTarget)} />
          </label>
          <div class="kv">
            <span>Colour</span>
            <span class="swatches" role="radiogroup" aria-label="Colour">
              {#each SWATCHES as hex (hex)}
                <button
                  type="button"
                  class="sw"
                  role="radio"
                  aria-checked={same(live.collection.color, hex)}
                  aria-label={hex}
                  style:background={hex}
                  disabled={readOnly || live.readOnly}
                  onclick={() => colour(hex)}
                ></button>
              {/each}
              <button type="button" class="none" role="radio" aria-checked={!live.collection.color} disabled={readOnly || live.readOnly} onclick={() => colour(null)}>None</button>
            </span>
          </div>
          <label class="kv top">
            <span>Description</span>
            <textarea rows="3" placeholder="What this collection is for (a campus, a ministry, a room)" disabled={readOnly || live.readOnly} onchange={(e) => void describe(e.currentTarget)}
              value={live.collection.description ?? ""}
            ></textarea>
          </label>
          <p class="saved">Changes are saved as you make them.</p>
        </div>
      {/key}
    {/if}

    {#if live || gone}
      <div class="facts">
        <h3 class="cap">In it</h3>
        <div class="grid">
          {#each facts as [label, n] (label)}
            <div class="fact"><span class="n">{n}</span><span class="l">{label}</span></div>
          {/each}
        </div>
        <h3 class="cap">On disk</h3>
        <div class="kv"><span>Documents</span><span>{formatBytes((live ?? gone)!.documentBytes)}</span></div>
        <div class="kv"><span>Media it uses</span><span>{formatBytes((live ?? gone)!.mediaBytes)}</span></div>
        {#if storage}
          <div class="kv"><span>Only it uses</span><span>{formatBytes(storage.uniqueMediaBytes)}</span></div>
          {#if storage.missing}<div class="kv"><span>Missing files</span><span class="warn">{storage.missing}</span></div>{/if}
        {/if}
        {#if live?.collection.createdAt}<div class="kv"><span>Created</span><span>{shortTime(live.collection.createdAt)}</span></div>{/if}
        {#if gone}
          <div class="kv"><span>Deleted by</span><span>{gone.by.client || "—"}</span></div>
          <div class="kv"><span>Deleted items</span><span>{plural(gone.trashItems, "item")}</span></div>
        {/if}
      </div>

      <div class="danger">
        {#if live}
          <Button size="sm" variant="danger" icon={Trash2} disabled={readOnly || live.active || libraryPage.busy} onclick={() => libraryPage.deleteCollection(live!)}>Delete collection…</Button>
          <span class="note">{live.active ? "This box shows it: show another collection first." : "It goes to the collection trash and can be restored."}</span>
        {:else if gone}
          <Button size="sm" variant="danger" icon={Trash2} disabled={readOnly || libraryPage.busy} onclick={() => libraryPage.purgeCollection(gone!)}>Delete for good…</Button>
          <span class="note">Its songs, playlists and change log go for good. Media files stay in the media library.</span>
        {/if}
      </div>
    {:else}
      <EmptyState icon={Layers} title="Collections in the Codex">
        Each collection holds the songs, media, themes and playlists of a campus, a ministry or a room. Pick one to rename it, colour it or describe it. Deleted collections wait in the collection
        trash.
      </EmptyState>
    {/if}
  </div>
</main>

<style>
  .item {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface-app);
  }
  .head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--surface-raised);
  }
  .kind {
    padding: 2px 8px;
    border-radius: 10px;
    background: var(--surface-raised);
    font-size: 10.5px;
    font-weight: 800;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-2);
    white-space: nowrap;
  }
  h2 {
    margin: 0;
    font-size: 18px;
    font-weight: 800;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    flex: 1;
    min-width: 0;
    color: var(--text-meta);
    font-size: var(--fs-sm);
    white-space: nowrap;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 18px;
    max-width: 640px;
  }
  .body > :global(.ui-empty) {
    margin: auto;
  }
  .form,
  .facts {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .kv {
    display: grid;
    grid-template-columns: 130px minmax(0, 1fr);
    gap: 10px;
    align-items: center;
    min-height: 28px;
    font-size: 12.5px;
  }
  .kv.top {
    align-items: start;
  }
  .kv > span:first-child {
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  input,
  textarea {
    width: 100%;
    box-sizing: border-box;
  }
  textarea {
    resize: vertical;
    font: inherit;
  }
  .swatches {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .sw {
    width: 22px;
    height: 22px;
    padding: 0;
    border-radius: 6px;
    border: 2px solid transparent;
  }
  .sw[aria-checked="true"] {
    border-color: var(--text-strong);
    box-shadow: 0 0 0 2px var(--accent-ring);
  }
  .none {
    padding: 2px 8px;
    border-radius: 6px;
    border: 1px solid var(--line-strong);
    background: none;
    color: var(--text-label);
    font-size: var(--fs-xs);
  }
  .none[aria-checked="true"] {
    border-color: var(--accent);
    color: var(--text);
  }
  .saved {
    margin: 0 0 0 140px;
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
  .cap {
    margin: 6px 0 2px;
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: var(--label-tracking);
    text-transform: uppercase;
    color: var(--text-label);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(110px, 1fr));
    gap: 8px;
  }
  .fact {
    display: flex;
    flex-direction: column;
    padding: 8px 10px;
    border-radius: var(--r-card);
    background: var(--surface-panel);
    border: 1px solid var(--line);
  }
  .n {
    font-size: 18px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .l {
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
  .warn {
    color: var(--live);
  }
  .danger {
    display: flex;
    align-items: center;
    gap: 10px;
    padding-top: 12px;
    border-top: 1px solid var(--surface-raised);
  }
  .note {
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
</style>
