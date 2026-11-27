<script lang="ts">
  // Relinking a missing media file (plan 4.2, round 4): pick any item of the media library
  // (`GET /codex/library`, of the file's kind; where the box can't list it, e.g. a show box, the
  // collection's files that are online), let the server look for it, or upload the file from
  // this computer (an upload with `replaces`). Also the answer of "Find missing files" and of a
  // folder match.
  import Upload from "lucide-svelte/icons/upload";
  import SearchCheck from "lucide-svelte/icons/search-check";
  import type { CollectionMedia, LibraryItem } from "$protocol";
  import type { Connection } from "../../server";
  import { CodexClient, assetBlob } from "../../server/codex";
  import { Button, Dialog } from "../../ui";
  import { libraryPage } from "../../state/library.svelte";
  import { matchesFilter, mediaKind } from "./logic";

  let { conn }: { conn: Connection } = $props();

  let m = $derived(libraryPage.relinkFor);
  let q = $state("");
  let pick = $state<string | null>(null);
  let fileEl = $state<HTMLInputElement | null>(null);

  let codex = $derived(new CodexClient(conn.rest));

  /** A file to pick: a library item (its ID), with its picture and name. */
  interface Candidate {
    id: string;
    name: string;
    detail: string | null;
    thumb: string | null;
  }

  // Every item of the media library of the missing file's kind (null: the box can't list it).
  let library = $state.raw<LibraryItem[] | null>(null);
  $effect(() => {
    const a = m?.asset;
    library = null;
    if (!a) return;
    const k = mediaKind(a);
    const kind = k === "video" ? "video" : k === "audio" ? "sound" : "picture";
    let gone = false;
    codex
      .libraryItems({ kind, limit: 2000 })
      .then((r) => {
        if (!gone) library = r.items;
      })
      .catch(() => {
        /* (a show box: the collection's own files below) */
      });
    return () => {
      gone = true;
    };
  });

  function ending(path: string): string {
    return /\.([A-Za-z0-9]{1,8})$/.exec(path)?.[1]?.toLowerCase() ?? "bin";
  }

  let candidates = $derived.by((): Candidate[] => {
    if (!m) return [];
    const own = m.asset.location?.type === "library" ? m.asset.location.id : null;
    if (library)
      return library
        .filter((i) => i.id !== own && matchesFilter(i.path, q))
        .map((i) => ({
          id: i.id,
          name: i.title || i.path,
          detail: i.path,
          thumb: i.sha256 ? codex.blobThumbnailUrl(i.sha256, ending(i.path)) : null,
        }));
    return libraryPage.media
      .filter((x) => x.availability === "online" && x.asset.id !== m.asset.id && x.asset.location?.type === "library" && matchesFilter(x.asset.name ?? "", q))
      .map((x) => ({ id: x.asset.location?.type === "library" ? x.asset.location.id : x.asset.id, name: x.asset.name ?? "", detail: null, thumb: thumb(x) }));
  });
  let chosen = $derived(candidates.find((x) => x.id === pick) ?? null);
  function thumb(x: CollectionMedia): string {
    const b = assetBlob(x.asset);
    return b ? codex.blobThumbnailUrl(b.sha256, b.ext) : conn.rest.mediaThumbnailUrl(x.asset.id, 320);
  }

  function close() {
    libraryPage.relinkFor = null;
    pick = null;
    q = "";
  }

  function relink() {
    if (!m || !chosen) return;
    void libraryPage.relinkTo(m, { type: "library", id: chosen.id }, `“${chosen.name}”`);
  }

  function uploaded(e: Event & { currentTarget: HTMLInputElement }) {
    const f = e.currentTarget.files?.[0];
    e.currentTarget.value = "";
    if (!f || !m) return;
    libraryPage.importFiles([{ file: f, replaces: m.asset.id }]);
    close();
  }

  let origin = $derived.by(() => {
    const a = m?.asset;
    if (!a) return null;
    return a.origin?.absoluteHint ?? a.origin?.relativePath ?? (a.location?.type === "linked" ? a.location.path : null);
  });
</script>

<input bind:this={fileEl} type="file" accept="image/*,video/*,audio/*" hidden onchange={uploaded} />

{#if m}
  <Dialog title={`Relink “${m.asset.name}”`} onclose={close} width={640}>
    <div class="relink">
      <p class="why">
        {m.problem ?? "Its file can't be found on this box."}
        {#if origin}<span class="origin">Last known place: {origin}</span>{/if}
      </p>
      <div class="ways">
        <Button size="sm" variant="secondary" icon={SearchCheck} disabled={libraryPage.busy} onclick={() => libraryPage.findMissing([m!.asset.id]).then(close)}>Find it automatically</Button>
        <Button size="sm" variant="secondary" icon={Upload} onclick={() => fileEl?.click()}>Upload it from this computer…</Button>
      </div>
      <h3 class="cap">Or pick a file of the media library</h3>
      <input type="search" placeholder="Filter by name" bind:value={q} aria-label="Filter the media library" />
      <div class="grid" role="listbox" aria-label="Media library">
        {#each candidates as x (x.id)}
          <button type="button" class="tile" class:on={pick === x.id} role="option" aria-selected={pick === x.id} title={x.detail ?? x.name} onclick={() => (pick = x.id)} ondblclick={relink}>
            <span class="thumb">{#if x.thumb}<img src={x.thumb} alt="" loading="lazy" draggable="false" />{/if}</span>
            <span class="tname">{x.name}</span>
            {#if x.detail}<span class="tpath">{x.detail}</span>{/if}
          </button>
        {:else}
          <p class="empty">{q ? `No file matches “${q}”.` : library ? "The media library has no other file of this kind." : "No other files in this collection yet."}</p>
        {/each}
      </div>
    </div>
    {#snippet footer()}
      <Button variant="secondary" onclick={close}>Cancel</Button>
      <Button variant="primary" disabled={!chosen || libraryPage.busy} onclick={relink}>Use this file</Button>
    {/snippet}
  </Dialog>
{/if}

{#if libraryPage.relinkReport}
  {@const r = libraryPage.relinkReport}
  <Dialog title={r.title} onclose={() => (libraryPage.relinkReport = null)} width={460}>
    {#each r.lines as line, i (i)}<p class="line">{line}</p>{/each}
    {#snippet footer()}
      <Button variant="primary" onclick={() => (libraryPage.relinkReport = null)}>OK</Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .relink {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .why {
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
    color: var(--text-2);
  }
  .origin {
    font-size: var(--fs-xs);
    color: var(--text-meta);
    font-family: var(--mono);
    word-break: break-all;
  }
  .ways {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .cap {
    margin: 6px 0 0;
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: var(--label-tracking);
    text-transform: uppercase;
    color: var(--text-label);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(140px, 1fr));
    gap: 8px;
    max-height: 300px;
    overflow-y: auto;
    padding: 2px;
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px;
    border-radius: 7px;
    border: 2px solid transparent;
    background: var(--surface-panel);
    color: var(--text);
    text-align: left;
    min-width: 0;
  }
  .tile:hover {
    border-color: var(--line-strong);
  }
  .tile.on {
    border-color: var(--accent);
  }
  .thumb {
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
  .tname {
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tpath {
    font-size: 10.5px;
    color: var(--text-meta);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .empty {
    grid-column: 1 / -1;
    margin: 0;
    color: var(--text-label);
  }
  .line {
    margin: 0 0 6px;
  }
</style>
