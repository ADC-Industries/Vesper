<script lang="ts">
  // Settings ▸ Bible: this computer's Bible defaults (Oriel preferences, also in the preferences
  // backup): the default translation, verse numbers on generated slides and the slide split.
  // Online providers (API.Bible, ESV) come later, with the church's key kept on the server.
  import { onMount } from "svelte";
  import { Segmented, Select, Switch } from "../../ui";
  import { bible } from "./bibleState.svelte";
  import { SPLITS, type Split } from "./bible";

  onMount(() => {
    if (!bible.ready) void bible.load();
  });
</script>

<section class="card" data-testid="bible-settings">
  <h2>Generated slides</h2>
  <label class="row">
    <span>Default translation</span>
    <Select
      options={bible.translations.length ? bible.translations.map((t) => ({ value: t.id, label: `${t.abbreviation} — ${t.name}` })) : [{ value: "", label: "No translations" }]}
      value={bible.translation}
      onchange={(v) => bible.setDefaultTranslation(v)}
      disabled={!bible.translations.length}
    />
  </label>
  <div class="row"><span>Verse numbers</span><Switch label="Verse numbers" checked={bible.verseNumbers} onchange={(c) => bible.setVerseNumbers(c)} /></div>
  <div class="row"><span>Verses per slide</span><Segmented label="Verses per slide" size="sm" options={SPLITS} value={bible.split} onchange={(v: Split) => bible.setSplit(v)} /></div>
  <span class="meta">Auto fits as many verses as the theme’s text box holds (a number is the most per slide); a verse too long for one slide continues on the next.</span>
</section>
<section class="card">
  <h2>Translations on this server</h2>
  {#if !bible.translations.length}
    <span class="meta">{bible.problem ?? "None installed."}</span>
  {:else}
    <dl>
      {#each bible.translations as t (t.id)}
        <dt>{t.abbreviation}</dt>
        <dd>{t.name} <span class="meta">· {t.license}{t.origin === "local" ? " · from the data folder" : ""}</span></dd>
      {/each}
    </dl>
  {/if}
  <span class="meta">Online translations (API.Bible, ESV) with the church's own key come in a later update.</span>
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
  .row > span:first-child {
    min-width: 140px;
    color: var(--text-label);
  }
  .meta {
    color: var(--text-meta);
  }
  dl {
    display: grid;
    grid-template-columns: 80px 1fr;
    gap: 6px 16px;
    margin: 0;
  }
  dt {
    color: var(--text-label);
  }
  dd {
    margin: 0;
  }
</style>
