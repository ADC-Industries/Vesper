<script lang="ts">
  // Site ▸ Updates (G10, docs/LAUNCHER-PLAN.md): every machine's Vesper Launcher (its products,
  // versions and state, "update available", Update and Roll back, the machine's numbers) and the
  // site's update channel (signed packages: upload with their signature, delete). The Launcher
  // picks a queued action up with its next report (within 30 s), verifies the package's
  // signature itself and rolls back by itself if the new version doesn't come up healthy.
  // "Add app…" on a machine queues an install of a product it can run (the update channel's,
  // or GitHub's). "Releases from GitHub": the server brings new signed releases into the
  // channel by itself (its last check, errors, the channel and the off switch).
  import { onDestroy, onMount } from "svelte";
  import type { LauncherEntry, LauncherProduct, ReleaseChannel } from "$protocol";
  import { Button, Select, StatusDot, Switch } from "../../ui";
  import { updatesPage } from "../../state/updates.svelte";
  import { formatWhen } from "./logic";
  import { addVersion, broughtIn, byProduct, bytes, failedAdds, githubLine, packageSource, productName, queuedAdds, roleLabel, canAct, healthLine, outcomeText, pendingFor, platformLabel, seenAgo, sourcesText, stateLabel, stateTone, updateFor } from "./updates";

  onMount(() => updatesPage.open());
  onDestroy(() => updatesPage.close());

  let list = $derived(updatesPage.launchers);
  let groups = $derived(byProduct(updatesPage.packages));
  let pkgFile = $state<File | null>(null);
  let sigFile = $state<File | null>(null);
  let pkgInput = $state<HTMLInputElement | null>(null);
  let sigInput = $state<HTMLInputElement | null>(null);
  let deleteAsk = $state<string | null>(null);

  function update(e: LauncherEntry, p: LauncherProduct, version: string) {
    void updatesPage.queue(e.report.machineId, { type: "update", product: p.product, version });
  }
  function rollback(e: LauncherEntry, p: LauncherProduct) {
    void updatesPage.queue(e.report.machineId, { type: "rollback", product: p.product });
  }
  async function upload() {
    if (!pkgFile || !sigFile) return;
    const p = await updatesPage.upload(pkgFile, sigFile);
    if (p) {
      pkgFile = null;
      sigFile = null;
      if (pkgInput) pkgInput.value = "";
      if (sigInput) sigInput.value = "";
    }
  }
  const pick = (ev: Event) => (ev.currentTarget as HTMLInputElement).files?.[0] ?? null;

  let gh = $derived(updatesPage.github);
  let repo = $state("");
  let repoFor: string | null = null;
  $effect(() => {
    // The repository field follows the server's until it is edited.
    const r = gh?.settings.repository ?? null;
    if (r !== null && r !== repoFor) {
      repo = r;
      repoFor = r;
    }
  });
  const CHANNELS = [
    { value: "stable", label: "Stable releases" },
    { value: "beta", label: "Betas too" },
  ];
  const INTERVALS = [
    { value: "1", label: "Every hour" },
    { value: "6", label: "Every 6 hours" },
    { value: "24", label: "Once a day" },
  ];
  let intervals = $derived.by(() => {
    const h = String(gh?.settings.intervalHours ?? 6);
    return INTERVALS.some((o) => o.value === h) ? INTERVALS : [...INTERVALS, { value: h, label: `Every ${h} hours` }];
  });
  function setGithub(change: Partial<{ enabled: boolean; channel: ReleaseChannel; intervalHours: number; repository: string }>) {
    if (!gh) return;
    void updatesPage.setGithub({ ...gh.settings, ...change });
  }
</script>

<div class="st-head">
  <h1>Updates</h1>
  <span class="sub">{list.length} machine{list.length === 1 ? "" : "s"} · {updatesPage.packages.length} package{updatesPage.packages.length === 1 ? "" : "s"} in the channel</span>
</div>

