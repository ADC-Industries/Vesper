<script lang="ts">
  // Site ▸ Backups (settings): one row per backup set (Vesper per box, Oriel per computer), its
  // newest snapshot, Back up now (this box), Restore… with the server's preview (items to tick,
  // what needs a restart, onto which box, "adopt" with a warning) and the result; restores
  // queued for show boxes with their state and Cancel. The schedule is fixed on the server:
  // shown read-only (plan question 11).
  import type { BackupSetSummary, RestorePreview, RestoreResult, SnapshotSummary } from "$protocol";
  import { Button, StatusDot } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { sitePage } from "../../state/site.svelte";
  import { CodexBackupsClient, isQueued } from "../../server/codexBackups";
  import { errorText } from "../../state/errors";
  import { boxName } from "../../server/codexBox";
  import { RESTORE_STATE, formatWhen, isThisBox } from "./logic";

  const REASON: Record<string, string> = { scheduled: "daily", change: "after a change", manual: "by hand", beforeRestore: "before a restore", upload: "from Oriel", unknown: "" };

  let open = $state<string | null>(null);
  let snaps = $state.raw<SnapshotSummary[]>([]);
  let snap = $state<string | null>(null);
  let preview = $state.raw<RestorePreview | null>(null);
  /** What `preview` was asked for: the restore sends exactly that snapshot, onto that box. */
  let previewFor = $state.raw<{ set: string; snapshot: string; target: string } | null>(null);
  /** Answers that come back after a newer question (another set, snapshot or box) are dropped. */
  let seq = 0;
  let picked = $state<Record<string, boolean>>({});
  let target = $state<string>("");
  let adopt = $state(false);
  let busy = $state(false);
  let problem = $state<string | null>(null);
  let result = $state.raw<{ set: string; r: RestoreResult } | null>(null);

  let thisId = $derived(session.server?.id ?? "");
  let elsewhere = $derived(sitePage.elsewhere("backups") && !sitePage.managedOn);

  function machineName(s: BackupSetSummary): string {
    return s.set.name ?? s.set.machine;
  }
  async function toggle(s: BackupSetSummary) {
    result = null;
    if (open === s.set.id) {
      open = null;
      return;
    }
    open = s.set.id;
    preview = null;
    previewFor = null;
    problem = null;
    snaps = [];
    target = s.set.machine;
    adopt = false;
    const mine = ++seq;
    try {
      const list = await sitePage.codexBox((c) => c.snapshots(s.set.id));
      if (mine !== seq) return;
      snaps = list;
      snap = snaps[0]?.id ?? null;
      if (snap) await loadPreview(s);
    } catch (e) {
      if (mine === seq) problem = errorText(e);
    }
  }
  /** The preview of the chosen snapshot onto the chosen box (the box's own settings decide
   * what would change: for a replacement box, its own, not the old box's). */
  async function loadPreview(s: BackupSetSummary) {
    if (!snap) return;
    const ask = { set: s.set.id, snapshot: snap, target };
    const mine = ++seq;
    preview = null;
    previewFor = null;
    problem = null;
    const onto = ask.target !== s.set.machine ? ask.target : undefined;
    const p = await sitePage.admin("The restore preview", () => sitePage.codexBox((c) => c.preview(ask.set, ask.snapshot, onto)));
    if (mine !== seq) return;
    if (!p) {
      problem = "The preview isn't available.";
      return;
    }
    preview = p;
    previewFor = ask;
    picked = Object.fromEntries(p.changes.map((c) => [c.key, true]));
  }
  async function restore(s: BackupSetSummary) {
    const pv = preview;
    const ask = previewFor;
    if (!pv || !ask || ask.set !== s.set.id) return;
    const keys = pv.changes.filter((c) => picked[c.key]).map((c) => c.key);
    const all = keys.length === pv.changes.length;
    busy = true;
    const r = await sitePage.admin(`Restoring ${machineName(s)}`, () =>
      sitePage.codexBox((c) =>
        c.restore(ask.set, ask.snapshot, {
          ...(all ? {} : { items: keys }),
          ...(ask.target !== s.set.machine ? { target: ask.target, adoptServerId: adopt || undefined } : {}),
        }),
      ),
    );
    busy = false;
    if (!r) return;
    open = null;
    if (isQueued(r)) {
      session.conn?.pushNotice("info", `The restore waits for ${r.targetName}: it applies once that box asks (at most 30 s while it's online).`, "site:restore");
    } else {
      result = { set: s.set.id, r };
    }
    void sitePage.loadBackups();
  }
  async function backUpNow() {
    // Always asked of the box Oriel is connected to: a show box offers its own settings to the
    // Codex itself (the Codex box would back up its own instead).
    const r = await sitePage.admin("Backing up this box", () =>
      session.conn ? new CodexBackupsClient(session.conn.rest).backUpNow() : Promise.reject(new Error("Not connected")),
    );
    if (r) session.conn?.pushNotice("info", r.created ? "This box's settings were backed up." : "Nothing changed since the last backup.", "site:backup");
    void sitePage.loadBackups();
  }
  async function cancel(id: string) {
    await sitePage.admin("Cancelling the restore", () => sitePage.codexBox((c) => c.cancelRestore(id)));
    void sitePage.loadBackups();
  }
