<script lang="ts">
  // Protect the Codex ▸ Nightly exports: on / off, the folder on the Codex box, the time, keep
  // 7 nightly + 4 weekly, the stored passphrase (set / clear, never shown), last and next, the
  // problem in words, and the export files.
  import type { UpdateExportSettings } from "$protocol";
  import { Button, Switch } from "../../ui";
  import { sitePage } from "../../state/site.svelte";
  import { formatBytes } from "../library/logic";
  import { formatWhen, passphraseProblem } from "./logic";

  let st = $derived(sitePage.exportStatus);
  let folder = $state<string | null>(null);
  let time = $state<string | null>(null);
  let pass = $state("");
  let setting = $state(false);

  async function save(body: UpdateExportSettings) {
    const r = await sitePage.admin("Changing nightly exports", () => sitePage.codexBox((c) => c.saveExportSettings(body)));
    if (r) sitePage.exportStatus = r;
    folder = time = null;
  }
  async function setPass() {
    if (passphraseProblem(pass)) return;
    const r = await sitePage.admin("Storing the passphrase", () => sitePage.codexBox((c) => c.setPassphrase(pass)));
    if (r) sitePage.exportStatus = r;
    pass = "";
    setting = false;
  }
  async function clearPass() {
    const r = await sitePage.admin("Clearing the passphrase", () => sitePage.codexBox((c) => c.clearPassphrase()));
    if (r) sitePage.exportStatus = r;
  }
</script>

<section class="st-card" aria-labelledby="nightly-h" data-testid="nightly-card">
  <div class="st-row">
    <h2 id="nightly-h">Nightly exports</h2>
    <span class="st-grow"></span>
    {#if st}<Switch label="Nightly exports" checked={st.settings.enabled} onchange={(v) => save({ enabled: v })} />{/if}
  </div>
  {#if sitePage.problems.export}
    <p class="st-problem">{sitePage.problems.export}</p>
  {:else if !st}
    <p class="st-empty">Reading…</p>
  {:else}
    <label class="f">Folder
      <input type="text" value={folder ?? st.settings.folder ?? st.folder} oninput={(e) => (folder = e.currentTarget.value)} onchange={() => folder !== null && save({ folder })} title="A folder on the Codex box: a USB drive or a NAS mount" />
    </label>
    <label class="f">Time
      <input type="time" value={time ?? st.settings.time} oninput={(e) => (time = e.currentTarget.value)} onchange={() => time && save({ time })} />
    </label>
    <div class="f"><span>Keeps</span><span>{st.settings.keepNightly} nightly + {st.settings.keepWeekly} weekly</span></div>
    <div class="f">
      <span>Passphrase</span>
      {#if setting}
        <span class="st-row">
          <input type="password" autocomplete="new-password" bind:value={pass} placeholder="At least 8 characters" />
          <Button size="sm" variant="primary" onclick={setPass} disabled={!!passphraseProblem(pass)}>Store</Button>
          <Button size="sm" onclick={() => ((setting = false), (pass = ""))}>Cancel</Button>
        </span>
      {:else}
        <span class="st-row">
          <span>{st.passphraseSet ? "Stored on the Codex box (never shown)" : "None stored"}</span>
          <Button size="sm" onclick={() => (setting = true)}>{st.passphraseSet ? "Change…" : "Set…"}</Button>
          {#if st.passphraseSet}<Button size="sm" onclick={clearPass}>Clear</Button>{/if}
        </span>
      {/if}
    </div>
    <div class="f"><span>Last</span><span>{st.last ? `${formatWhen(st.last.createdAt)} · ${formatBytes(st.last.bytes)}` : "None yet"}</span></div>
    <div class="f"><span>Next</span><span>{st.active ? formatWhen(st.nextAt) : "Not scheduled"}</span></div>
    {#if st.problem}<p class="warn">{st.problem}</p>{/if}
    {#if sitePage.scheduled.length}
      <details>
        <summary>{sitePage.scheduled.length} export file{sitePage.scheduled.length === 1 ? "" : "s"} in the folder</summary>
        <ul>
          {#each sitePage.scheduled as f (f.file)}
            <li><span class="st-mono">{f.file}</span> <span class="st-meta">{formatWhen(f.createdAt)} · {formatBytes(f.bytes)}</span></li>
          {/each}
        </ul>
      </details>
    {/if}
  {/if}
</section>

<style>
  .f {
    display: grid;
    grid-template-columns: 96px 1fr;
    align-items: center;
    gap: 8px;
    color: var(--text-2);
  }
  .f > :first-child {
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  label.f {
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  input[type="time"] {
    width: 130px;
    height: var(--control-h);
    padding: 0 8px;
    border-radius: var(--r);
    border: 1px solid var(--line);
    background: var(--surface-raised);
    color: var(--text);
    color-scheme: dark;
  }
  .warn {
    margin: 0;
    color: var(--live);
  }
  details summary {
    cursor: pointer;
    color: var(--accent-link);
  }
  ul {
    margin: 6px 0 0;
    padding-left: 18px;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
</style>
