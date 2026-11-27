<script lang="ts">
  // Protect the Codex ▸ Import an export: upload the file, type its passphrase, see what it
  // holds per collection (new / same / newer / older / in the trash) with a choice each, its
  // settings backups and the site's identity, then apply. "Restart needed" once the site's
  // identity was staged.
  import type { ImportPreview, ImportResult, ImportUpload } from "$protocol";
  import { Button, Chip } from "../../ui";
  import { sitePage } from "../../state/site.svelte";
  import { session } from "../../state/session.svelte";
  import { ApiRequestError } from "../../server/rest";
  import { errorText } from "../../state/errors";
  import { formatBytes } from "../library/logic";
  import { CHOICE_LABEL, COMPARISON, defaultChoices, formatWhen, importNote, importOptions, importRequest, type ImportChoice } from "./logic";

  let { siteOption }: { siteOption: boolean } = $props();

  let upload = $state.raw<ImportUpload | null>(null);
  let fileName = $state("");
  let pass = $state("");
  let preview = $state.raw<ImportPreview | null>(null);
  let choices = $state<Record<string, ImportChoice>>({});
  let backups = $state(true);
  let site = $state(false);
  let busy = $state<string | null>(null);
  let error = $state<string | null>(null);
  let result = $state.raw<ImportResult | null>(null);
  let input: HTMLInputElement | undefined = $state();

  async function pick(e: Event & { currentTarget: HTMLInputElement }) {
    const f = e.currentTarget.files?.[0];
    if (!f) return;
    await reset();
    fileName = f.name;
    busy = "Uploading…";
    upload = (await sitePage.admin("Uploading the export", () => sitePage.codexBox((c) => c.uploadImport(f, sitePage.auth())))) ?? null;
    busy = null;
    if (input) input.value = "";
  }
  async function open() {
    if (!upload) return;
    busy = "Opening…";
    error = null;
    try {
      const id = upload.id;
      const p = await sitePage.codexBox((c) => c.previewImport(id, pass));
      preview = p;
      choices = defaultChoices(p);
      backups = true;
      site = false;
    } catch (e) {
      error = e instanceof ApiRequestError && e.field === "passphrase" ? "Wrong passphrase, or the file is damaged." : errorText(e);
    }
    busy = null;
  }
  async function apply() {
    if (!upload || !preview) return;
    const id = upload.id;
    const req = importRequest(pass, preview, choices, { backups, site: site && siteOption && preview.site });
    busy = "Importing…";
    const r = await sitePage.admin("The import", () => sitePage.codexBox((c) => c.applyImport(id, req)));
    busy = null;
    if (!r) return;
    result = r;
    upload = null;
    preview = null;
    pass = "";
    session.conn?.pushNotice("info", `Imported: ${r.added.length} added, ${r.replaced.length} replaced.`, "site:import");
    void sitePage.loadBackups();
  }
  async function reset() {
    const id = upload?.id;
    upload = null;
    preview = null;
    result = null;
    error = null;
    pass = "";
    if (id) await sitePage.codexBox((c) => c.dropImport(id)).catch(() => {});
  }
</script>

