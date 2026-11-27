<script lang="ts">
  // Site ▸ Codex, on the box that holds the Codex (G14): which Vespers show which collection,
  // and how far behind they are. The Codex learns it from the show boxes' requests (they ask
  // every 30 s, and at once after a change); a box not heard from for 10 minutes is greyed.
  import { StatusDot } from "../../ui";
  import { siteState } from "../../state/site.svelte";
  import { libraryPage } from "../../state/library.svelte";
  import { session } from "../../state/session.svelte";
  import { boxState, syncedText } from "../library/sync";
  import { formatWhen } from "./logic";

  let codex = $derived(siteState.codex);
  let boxes = $derived(codex?.boxes ?? []);
  let names = $derived(new Map(libraryPage.collections.map((c) => [c.collection.id, c.collection.name ?? "Untitled"])));
  let synced = $derived(syncedText(libraryPage.collections.find((c) => c.active)?.boxes));
  $effect(() => {
    if (!libraryPage.collections.length) void libraryPage.loadCollections();
  });
</script>

<h2 class="st-section">Vespers showing its collections</h2>
<div class="st-table-wrap">
  <table class="st" data-testid="codex-boxes">
    <thead><tr><th>Vesper</th><th>Shows</th><th>State</th><th>Last heard</th></tr></thead>
    <tbody>
      <tr>
        <td><span class="st-two"><span>{session.server?.name ?? "This server"}</span><span class="st-meta">Holds the Codex</span></span></td>
        <td>{codex?.collectionName ?? "–"}</td>
        <td><StatusDot status="ok" label="Up to date" /></td>
        <td class="st-meta">Now</td>
      </tr>
      {#each boxes as b (b.fingerprint)}
        {@const s = boxState(b)}
        <tr class:gone={!b.online}>
          <td>{b.name || "Unnamed box"}</td>
          <td>{names.get(b.collection) ?? "Another collection"}</td>
          <td><StatusDot status={s.status} label={s.text} /></td>
          <td class="st-meta">{formatWhen(b.lastSeen)}</td>
        </tr>
      {/each}
    </tbody>
  </table>
</div>
<p class="st-note">
  {#if synced}{synced.text} show “{codex?.collectionName}”.{/if}
  {boxes.length ? "" : "No show box has asked for a copy yet. Boxes that join the site appear here once they follow a collection."}
</p>

<style>
  .gone td {
    color: var(--text-label);
  }
</style>