{#if updatesPage.problem}<p class="st-problem">Couldn't read the updates: {updatesPage.problem}</p>{/if}

<h2 class="st-section">Machines</h2>
{#if updatesPage.loaded && list.length === 0}
  <p class="st-note" data-testid="updates-empty">
    No Vesper Launcher has reported to this server yet. A machine appears here once its Launcher is set up with this server as its update channel.
  </p>
{:else}
  <div class="st-table-wrap">
    <table class="st" data-testid="updates-machines">
      <thead><tr><th class="what">Machine and product</th><th>State</th><th>Version</th><th aria-label="Actions"></th></tr></thead>
      <tbody>
        {#each list as e (e.report.machineId)}
          <tr class="machine">
            <td colspan="4">
              <span class="st-two">
                <span class="mrow">
                  <span class="st-strong">{e.report.machineName}</span>
                  {#if e.online}
                    <StatusDot status="ok" label="Online" />
                  {:else}
                    <StatusDot status="error" label={`Offline · last report ${seenAgo(e.lastSeenSeconds)}`} />
                  {/if}
                  <span class="grow"></span>
                  <Button size="sm" onclick={() => void updatesPage.openAdd(e.report.machineId)} disabled={!e.online} data-testid="updates-add-app">{updatesPage.addFor === e.report.machineId ? "Close" : `Add app to ${e.report.machineName}…`}</Button>
                </span>
                <span class="st-meta">
                  {platformLabel(e.report.platform)}{e.report.roles.length ? ` · ${e.report.roles.map(roleLabel).join(", ")}` : ""} · Launcher {e.report.launcherVersion}
                </span>
                <span class="st-meta" data-testid="updates-health">{healthLine(e.report.health)}</span>
                {#each queuedAdds(e) as q (q)}<span class="st-meta avail">{q}</span>{/each}
                {#each failedAdds(e) as f (f.text)}<span class="st-meta bad" data-testid="updates-add-failed">{f.text} · {formatWhen(f.at)}</span>{/each}
              </span>
            </td>
          </tr>
          {#if updatesPage.addFor === e.report.machineId}
            <tr class="addapp" data-testid="updates-catalogue">
              <td colspan="4">
                {#if updatesPage.catalogueProblem}
                  <span class="st-meta bad">Couldn't read what can be added: {updatesPage.catalogueProblem}</span>
                {:else if updatesPage.catalogue === null}
                  <span class="st-meta">Looking for apps…</span>
                {:else if updatesPage.catalogue.length === 0}
                  <span class="st-meta">Nothing to add: {e.report.machineName} has every app it can run ({platformLabel(e.report.platform)}{e.report.roles.length ? `, ${e.report.roles.map(roleLabel).join(", ")}` : ""}).</span>
                {:else}
                  <span class="st-meta">Apps {e.report.machineName} can run. Its Launcher installs the one you pick with its next report and checks its signature first.</span>
                  <span class="cat">
                    {#each updatesPage.catalogue as c (c.product)}
                      {@const v = addVersion(c)}
                      <span class="catrow">
                        <span class="st-two">
                          <span class="pname">{c.name}</span>
                          <span class="st-meta">{v}{c.stable && c.beta ? ` · beta ${c.beta}` : !c.stable ? " (beta)" : ""} · from {sourcesText(c)}</span>
                        </span>
                        {#if v}<Button size="sm" variant="primary" onclick={() => void updatesPage.add(e.report.machineId, c.product, v)}>Add {c.name}</Button>{/if}
                      </span>
                    {/each}
                  </span>
                {/if}
              </td>
            </tr>
          {/if}
          {#each e.report.products as p (p.product)}
            {@const next = updateFor(e, p.product)}
            {@const queued = pendingFor(e, p.product)}
            <tr class="product" data-testid="updates-product">
              <td>
                <span class="st-two">
                  <span class="pname">{p.name}</span>
                  {#if p.lastUpdate}
                    <span class="st-meta" class:bad={p.lastUpdate.result !== "installed"} title={p.lastUpdate.message}>{outcomeText(p.lastUpdate)} · {formatWhen(p.lastUpdate.at)}</span>
                  {/if}
                  {#if p.restarts > 0}<span class="st-meta">Restarted {p.restarts}× after a crash</span>{/if}
                </span>
              </td>
              <td>
                {#if e.online}
                  <StatusDot status={stateTone(p.state)} label={stateLabel(p.state)} pulse={p.state === "updating" || p.state === "rollingBack"} />
                {:else}
                  <StatusDot status="off" label={`${stateLabel(p.state)} at the last report`} />
                {/if}
              </td>
              <td class="ver">
                <span class="st-two">
                  <span>{p.version ?? "Not installed"}</span>
                  {#if next}<span class="avail">Update available: {next}</span>{/if}
                  {#if queued}<span class="st-meta">{queued}</span>{/if}
                </span>
              </td>
              <td class="actions">
                {#if next}
                  <Button size="sm" variant="primary" onclick={() => update(e, p, next)} disabled={!canAct(e, p)}>Update to {next}</Button>
                {/if}
                {#if p.previous}
                  <Button size="sm" onclick={() => rollback(e, p)} disabled={!canAct(e, p)}>Roll back to {p.previous}</Button>
                {/if}
              </td>
            </tr>
          {/each}
        {/each}
      </tbody>
    </table>
  </div>
{/if}

{#if gh}
  <h2 class="st-section">Releases from GitHub</h2>
  <p class="st-note">
    This server looks for new Vesper releases on GitHub and brings the ones your machines need into the update channel, signature checked. Installing them stays your choice.
  </p>
  <div class="st-card" data-testid="updates-github">
    <div class="st-row">
      <Switch checked={gh.settings.enabled} label="Look for new releases" showLabel onchange={(v) => setGithub({ enabled: v })} data-testid="updates-github-enabled" />
      <span class="grow"></span>
      <Button size="sm" onclick={() => void updatesPage.checkGithub()} disabled={!gh.settings.enabled || gh.checking}>{gh.checking ? "Checking…" : "Check now"}</Button>
    </div>
    <span class="st-meta" data-testid="updates-github-status">{githubLine(gh, (iso) => formatWhen(iso))}</span>
    {#if gh.error}<span class="st-meta bad" data-testid="updates-github-error">{gh.error}</span>{/if}
    <div class="st-row">
      <span class="lbl">Releases</span>
      <Select label="Which releases" options={CHANNELS} value={gh.settings.channel} onchange={(v) => setGithub({ channel: v as ReleaseChannel })} />
      <Select label="How often" options={intervals} value={String(gh.settings.intervalHours)} onchange={(v) => setGithub({ intervalHours: Number(v) })} />
    </div>
    <div class="st-row">
      <span class="lbl">Repository</span>
      <input type="text" class="repo" bind:value={repo} aria-label="GitHub repository" onchange={() => repo.trim() && repo.trim() !== gh?.settings.repository && setGithub({ repository: repo.trim() })} />
    </div>
    {#if gh.added.length}
      <span class="st-meta">Brought in lately: {gh.added.slice(0, 5).map(broughtIn).join(" · ")}</span>
    {/if}
  </div>
{/if}

<h2 class="st-section">Update channel</h2>
<p class="st-note">
  Packages signed with the Vesper release key. This server checks the signature when a package is uploaded, and each machine's Launcher checks it again before installing.
</p>
<div class="st-card" data-testid="updates-upload">
  <span class="st-meta">Upload a release: the package (a .deb or a Windows installer) and its signature file (.minisig) from the same release.</span>
  <div class="st-row">
    <label class="ui-file">
      <input bind:this={pkgInput} type="file" data-testid="updates-package-file" onchange={(ev) => (pkgFile = pick(ev))} />
      <span>{pkgFile ? pkgFile.name : "Choose the package…"}</span>
    </label>
    <label class="ui-file">
      <input bind:this={sigInput} type="file" accept=".minisig" data-testid="updates-signature-file" onchange={(ev) => (sigFile = pick(ev))} />
      <span>{sigFile ? sigFile.name : "Choose its signature…"}</span>
    </label>
    <Button size="sm" variant="primary" onclick={upload} disabled={!pkgFile || !sigFile || updatesPage.uploading}>{updatesPage.uploading ? "Uploading…" : "Upload"}</Button>
  </div>
</div>

{#if groups.length === 0}
  <p class="st-empty">No packages yet.</p>
{:else}
  <div class="st-table-wrap">
    <table class="st" data-testid="updates-packages">
      <thead><tr><th>Product</th><th>Version</th><th>Platform</th><th>Size</th><th>Signed by</th><th>Added</th><th aria-label="Actions"></th></tr></thead>
      <tbody>
        {#each groups as [product, pkgs] (product)}
          {#each pkgs as k (k.id)}
            <tr>
              <td>{productName(k.product)}</td>
              <td class="ver">{k.version}</td>
              <td>{platformLabel(k.platform)}</td>
              <td class="ver">{bytes(k.size)}</td>
              <td class="st-mono" title={k.trustedComment}>{k.keyId}</td>
              <td><span class="st-two"><span>{formatWhen(k.uploadedAt)}</span><span class="st-meta">{packageSource(k)}</span></span></td>
              <td class="actions">
                {#if deleteAsk === k.id}
                  <Button size="sm" variant="danger" onclick={() => ((deleteAsk = null), void updatesPage.remove(k.id))}>Delete {k.version}</Button>
                  <Button size="sm" onclick={() => (deleteAsk = null)}>Keep</Button>
                {:else}
                  <Button size="sm" onclick={() => (deleteAsk = k.id)}>Delete…</Button>
                {/if}
              </td>
            </tr>
          {/each}
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  th.what {
    width: 44%;
  }
  tr.machine td {
    background: var(--surface-sunk);
  }
  .mrow {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  tr.product td:first-child {
    padding-left: 24px;
  }
  .pname {
    color: var(--text);
  }
  .ver {
    font-variant-numeric: tabular-nums;
  }
  .avail {
    color: var(--accent-link);
    font-size: var(--fs-sm);
  }
  .bad,
  /* (over the Site page's `.st-meta` colour) */
  span.st-meta.bad {
    color: var(--warn);
  }
  [data-testid="updates-upload"] {
    margin-bottom: 12px;
  }
  .ui-file {
    position: relative;
    display: inline-flex;
    align-items: center;
    max-width: 320px;
    height: 26px;
    padding: 0 10px;
    border-radius: var(--r);
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    cursor: pointer;
    font-size: var(--fs-sm);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .grow {
    flex: 1;
  }
  tr.addapp td {
    padding-left: 24px;
  }
  .cat {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 8px;
    max-width: 640px;
  }
  .catrow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
  }
  [data-testid="updates-github"] {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 12px;
  }
  .lbl {
    min-width: 84px;
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  input.repo {
    width: 260px;
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
</style>