<section class="st-card" aria-labelledby="import-h" data-testid="import-card">
  <div class="st-row">
    <h2 id="import-h">Import an export</h2>
    <span class="st-grow"></span>
    {#if upload || preview}<Button size="sm" onclick={reset}>Cancel</Button>{/if}
  </div>
  {#if result}
    <div class="result" role="status">
      <span>Imported: {result.added.length} collection{result.added.length === 1 ? "" : "s"} added, {result.replaced.length} replaced, {result.skipped.length} left as they were; {result.backupSets} settings backup set{result.backupSets === 1 ? "" : "s"} added.</span>
      {#if result.restartNeeded}<span class="restart">Restart needed: the site's identity takes over at the server's next start.</span>{/if}
      <span class="st-meta">{result.note}</span>
    </div>
  {/if}
  {#if !upload}
    <span class="st-meta">Brings collections and settings backups back from a Codex export (.vcodex). Collections that are here are kept unless you replace them; a replaced one goes to the collection trash first.</span>
    <div class="st-row">
      <label class="ui-file">
        <input bind:this={input} type="file" accept=".vcodex,application/octet-stream" onchange={pick} disabled={!!busy} data-testid="import-file" />
        <span>Choose an export file…</span>
      </label>
      {#if busy}<span class="st-meta" role="status">{busy}</span>{/if}
    </div>
  {:else if !preview}
    <span>{fileName} · {formatBytes(upload.bytes)} uploaded</span>
    <label class="f">Passphrase<input type="password" bind:value={pass} onkeydown={(e) => e.key === "Enter" && open()} /></label>
    {#if error}<span class="err">{error}</span>{/if}
    <div class="st-row"><Button size="sm" variant="primary" onclick={open} disabled={!pass || !!busy}>{busy ?? "Open"}</Button></div>
  {:else}
    <span class="st-meta">From {preview.manifest.serverName} · made {formatWhen(preview.manifest.createdAt)} · Vesper {preview.manifest.version}</span>
    <div class="st-table-wrap">
      <table class="st" data-testid="import-preview">
        <thead><tr><th>Collection</th><th>In the file</th><th>Compared with here</th><th>Import</th></tr></thead>
        <tbody>
          {#each preview.collections as c (c.collection.id)}
            {@const opts = importOptions(c)}
            {@const note = importNote(c)}
            <tr>
              <td><span class="st-two"><span class="st-strong">{c.collection.name}</span>{#if note}<span class="st-meta">{note}</span>{/if}</span></td>
              <td class="num">{c.collection.documents} documents · {formatWhen(c.collection.lastChangeAt)}</td>
              <td><Chip kind={c.comparison === "new" ? "next" : c.comparison === "newer" ? "edited" : "neutral"}>{COMPARISON[c.comparison]}</Chip></td>
              <td>
                {#if opts.length > 1}
                  <select aria-label={`Import ${c.collection.name}`} value={choices[c.collection.id]} onchange={(e) => (choices = { ...choices, [c.collection.id]: e.currentTarget.value as ImportChoice })}>
                    {#each opts as o (o)}<option value={o}>{CHOICE_LABEL[o]}</option>{/each}
                  </select>
                {:else}
                  <span class="st-meta">{CHOICE_LABEL[opts[0] ?? "skip"]}</span>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
    {#if preview.backupSets.length}
      <label class="tick"><input type="checkbox" bind:checked={backups} /> Add the settings backups this Codex lacks ({preview.backupSets.filter((b) => !b.present).map((b) => b.name).join(", ") || "none new"})</label>
    {/if}
    {#if preview.site && siteOption}
      <label class="tick"><input type="checkbox" bind:checked={site} /> Replace the site's identity with the one in the file (at the server's next start)</label>
    {/if}
    <p class="warn">{preview.note}</p>
    <div class="st-row"><Button size="sm" variant="primary" onclick={apply} disabled={!!busy}>{busy ?? "Import"}</Button></div>
  {/if}
</section>

<style>
  .f {
    display: grid;
    grid-template-columns: 96px minmax(0, 320px);
    align-items: center;
    gap: 8px;
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .err {
    color: var(--danger-text);
  }
  .warn {
    margin: 0;
    color: var(--live);
    font-size: var(--fs-sm);
  }
  .tick {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .tick input {
    accent-color: var(--accent);
  }
  .num {
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .ui-file {
    position: relative;
    display: inline-flex;
    align-items: center;
    height: 26px;
    padding: 0 10px;
    border-radius: var(--r);
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    cursor: pointer;
    font-size: var(--fs-sm);
  }
  .ui-file:hover {
    background: var(--surface-overlay);
  }
  .ui-file input {
    position: absolute;
    inset: 0;
    opacity: 0;
    cursor: pointer;
  }
  .result {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 10px 12px;
    border-radius: var(--r);
    background: var(--ok-bg);
    color: var(--ok-bg-text);
  }
  .restart {
    color: var(--live);
    font-weight: 600;
  }
</style>
