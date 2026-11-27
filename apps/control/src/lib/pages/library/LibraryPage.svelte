<script lang="ts">
  // The Library page (docs/ORIEL-REDESIGN-PLAN.md 4.2, the board "Library"), round 3: browsing
  // the Codex. Three parts: where (collection, tree, connection), what (the folder's items) and
  // the item. The page toolbar holds the "Add to" playlist picker. Round 4: uploads, relinking,
  // the all-collections view (its own right part, CollectionPanel) and a song in Edit.
  import { onMount } from "svelte";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import type { Connection } from "../../server";
  import PageFrame from "../PageFrame.svelte";
  import WherePanel from "./WherePanel.svelte";
  import ItemsPanel from "./ItemsPanel.svelte";
  import ItemPanel from "./ItemPanel.svelte";
  import CollectionPanel from "./CollectionPanel.svelte";
  import RelinkDialog from "./RelinkDialog.svelte";
  import { Select } from "../../ui";
  import { flatPlaylists, libraryPage } from "../../state/library.svelte";
  import { session } from "../../state/session.svelte";
  import { siteState } from "../../state/site.svelte";
  import { presence } from "../../state/presence.svelte";
  import { connectionCard } from "./logic";
  import { binsPage } from "../../state/bins.svelte";
  import { binDrag } from "./binDrag.svelte";
  import { mediaPrep } from "../../state/mediaPrep.svelte";

  let { conn }: { conn: Connection } = $props();

  // svelte-ignore state_referenced_locally
  const status = conn.status;
  let card = $derived(connectionCard(siteState.codex, $status.phase === "online", session.server?.name ?? "the server"));

  onMount(() => {
    const off = libraryPage.attach();
    const offBins = binsPage.attach();
    const offPrep = mediaPrep.watch();
    return () => {
      off();
      offBins();
      offPrep();
      // Leaving the page ends Edit: Show gets back what it had open.
      void libraryPage.endEdit();
    };
  });

  // Back online: waiting uploads go on at once.
  $effect(() => {
    if ($status.phase === "online") libraryPage.resumeUploads();
  });

  // The box switched collections (here or from another Oriel): reload the lists.
  $effect(() => {
    void siteState.codex?.collection;
    libraryPage.followCollection();
  });

  // Presence (G8): this page has the box's collection open ("2 others have this collection
  // open" on the connection card).
  $effect(() => {
    const c = siteState.codex?.collection;
    presence.setOpen("collection", c ? [{ kind: "collection", id: c, place: "library" }] : null);
  });
  $effect(() => () => presence.setOpen("collection", null));

  let playlists = $derived(flatPlaylists(session.playlistTree));
  let options = $derived(playlists.map((p) => ({ value: p.id, label: p.path.length ? `${p.path.join(" › ")} › ${p.name}` : p.name })));
  let target = $state("");
  $effect(() => {
    target = libraryPage.target ?? "";
  });
</script>

<PageFrame page="library" subtitle={siteState.codex?.collectionName}>
  {#snippet controls()}
    {#if libraryPage.managedOn}
      <span class="managed" title="Uploads and collection management go to this box">Managed on {libraryPage.managedOn.name ?? siteState.codex?.sync?.name ?? "the site's Codex box"}</span>
    {/if}
    <span class="addto">Add to</span>
    {#if options.length}
      <Select label="Playlist that Add to uses" {options} bind:value={target} onchange={(v) => (libraryPage.target = v)} style="width: 220px" />
    {:else}
      <span class="none">No playlists yet</span>
    {/if}
  {/snippet}
  <div class="library">
    {#if card.readOnly && card.status === "warn"}
      <div class="amber" role="status">
        <TriangleAlert size={16} strokeWidth={1.8} aria-hidden="true" />
        Editing needs the site's server ({libraryPage.managedOn?.name ?? siteState.codex?.sync?.name ?? "the Codex box"}). The show keeps playing.
      </div>
    {/if}
    <div class="parts">
      <WherePanel {card} />
      <ItemsPanel {conn} readOnly={card.readOnly} />
      {#if libraryPage.view.kind === "collections"}
        <CollectionPanel readOnly={card.readOnly} />
      {:else}
        <ItemPanel {conn} readOnly={card.readOnly} />
      {/if}
    </div>
    <RelinkDialog {conn} />
    {#if binDrag.files}
      <div class="ghost" style:left={`${binDrag.x + 14}px`} style:top={`${binDrag.y + 10}px`} aria-hidden="true">
        {binDrag.files.length === 1 ? (binDrag.files[0]!.asset.name ?? "1 file") : `${binDrag.files.length} files`}{binDrag.over ? "" : " · drop on a bin"}
      </div>
    {/if}
  </div>
</PageFrame>

<style>
  .library {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .parts {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .amber {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 16px;
    background: var(--live-soft);
    border-bottom: 1px solid rgba(245, 165, 36, 0.35);
    color: var(--live);
    font-size: 12.5px;
    font-weight: 600;
  }
  .ghost {
    position: fixed;
    z-index: 50;
    pointer-events: none;
    padding: 4px 10px;
    border-radius: var(--r);
    background: var(--surface-overlay);
    border: 1px solid var(--accent);
    color: var(--text-strong);
    font-size: var(--fs-sm);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.45);
  }
  .addto {
    font-size: var(--fs-sm);
    color: var(--text-meta);
  }
  .none {
    font-size: var(--fs-sm);
    color: var(--text-label);
  }
  .managed {
    padding: 3px 9px;
    border-radius: var(--r-pill);
    background: var(--accent-sel-2);
    border: 1px solid var(--accent);
    color: var(--accent-link);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
</style>
