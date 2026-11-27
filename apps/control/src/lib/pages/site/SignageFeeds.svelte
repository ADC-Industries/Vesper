<script lang="ts">
  // Signage feeds and settings. Feeds (read by IDN's feed reader on the site's server): each with
  // its address, last read and rows; add or edit one (a published Google Sheet's link, a CSV,
  // TSV, JSON, XML or RSS address, or a file in the server's signage-feeds folder), preview its rows and map columns to
  // a feed slide's title, text and time. Settings: the weather's place (found by name through
  // Open-Meteo's geocoding), °C/°F, and the weekly service times the countdown counts to.
  import type { SignageFeed, SignageFeedPreview, SignagePlace, SignageServiceTime, SignageSettings, SignageWeather } from "$protocol";
  import { Button, Select, StatusDot, Switch } from "../../ui";
  import { signagePage } from "../../state/signage.svelte";
  import { DAYS, feedStatus, hhmm, parseHhmm } from "./signage";

  let { feeds, settings, weather }: { feeds: SignageFeed[]; settings: SignageSettings; weather: SignageWeather | null } = $props();

  const blank = (): SignageFeed => ({ id: "", name: "", url: "", format: "auto", refresh: 900, mapping: { title: "", body: "", when: "" }, columns: [], rows: [] });
  let edit = $state<SignageFeed | null>(null);
  let preview = $state<SignageFeedPreview | null>(null);
  const FORMATS = ["auto", "csv", "tsv", "json", "xml"].map((v) => ({ value: v, label: v === "auto" ? "Automatic" : v === "xml" ? "XML / RSS" : v.toUpperCase() }));
  const REFRESH = [
    { value: "300", label: "Every 5 minutes" },
    { value: "900", label: "Every 15 minutes" },
    { value: "3600", label: "Every hour" },
    { value: "86400", label: "Once a day" },
  ];
  let columns = $derived(preview?.columns ?? edit?.columns ?? []);
  let colChoices = $derived([{ value: "", label: "—" }, ...columns.map((c) => ({ value: c, label: c }))]);
  async function doPreview() {
    if (!edit?.url.trim()) return;
    preview = (await signagePage.previewFeed({ ...edit, name: edit.name || "Preview" })) ?? null;
  }
  async function save() {
    if (!edit) return;
    const r = await signagePage.saveFeed(edit);
    if (r) {
      edit = null;
      preview = null;
    }
  }

  // Settings.
  let q = $state("");
  let found = $state<SignagePlace[]>([]);
  async function search() {
    found = (await signagePage.places(q))?.places ?? [];
  }
  function saveSettings(patch: Partial<SignageSettings>) {
    void signagePage.putSettings({ ...settings, ...patch });
  }
  function setService(i: number, patch: Partial<SignageServiceTime>) {
    saveSettings({ services: settings.services.map((s, j) => (j === i ? { ...s, ...patch } : s)) });
  }
  const dayChoices = DAYS.map((d, i) => ({ value: String(i), label: d }));
</script>

