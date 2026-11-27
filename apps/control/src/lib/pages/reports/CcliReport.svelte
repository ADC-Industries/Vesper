<script lang="ts">
  // Reports ▸ CCLI usage (docs/MEDIA-PREP-PLAN.md): the songs this box showed live in a date
  // range, one use per service, with how often each was projected, streamed and recorded;
  // songs without a CCLI number flagged; Export CSV in the columns CCLI's reporting asks for.
  // Reporting itself is done on CCLI's site.
  import type { CcliReport } from "$protocol";
  import { Button, Select } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { errorText } from "../../state/errors";
  import { ReportsClient } from "../../server/mediaPrep";
  import { RANGES, ccliCsv, rangeOf, totals, type RangeName } from "./ccli";

  let rangeName = $state<RangeName>("half");
  let from = $state(rangeOf("half", new Date()).from);
  let to = $state(rangeOf("half", new Date()).to);
  let report = $state.raw<CcliReport | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(false);

  async function load() {
    const conn = session.conn;
    if (!conn) return;
    loading = true;
    error = null;
    try {
      const r = await new ReportsClient(conn.rest).ccli(from, to);
      if (session.conn === conn) report = r;
    } catch (e) {
      error = errorText(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void from;
    void to;
    void load();
  });
  function pick(v: RangeName) {
    rangeName = v;
    const r = rangeOf(v, new Date());
    from = r.from;
    to = r.to;
  }
  function exportCsv() {
    if (!report) return;
    const url = URL.createObjectURL(new Blob([ccliCsv(report)], { type: "text/csv" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = `ccli-usage-${report.from}-to-${report.to}.csv`;
    a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  let t = $derived(report ? totals(report) : null);
</script>

<section class="card" data-testid="ccli-report">
  <div class="head">
    <h2>CCLI usage</h2>
    <span class="grow"></span>
    <Select label="Range" options={RANGES} value={rangeName} onchange={(v) => pick(v)} style="width: 160px" />
    <label class="d">From <input type="date" bind:value={from} max={to} /></label>
    <label class="d">To <input type="date" bind:value={to} min={from} /></label>
    <Button size="sm" variant="primary" disabled={!report?.rows.length} onclick={exportCsv}>Export CSV</Button>
  </div>
  <p class="meta">Songs shown live on this box, counted once per service (the playlist they were shown from, or the day). Submit the counts on CCLI's site: this list and the CSV are what it asks for.</p>
  {#if error}
    <p class="err">Couldn't load the report: {error}</p>
  {:else if !report}
    <p class="meta">{loading ? "Loading…" : ""}</p>
  {:else if !report.rows.length}
    <p class="meta">No songs were shown live between {report.from} and {report.to}.</p>
  {:else}
    {#if t}<p class="sum">{t.songs} {t.songs === 1 ? "song" : "songs"} · {t.uses} {t.uses === 1 ? "use" : "uses"} in {t.services} {t.services === 1 ? "service" : "services"}{t.noNumber ? ` · ${t.noNumber} without a CCLI number` : ""}</p>{/if}
    <table>
      <thead>
        <tr><th>Song</th><th>CCLI #</th><th>Authors</th><th class="n">Used</th><th class="n">Projected</th><th class="n">Streamed</th><th class="n">Recorded</th><th>Last used</th></tr>
      </thead>
      <tbody>
        {#each report.rows as r, i (i)}
          <tr>
            <td>{r.title}</td>
            <td>{#if r.ccliNumber}{r.ccliNumber}{:else}<span class="warn" title="Add the CCLI number in the song's details to report it">none</span>{/if}</td>
            <td class="auth" title={r.authors}>{r.authors}</td>
            <td class="n">{r.count}</td>
            <td class="n">{r.projected}</td>
            <td class="n">{r.streamed}</td>
            <td class="n">{r.recorded}</td>
            <td>{r.lastUsed}</td>
          </tr>
        {/each}
      </tbody>
    </table>
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
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .grow {
    flex: 1;
  }
  h2 {
    margin: 0;
    font-size: var(--fs);
  }
  .d {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .meta,
  .sum {
    margin: 0;
    color: var(--text-meta);
    line-height: 1.45;
  }
  .sum {
    color: var(--text-2);
  }
  .err {
    margin: 0;
    color: var(--live);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-sm);
  }
  th {
    text-align: left;
    font-weight: 600;
    color: var(--text-label);
    padding: 6px 8px;
    border-bottom: 1px solid var(--line);
  }
  td {
    padding: 6px 8px;
    border-bottom: 1px solid var(--surface-raised);
    color: var(--text-2);
  }
  .n {
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .auth {
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .warn {
    color: var(--live);
  }
</style>
