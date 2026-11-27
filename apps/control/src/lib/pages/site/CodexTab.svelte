<script lang="ts">
  // Site ▸ Codex: the Codex box, its collections (managed in the Library's all-collections
  // view), storage, and "Protect the Codex": a manual export, nightly exports and importing an
  // export. The canvas's per-library "Back up now" and "Last backup" are this one export
  // (plan question 11). On a show box these go to the Codex box over the side connection.
  import { Button, StatusDot } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { shell } from "../../state/shell.svelte";
  import { siteState, sitePage } from "../../state/site.svelte";
  import { libraryPage } from "../../state/library.svelte";
  import { formatBytes } from "../library/logic";
  import { boxName } from "../../server/codexBox";
  import { formatWhen } from "./logic";
  import ExportCard from "./ExportCard.svelte";
  import NightlyCard from "./NightlyCard.svelte";
  import ImportCard from "./ImportCard.svelte";
  import CodexBoxes from "./CodexBoxes.svelte";

  let codex = $derived(siteState.codex);
  let host = $derived(codex?.role === "host");
  let st = $derived(sitePage.storage);
  let canExport = $derived(siteState.has("codexExport"));
  let elsewhere = $derived(sitePage.elsewhere("export") && !sitePage.managedOn);
  let boxLabel = $derived(host ? (session.server?.name ?? "this server") : boxName(sitePage.managedOn ?? null, codex?.sync?.name));

  function manage() {
    libraryPage.go({ kind: "collections" });
    shell.go("library");
  }
</script>

<div class="st-head">
  <h1>Codex</h1>
  <span class="sub">{codex ? `${codex.collections} collection${codex.collections === 1 ? "" : "s"} · ${codex.documents} documents` : ""}</span>
  <span class="st-grow"></span>
  <Button size="sm" onclick={manage}>Manage collections…</Button>
</div>

<div class="box" data-testid="codex-box">
  <span class="st-two"><span class="st-strong">Codex on {boxLabel}</span><span class="st-meta">{host ? "This server holds the site's Codex" : "This box shows a copy of one collection"}</span></span>
  {#if codex?.sync}
    <StatusDot status={codex.sync.reachable ? "ok" : "error"} label={codex.sync.reachable ? (codex.sync.behind ? `${codex.sync.behind} changes behind` : "In sync") : "Can't be reached"} />
  {:else}
    <StatusDot status={codex ? "ok" : "off"} label={codex ? "Online" : "Unknown"} />
  {/if}
  <span class="fig"><span class="st-meta">Shown here</span><span>{codex?.collectionName ?? "–"}</span></span>
  <span class="fig"><span class="st-meta">Collections</span><span>{codex?.collections ?? "–"}</span></span>
  {#if st}
    <span class="fig"><span class="st-meta">Media</span><span>{formatBytes(st.library.bytes)} · {st.library.items} files</span></span>
    <span class="fig"><span class="st-meta">Disk free</span><span>{formatBytes(st.disk.available)} of {formatBytes(st.disk.total)}</span></span>
  {/if}
  <span class="fig"><span class="st-meta">Last export</span><span>{formatWhen(sitePage.exportStatus?.last?.createdAt)}</span></span>
</div>
{#if codex?.sync?.problem}<p class="st-problem">{codex.sync.problem}</p>{/if}
<p class="st-note">This site has one Codex, and every collection lives on it. Other buildings reach it over the site VPN.</p>
{#if host}<CodexBoxes />{/if}

<h2 class="st-section">Protect the Codex</h2>
{#if !canExport}
  <p class="st-empty">This server can't export its Codex (update the server).</p>
{:else if elsewhere}
  <div class="st-card">
    <span>Exports are kept on {boxName(null, codex?.sync?.name)}, the box that holds the site's Codex.</span>
    <div class="st-row"><Button size="sm" variant="primary" onclick={() => sitePage.useCodexBox()}>Use the Codex box</Button></div>
  </div>
{:else}
  {#if sitePage.managedOn}<p class="st-meta">Managed on {boxName(sitePage.managedOn, codex?.sync?.name)}</p>{/if}
  <div class="protect">
    <ExportCard siteOption={host && !!sitePage.status?.running && !sitePage.managedOn} />
    <NightlyCard />
  </div>
  <ImportCard siteOption={host && !!sitePage.status?.running && !sitePage.managedOn} />
{/if}

<style>
  .box {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px 22px;
    padding: 12px 14px;
    border-radius: var(--r-card);
    background: var(--surface-panel);
    border: 1px solid var(--line);
  }
  .fig {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-variant-numeric: tabular-nums;
  }
  .protect {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(380px, 1fr));
    gap: 14px;
    align-items: start;
  }
</style>