<div class="st-card" data-testid="signage-feeds">
  <div class="st-row">
    <h2>Feeds</h2>
    <span class="st-meta">Live data for feed slides and the ticker: a published Google Sheet, CSV, TSV, JSON, XML or RSS.</span>
    <span class="st-grow"></span>
    <Button size="sm" onclick={() => ((edit = blank()), (preview = null))}>Add feed</Button>
  </div>
  {#if feeds.length}
    <div class="st-table-wrap">
      <table class="st">
        <thead><tr><th>Feed</th><th>Address</th><th>Last read</th><th></th></tr></thead>
        <tbody>
          {#each feeds as f (f.id)}
            {@const s = feedStatus(f, Date.now())}
            <tr data-testid="signage-feed">
              <td class="st-strong">{f.name}</td>
              <td class="st-mono addr" title={f.url}>{f.url}</td>
              <td><StatusDot status={s.status} label={s.label} /></td>
              <td class="actions">
                <Button size="sm" onclick={() => ((edit = JSON.parse(JSON.stringify(f))), (preview = f.rows.length ? { columns: f.columns, rows: f.rows.slice(0, 20) } : null))}>Edit</Button>
                <Button size="sm" variant="ghost" onclick={() => signagePage.deleteFeed(f.id)}>Delete</Button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {:else if !edit}
    <p class="st-empty">No feed yet.</p>
  {/if}
  {#if edit}
    <div class="feedform" data-testid="signage-feed-form">
      <div class="st-row">
        <label>Name<input type="text" bind:value={edit.name} placeholder="Events" /></label>
        <label class="grow">Address or file<input type="text" bind:value={edit.url} placeholder="https://docs.google.com/spreadsheets/d/…/pub?output=csv" /></label>
        <label>Format<Select label="Format" options={FORMATS} bind:value={edit.format} /></label>
        <label>Refresh<Select label="Refresh" options={REFRESH} value={String(edit.refresh)} onchange={(v) => edit && (edit.refresh = Number(v))} /></label>
        <Button size="sm" onclick={doPreview} disabled={!edit.url.trim()}>Preview</Button>
      </div>
      <p class="st-meta">A file is read from the <code>signage-feeds</code> folder of the server's data directory: give its name there ("events.csv").</p>
      {#if columns.length}
        <div class="st-row">
          <span class="st-meta">Map columns:</span>
          <label>Title<Select label="Title column" options={colChoices} bind:value={edit.mapping.title} /></label>
          <label>Text<Select label="Text column" options={colChoices} bind:value={edit.mapping.body} /></label>
          <label>When<Select label="When column" options={colChoices} bind:value={edit.mapping.when} /></label>
        </div>
      {/if}
      {#if preview}
        <div class="st-table-wrap preview" data-testid="signage-feed-preview">
          <table class="st">
            <thead><tr>{#each preview.columns as c (c)}<th class:mapped={c === edit.mapping.title || c === edit.mapping.body || c === edit.mapping.when}>{c}</th>{/each}</tr></thead>
            <tbody>
              {#each preview.rows.slice(0, 6) as r, i (i)}
                <tr>{#each preview.columns as c (c)}<td>{r[c] ?? ""}</td>{/each}</tr>
              {/each}
            </tbody>
          </table>
        </div>
        <span class="st-meta">{preview.rows.length} row{preview.rows.length === 1 ? "" : "s"} read{preview.rows.length > 6 ? " (the first 6 shown)" : ""}.</span>
      {/if}
      <div class="st-row">
        <span class="st-grow"></span>
        <Button size="sm" onclick={() => ((edit = null), (preview = null))}>Cancel</Button>
        <Button size="sm" variant="primary" onclick={save} disabled={!edit.name.trim() || !edit.url.trim()}>Save feed</Button>
      </div>
    </div>
  {/if}
</div>

<div class="st-card" data-testid="signage-settings">
  <div class="st-row"><h2>Clock, countdown and weather</h2></div>
  <div class="st-row">
    <span class="lbl">Weather for</span>
    {#if settings.place}
      <span class="st-strong">{settings.place.name}</span>
      <span class="st-meta">{settings.place.latitude.toFixed(2)}, {settings.place.longitude.toFixed(2)}</span>
      {#if weather}<span>{Math.round(weather.temperature)}{settings.fahrenheit ? "°F" : "°C"} · {weather.summary}</span>{:else}<span class="st-meta">No forecast yet (hidden on the screens until one is read)</span>{/if}
      <Button size="sm" variant="ghost" onclick={() => saveSettings({ place: undefined })}>No weather</Button>
    {:else}
      <span class="st-meta">Not set</span>
    {/if}
    <input type="text" placeholder="Find a town or city" bind:value={q} onkeydown={(e) => e.key === "Enter" && search()} aria-label="Find a place" />
    <Button size="sm" onclick={search} disabled={q.trim().length < 2}>Find</Button>
    <Switch label="°F" showLabel checked={settings.fahrenheit} onchange={(v) => saveSettings({ fahrenheit: v })} />
  </div>
  {#if found.length}
    <div class="st-row">
      {#each found as p (p.name + p.latitude)}
        <Button size="sm" onclick={() => { saveSettings({ place: p }); found = []; q = ""; }}>{p.name}</Button>
      {/each}
    </div>
  {/if}
  <span class="st-meta">Weather data by Open-Meteo.com (CC BY 4.0), shown with the weather on every screen.</span>
  <div class="st-row">
    <span class="lbl">Services</span>
    {#each settings.services as s, i (i)}
      <span class="svc">
        <Select label="Day" options={dayChoices} value={String(s.day)} onchange={(v) => setService(i, { day: Number(v) })} />
        <input type="time" value={hhmm(s.minute)} onchange={(e) => { const m = parseHhmm(e.currentTarget.value); if (m !== null && m < 1440) setService(i, { minute: m }); }} aria-label="Time" />
        <input type="text" value={s.name} placeholder="Name" onchange={(e) => setService(i, { name: e.currentTarget.value })} aria-label="Name" />
        <Button size="sm" variant="ghost" onclick={() => saveSettings({ services: settings.services.filter((_, j) => j !== i) })} aria-label="Remove">✕</Button>
      </span>
    {/each}
    <Button size="sm" onclick={() => saveSettings({ services: [...settings.services, { day: 6, minute: 600, name: "Sunday service" }] })}>Add service time</Button>
  </div>
</div>

<style>
  .addr {
    max-width: 360px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .feedform {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px;
    border: 1px solid var(--line);
    border-radius: var(--r);
  }
  .feedform label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: var(--fs-sm);
    color: var(--text-label);
  }
  .feedform label.grow {
    flex: 1;
    min-width: 280px;
  }
  .preview {
    max-height: 220px;
    overflow: auto;
  }
  th.mapped {
    color: var(--accent);
  }
  .lbl {
    min-width: 90px;
    color: var(--text-label);
  }
  .svc {
    display: inline-flex;
    gap: 4px;
    align-items: center;
  }
</style>