</script>

<div class="st-head">
  <h1>Backups</h1>
  <span class="sub">Schedule: daily and after changes</span>
  <span class="st-grow"></span>
  <Button size="sm" onclick={backUpNow}>Back up this box now</Button>
</div>
<p class="st-note">Every box's settings (screens, looks, outputs, audio) and each Oriel's preferences are backed up to the Codex. To replace a broken box, set up the new one and restore the old box's backup onto it.</p>
{#if sitePage.managedOn}<p class="st-meta">Managed on {boxName(sitePage.managedOn)}</p>{/if}

{#if elsewhere}
  <div class="st-card">
    <span>Settings backups are kept on the box that holds the site's Codex.</span>
    <div class="st-row"><Button size="sm" variant="primary" onclick={() => sitePage.useCodexBox()}>Use the Codex box</Button></div>
  </div>
{:else}
  {#if sitePage.problems.backups}<p class="st-problem">Couldn't read the backups: {sitePage.problems.backups}</p>{/if}
  {#if result}
    <div class="result" role="status">
      <span>Restored: {result.r.summary}.</span>
      {#if result.r.restart.length}<span class="restart">Restart needed for {result.r.restart.join(", ")}.</span>{/if}
      <span class="st-meta">Restoring the “before restore” backup taken {formatWhen(result.r.beforeRestore.takenAt)} undoes it.</span>
    </div>
  {/if}
  <div class="st-table-wrap">
    <table class="st" data-testid="backup-sets">
      <thead><tr><th>Product</th><th>Machine</th><th>Last backup</th><th>Snapshots</th><th aria-label="Actions"></th></tr></thead>
      <tbody>
        {#each sitePage.backups as s (s.set.id)}
          <tr class:sel={open === s.set.id}>
            <td>{s.set.product === "oriel" ? "Oriel" : s.set.product === "vesper" ? "Vesper" : s.set.product}</td>
            <td>
              <span class="st-two">
                <span class="st-strong">{machineName(s)}</span>
                <span class="st-meta">{isThisBox(s, thisId, !!sitePage.managedOn) ? "this box" : s.yours ? "this computer" : s.set.product === "oriel" ? "an Oriel computer" : "another box"}</span>
              </span>
            </td>
            <td>{s.latest ? `${formatWhen(s.latest.takenAt)}${REASON[s.latest.reason] ? ` · ${REASON[s.latest.reason]}` : ""}` : "–"}</td>
            <td class="num">{s.snapshots}</td>
            <td class="actions">
              {#if s.set.product === "oriel"}
                <span class="st-meta">{s.yours ? "Restore in Settings ▸ General" : "Restored from that computer"}</span>
              {:else}
                <Button size="sm" onclick={() => toggle(s)} aria-expanded={open === s.set.id}>Restore…</Button>
              {/if}
            </td>
          </tr>
          {#if open === s.set.id}
            <tr>
              <td colspan="5" class="restore-cell"><div class="restore">
                <div class="st-row">
                  <label class="st-row">Snapshot
                    <select bind:value={snap} onchange={() => loadPreview(s)}>
                      {#each snaps as x (x.id)}<option value={x.id}>{formatWhen(x.takenAt)} · {REASON[x.reason] ?? x.reason} · {x.items} items</option>{/each}
                    </select>
                  </label>
                  <label class="st-row">Onto
                    <select bind:value={target} onchange={() => loadPreview(s)} data-testid="restore-target">
                      <option value={s.set.machine}>{machineName(s)} (its own box)</option>
                      {#if s.set.machine !== thisId}<option value={thisId}>{session.server?.name ?? "this box"} (this box)</option>{/if}
                    </select>
                  </label>
                </div>
                {#if problem}<p class="st-problem">{problem}</p>{/if}
                {#if preview}
                  <span class="st-strong">Restoring {machineName(s)} from {formatWhen(preview.snapshot.takenAt)} will: {preview.summary}</span>
                  {#if preview.currentAsOf}<span class="st-meta">Compared with its newest backup ({formatWhen(preview.currentAsOf)}).</span>{/if}
                  <div class="items">
                    {#each preview.changes as c (c.key)}
                      <label class="tick">
                        <input type="checkbox" checked={picked[c.key]} onchange={(e) => (picked = { ...picked, [c.key]: e.currentTarget.checked })} />
                        <span>{c.change === "added" ? "Add" : c.change === "removed" ? "Remove" : "Change"} {c.kind} {c.name ?? c.id ?? ""}</span>
                        {#if c.restart}<span class="st-meta">needs a restart</span>{/if}
                      </label>
                    {/each}
                    {#if !preview.changes.length}<span class="st-empty">Nothing would change.</span>{/if}
                  </div>
                  {#if preview.unchanged}<span class="st-meta">{preview.unchanged} other items are the same.</span>{/if}
                  {#if preview.restart.length}<span class="warn">After a restart: {preview.restart.join(", ")}.</span>{/if}
                  {#if target !== s.set.machine}
                    <label class="tick adopt">
                      <input type="checkbox" bind:checked={adopt} />
                      <span>Take over {machineName(s)}'s identity (its server ID): only for a box replacing one the site removed. The old box must never come back.</span>
                    </label>
                  {/if}
                  <div class="st-row">
                    <Button size="sm" onclick={() => (open = null)}>Cancel</Button>
                    <Button size="sm" variant="primary" onclick={() => restore(s)} disabled={busy || !preview.changes.some((c) => picked[c.key])}>Restore</Button>
                  </div>
                {:else if !problem}
                  <span class="st-meta">Reading the preview…</span>
                {/if}
              </div></td>
            </tr>
          {/if}
        {:else}
          <tr><td colspan="5" class="st-empty">No backups yet.</td></tr>
        {/each}
      </tbody>
    </table>
  </div>

  {#if sitePage.restores.length}
    <h2 class="st-section">Restores to show boxes</h2>
    <div class="st-table-wrap">
      <table class="st" data-testid="restores">
        <thead><tr><th>Box</th><th>State</th><th>Queued</th><th aria-label="Actions"></th></tr></thead>
        <tbody>
          {#each sitePage.restores as q (q.id)}
            <tr>
              <td class="st-strong">{q.targetName}</td>
              <td>
                <StatusDot status={RESTORE_STATE[q.state]?.status ?? "off"} label={RESTORE_STATE[q.state]?.label ?? q.state} />
                {#if q.error}<div class="st-meta">{q.error}</div>{/if}
                {#if q.result}<div class="st-meta">{q.result.summary}</div>{/if}
              </td>
              <td>{formatWhen(q.queuedAt)}</td>
              <td class="actions">{#if q.state === "queued"}<Button size="sm" onclick={() => cancel(q.id)}>Cancel</Button>{/if}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
{/if}

<style>
  tr.sel {
    background: var(--accent-sel);
  }
  .num {
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
  }
  .restore-cell {
    background: var(--surface-sunk);
  }
  .restore,
  .items {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
  }
  .items {
    padding-left: 4px;
  }
  .tick {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .tick input {
    accent-color: var(--accent);
  }
  .adopt {
    color: var(--live);
  }
  .warn {
    color: var(--live);
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
