<script lang="ts">
  // Settings ▸ CCLI: the church's CCLI licence number, the copyright line under a song's words
  // on or off and on which of its slides (the server's show settings, `PUT /settings/ccli`, the
  // site's admin; the line's layout and template stay as they are). When no number is set and
  // imported SongSelect files carried one, it is offered. Below, this computer's SongSelect
  // Downloads watcher (desktop app, off by default; state/songselectWatch.svelte.ts).
  import { onMount } from "svelte";
  import type { CcliDisplay, CcliLicense, CcliLicenseStatus } from "$protocol";
  import { Button, Select, Switch } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { sitePage } from "../../state/site.svelte";
  import { songSelectWatch } from "../../state/songselectWatch.svelte";
  import { SHOW_ON, licenceOffer, licenceValue, showOnValue } from "./ccli";

  let status = $state<CcliLicenseStatus | null>(null);
  let error = $state("");
  let number = $state("");
  let busy = $state(false);
  let offer = $derived(licenceOffer(status));
  let saved = $derived(status?.settings.licenseNumber ?? "");
  let dirty = $derived((licenceValue(number) ?? "") !== saved);

  async function load() {
    const rest = session.conn?.rest;
    if (!rest) return;
    try {
      status = await rest.ccliSettings();
      number = status.settings.licenseNumber ?? "";
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
  }

  async function save(change: Partial<CcliLicense>) {
    const rest = session.conn?.rest;
    if (!rest || !status) return;
    const next: CcliLicense = { ...status.settings, ...change };
    busy = true;
    try {
      const r = await sitePage.admin("Changing the CCLI settings", () => rest.saveCcliSettings(next));
      if (r) {
        status = r;
        number = r.settings.licenseNumber ?? "";
      }
    } finally {
      busy = false;
    }
  }

  onMount(() => void load());
</script>

<section class="card" data-testid="ccli-licence">
  <h2>CCLI licence</h2>
  {#if error}
    <p class="err">Could not read the CCLI settings: {error}</p>
  {:else if !status}
    <span class="meta">Loading…</span>
  {:else}
    <p class="meta">The church's CCLI Church Copyright Licence number, shown in the copyright line ("CCLI Licence #"). Vesper doesn't connect to CCLI.</p>
    <div class="row">
      <label class="lbl" for="ccli-number">Licence number</label>
      <input id="ccli-number" type="text" bind:value={number} maxlength="40" placeholder="Not set" autocomplete="off" />
      <Button size="sm" variant="primary" disabled={!dirty || busy} onclick={() => save({ licenseNumber: licenceValue(number) })}>Save</Button>
    </div>
    {#if offer}
      <div class="row offer" data-testid="ccli-offer">
        <span>Your imported SongSelect songs carry CCLI Licence # {offer}.</span>
        <Button size="sm" disabled={busy} onclick={() => save({ licenseNumber: offer! })}>Use {offer} from your imported songs</Button>
      </div>
    {/if}
  {/if}
</section>

{#if status}
  <section class="card" data-testid="ccli-line">
    <h2>Copyright line</h2>
    <p class="meta">A song with song details (Library ▸ the song's CCLI details) shows its copyright line, with its writers, publisher, CCLI song number and the licence number, over its slides.</p>
    <div class="row">
      <span class="lbl">Show the copyright line</span>
      <Switch label="Show the copyright line" checked={status.settings.enabled} disabled={busy} onchange={(on) => void save({ enabled: on })} data-testid="ccli-enabled" />
    </div>
    <div class="row">
      <label class="lbl" for="ccli-show-on">On which slides</label>
      <Select id="ccli-show-on" options={SHOW_ON} value={showOnValue(status.settings.showOn)} disabled={busy || !status.settings.enabled} onchange={(v: CcliDisplay) => void save({ showOn: v })} />
    </div>
  </section>
{/if}

<section class="card" data-testid="songselect-watch">
  <h2>SongSelect downloads</h2>
  {#if songSelectWatch.available}
    <div class="row">
      <span class="lbl">Watch my Downloads folder</span>
      <Switch label="Watch my Downloads folder for SongSelect songs" checked={songSelectWatch.enabled} onchange={(on) => songSelectWatch.set(on)} data-testid="songselect-watch-switch" />
    </div>
    <p class="meta">On this computer: when a song downloaded from SongSelect (lyrics, ChordPro or SongShow Plus file) lands in your Downloads folder, Oriel offers to import it. Nothing is imported until you say so.</p>
    {#if songSelectWatch.problem}<p class="err">The Downloads folder can't be watched: {songSelectWatch.problem}</p>{/if}
  {:else}
    <p class="meta">In the desktop app, Oriel can watch your Downloads folder and offer to import songs downloaded from SongSelect. In a browser, use Import ▸ Song from SongSelect…</p>
  {/if}
</section>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px;
    border-radius: var(--r-card);
    background: var(--surface-panel);
    border: 1px solid var(--line);
  }
  h2 {
    margin: 0;
    font-size: var(--fs);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
  }
  .row > .lbl {
    min-width: 180px;
    color: var(--text-label);
  }
  .row input {
    width: 180px;
  }
  .offer {
    padding: 8px 10px;
    border-radius: var(--r);
    background: var(--surface-raised);
  }
  .meta {
    color: var(--text-meta);
  }
  p {
    margin: 0;
  }
  .err {
    color: var(--danger-text);
  }
</style>
