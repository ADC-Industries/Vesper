<script lang="ts">
  // The server's diagnostics (`GET /diagnostics`) in the redesign's look: the renderer and
  // uptime on top, then one check card per area (missing media, fonts, and what isn't
  // supported or couldn't be loaded when there is any), each with its state and its entries.
  import { onMount } from "svelte";
  import type { DiagnosticsReport } from "$protocol";
  import Dialog from "../ui/Dialog.svelte";
  import Badge from "../ui/Badge.svelte";
  import type { Connection } from "../server";
  import { shell } from "../state/shell.svelte";

  let { conn }: { conn: Connection } = $props();

  let report = $state.raw<DiagnosticsReport | null>(null);
  let error = $state<string | null>(null);

  onMount(() => {
    conn.rest
      .diagnostics()
      .then((r) => (report = r))
      .catch((e: unknown) => (error = e instanceof Error ? e.message : String(e)));
  });

  function uptime(s: number): string {
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    return h ? `${h} h ${m} min` : `${m} min`;
  }

  let missing = $derived(report ? report.missingMedia.reduce((a, b) => a + b.count, 0) : 0);
</script>

{#snippet check(title: string, count: number, tone: "ok" | "warn" | "error", state: string)}
  <div class="check-head">
    <span class="glyph {tone}" aria-hidden="true">{tone === "ok" ? "✓" : "!"}</span>
    <span class="name">{title}</span>
    <span class="state {tone}">· {state}</span>
    <span class="grow"></span>
    <Badge {count} tone={tone === "error" ? "error" : "neutral"} label="entries" />
  </div>
{/snippet}

<Dialog title="Diagnostics" onclose={() => (shell.showDiagnostics = false)} width={680}>
  {#if error}
    <p class="err">Could not load diagnostics: {error}</p>
  {:else if !report}
    <p class="meta">Loading…</p>
  {:else}
    <div class="diag">
      <dl class="facts">
        <div><dt>Renderer</dt><dd>{report.adapter ?? "not running"}</dd></div>
        <div><dt>Up for</dt><dd>{uptime(report.uptimeSeconds)}</dd></div>
      </dl>

      <section class="check" aria-label="Missing media">
        {@render check("Missing media", missing, missing ? "warn" : "ok", missing ? `${missing} missing` : "None")}
        {#if report.missingMedia.length}
          <ul>
            {#each report.missingMedia as m, i (i)}
              <li><span class="what">{m.what}</span>{#if m.detail}<span class="detail">{m.detail}</span>{/if}<span class="c">×{m.count}</span></li>
            {/each}
          </ul>
        {/if}
      </section>

      <section class="check" aria-label="Fonts">
        {@render check("Fonts", report.fonts.length, report.fonts.length ? "warn" : "ok", report.fonts.length ? `${report.fonts.length} replaced` : "All fonts found")}
        {#if report.fonts.length}
          <ul>
            {#each report.fonts as f, i (i)}
              <li>
                <span class="what">{f.family}{f.weight !== 400 ? ` ${f.weight}` : ""}{f.italic ? " italic" : ""}</span>
                <span class="detail">→ {f.usedFamily} ({f.match})</span>
                <span class="c">×{f.uses}</span>
              </li>
            {/each}
          </ul>
        {/if}
      </section>

      {#if report.unsupported.length}
        <section class="check" aria-label="Not supported yet">
          {@render check("Not supported yet", report.unsupported.length, "warn", `${report.unsupported.length} kinds`)}
          <ul>
            {#each report.unsupported as m, i (i)}
              <li><span class="what">{m.what}</span>{#if m.detail}<span class="detail">{m.detail}</span>{/if}<span class="c">×{m.count}</span></li>
            {/each}
          </ul>
        </section>
      {/if}

      {#if report.loadIssues.length}
        <section class="check" aria-label="Files that could not be loaded">
          {@render check("Files that could not be loaded", report.loadIssues.length, "error", `${report.loadIssues.length} files`)}
          <ul>
            {#each report.loadIssues as m, i (i)}
              <li><span class="what">{m.file}</span><span class="detail">{m.error}{m.tooNew ? " (made by a newer version)" : ""}</span></li>
            {/each}
          </ul>
        </section>
      {/if}
    </div>
  {/if}
</Dialog>

<style>
  .diag {
    display: flex;
    flex-direction: column;
    gap: var(--sp-5);
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-4) 28px;
    margin: 0;
    padding: 10px 12px;
    border-radius: var(--r-card);
    background: var(--surface-sunk);
    border: 1px solid var(--line);
  }
  .facts div {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  dt {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: var(--label-tracking);
    text-transform: uppercase;
    color: var(--text-label);
  }
  dd {
    margin: 0;
    user-select: text;
  }
  .check {
    display: flex;
    flex-direction: column;
    border-radius: var(--r-card);
    background: var(--surface-raised);
    border: 1px solid var(--line);
    overflow: hidden;
  }
  .check-head {
    display: flex;
    align-items: center;
    gap: var(--sp-5);
    padding: 10px 12px;
  }
  .glyph {
    flex: none;
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    font-size: var(--fs-sm);
    font-weight: 800;
  }
  .glyph.ok {
    background: var(--ok-bg);
    color: var(--ok-text);
  }
  .glyph.warn {
    background: var(--live-soft);
    color: var(--live);
  }
  .glyph.error {
    background: var(--danger-soft);
    color: var(--danger-text);
  }
  .name {
    font-weight: 600;
  }
  .state {
    color: var(--text-label);
  }
  .state.ok {
    color: var(--ok-text);
  }
  .state.warn {
    color: var(--live);
  }
  .state.error {
    color: var(--danger-text);
  }
  .grow {
    flex: 1;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 220px;
    overflow-y: auto;
    border-top: 1px solid var(--line);
    background: var(--surface-panel);
  }
  li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    column-gap: var(--sp-5);
    row-gap: 2px;
    padding: 6px 12px 6px 42px;
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-sm);
    user-select: text;
  }
  li:last-child {
    border-bottom: 0;
  }
  .what {
    font-weight: 600;
    word-break: break-all;
  }
  .detail {
    grid-column: 1;
    grid-row: 2;
    color: var(--text-2);
    word-break: break-all;
  }
  .c {
    grid-column: 2;
    grid-row: 1;
    color: var(--text-meta);
    font-variant-numeric: tabular-nums;
  }
  .meta {
    margin: 0;
    color: var(--text-meta);
  }
  .err {
    margin: 0;
    color: var(--danger-text);
  }
</style>
