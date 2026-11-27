<script lang="ts">
  // Settings ▸ Translation (docs/DUAL-LANGUAGE-PLAN.md §5): the server's machine translation
  // models, one per language pair (OPUS-MT, Helsinki-NLP), each with its licence and the
  // attribution it asks for, its size and where it stands; Download / Stop / Delete (the site's
  // admin; the server answers 403 to anyone else). A model also downloads on first use, from
  // Reflow ▸ Translate to…. Models are fetched from their publisher at a pinned revision and
  // checked (SHA-256); they are never part of an installer.
  import { onDestroy, onMount } from "svelte";
  import type { TranslationModel, TranslationModels } from "$protocol";
  import { Button } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { sitePage } from "../../state/site.svelte";
  import { COMMON_LANGUAGES } from "../../edit/language";

  let data = $state<TranslationModels | null>(null);
  let error = $state("");
  let busy = $state<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | null = null;
  /** The page was left: a request still in flight schedules nothing more. */
  let gone = false;

  const lang = (c: string) => COMMON_LANGUAGES.find((l) => l.code === c.split("-")[0])?.name ?? c;
  const mb = (b: number) => `${Math.round(b / 1_000_000)} MB`;
  let downloading = $derived((data?.models ?? []).some((m) => m.status === "downloading"));

  async function load() {
    const rest = session.conn?.rest;
    if (!rest) return;
    try {
      data = await rest.translationModels();
      error = "";
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    }
    if (timer) clearTimeout(timer);
    timer = null;
    if (gone) return;
    timer = setTimeout(() => void load(), downloading ? 1000 : 5000);
  }

  // The site's admin only: on a locked site this asks for the PIN or a sign-in and tries again.
  async function act(m: TranslationModel, what: "download" | "stop" | "delete") {
    const rest = session.conn?.rest;
    if (!rest) return;
    busy = m.id;
    try {
      if (what === "download") await sitePage.admin(`Downloading ${m.model}`, () => rest.downloadTranslationModel(m.id));
      else if (what === "stop") await sitePage.admin(`Stopping the download of ${m.model}`, () => rest.stopTranslationModel(m.id));
      else await sitePage.admin(`Deleting ${m.model}`, () => rest.deleteTranslationModel(m.id));
    } finally {
      busy = null;
      if (!gone) await load();
    }
  }

  onMount(() => void load());
  onDestroy(() => {
    gone = true;
    if (timer) clearTimeout(timer);
    timer = null;
  });
</script>

<section class="card" data-testid="translation-settings">
  <div class="head"><h2>Translation models</h2>{#if data}<span class="meta">Engine: {data.engine}</span>{/if}</div>
  <p class="meta">Reflow ▸ Second language ▸ Translate to… fills the second language with an offline machine translation, shown first and saved only when you keep it. Each language pair has its own model, downloaded from its publisher the first time it is used (or here). <strong>{data?.notice ?? "Machine translation: check it, and check your licence allows a translation of this song"}.</strong></p>
  {#if error}
    <p class="err">Could not read the models: {error}</p>
  {:else if !data}
    <span class="meta">Loading…</span>
  {:else}
    <table class="models">
      <thead><tr><th>Pair</th><th>Model and licence</th><th class="num">Size</th><th>Status</th><th></th></tr></thead>
      <tbody>
        {#each data.models as m (m.id)}
          <tr data-pair={m.id}>
            <td class="pairname">{lang(m.from)} → {m.to.map(lang).filter((v, i, a) => a.indexOf(v) === i).join(", ")}{#if m.to.length > 1}<div class="meta small">{m.to.join(", ")}</div>{/if}</td>
            <td>
              <div><a href={m.card} target="_blank" rel="noreferrer">{m.model}</a> · <span class="lic">{m.licence}</span></div>
              <div class="meta small">{m.attribution}</div>
            </td>
            <td class="num">{mb(m.size)}</td>
            <td class="status">
              {#if m.status === "ready"}<span class="ok">Downloaded</span>
              {:else if m.status === "downloading"}
                <span>Downloading {mb(m.downloaded ?? 0)}</span>
                <progress max={m.size} value={m.downloaded ?? 0}></progress>
              {:else if m.status === "failed"}<span class="err" title={m.detail ?? undefined}>Failed</span><div class="meta small">{m.detail}</div>
              {:else}<span class="meta">Not downloaded</span>{/if}
            </td>
            <td class="acts">
              {#if m.status === "downloading"}
                <Button size="sm" disabled={busy === m.id} onclick={() => act(m, "stop")}>Stop</Button>
              {:else if m.status === "ready"}
                <Button size="sm" disabled={busy === m.id} onclick={() => act(m, "delete")}>Delete</Button>
              {:else}
                <Button size="sm" disabled={busy === m.id} onclick={() => act(m, "download")}>{m.status === "failed" ? "Retry" : "Download"}</Button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
    <p class="meta small">OPUS-MT models by Helsinki-NLP (University of Helsinki; Tiedemann &amp; Thottingal, "OPUS-MT — Building open translation services for the World", EAMT 2020), under the licence shown for each. Changing models needs the site's admin.</p>
  {/if}
</section>

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border-radius: var(--r-card);
    background: var(--surface-panel);
    border: 1px solid var(--line);
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: var(--fs);
  }
  .meta {
    color: var(--text-meta);
  }
  p.meta {
    margin: 0;
    line-height: 1.45;
  }
  .small {
    font-size: var(--fs-xs);
  }
  .err {
    color: var(--live);
  }
  .ok {
    color: var(--ok, #5cc98a);
  }
  .models {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-sm);
  }
  .models th {
    text-align: left;
    color: var(--text-label);
    font-weight: 600;
    padding: 4px 8px;
    border-bottom: 1px solid var(--line);
  }
  .models td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--line);
    vertical-align: top;
  }
  .num {
    text-align: right;
    white-space: nowrap;
  }
  .pairname {
    white-space: nowrap;
  }
  .lic {
    font-weight: 600;
  }
  .status progress {
    display: block;
    width: 120px;
  }
  .acts {
    text-align: right;
  }
  a {
    color: var(--accent-text, #c9c1ff);
  }
</style>
