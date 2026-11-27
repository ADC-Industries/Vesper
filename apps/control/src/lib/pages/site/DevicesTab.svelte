<script lang="ts">
  // Site ▸ Devices: the Site Manager's view of each machine (health, problems, version) with
  // Identify, Restart (a picker of the device's services as its Node reports them, or the
  // service's id typed when it reports none), Logs and Remove from site (confirmed in place, as
  // drawn). The Codex's own
  // membership shows as "Codex (built-in)" with no Remove. A pairing relay (a Vesper box the
  // site's server marked itself, IDN Beta11) says "Relays pairings"; an admin can turn that off
  // or on (Relay off / Relay on) for the rare manual case: Relay on only for a Vesper box
  // (M-6: the server refuses any other machine), Relay off for any machine marked. G9: "This Vesper" (version, OS, CPU,
  // GPU, uptime, products: `GET /server/details`) above the table, and under each machine's
  // name its products (the services its Node reports) and its details as its Node reports them
  // (`SiteDevice.machine`: system, CPU, GPUs, memory, uptime; in the same words as "This
  // Vesper"). A device whose Node doesn't report them (an older Node, no Node) shows none.
  import type { DeviceService, LogSnapshot, SiteDevice } from "$protocol";
  import { Button, StatusDot } from "../../ui";
  import { sitePage } from "../../state/site.svelte";
  import { shell } from "../../state/shell.svelte";
  import { errorText } from "../../state/errors";
  import { HEALTH, canRestartService, deviceKindLine, deviceName, deviceSummary, healthText, relayToggle, serviceState } from "./logic";
  import { serverDetails } from "../../state/details.svelte";
  import { detailsLines, machineLines } from "../../shell/details";
  import { siteState } from "../../state/site.svelte";
  import { mosaicPage } from "../../state/mosaic.svelte";
  import { boxSummary, updatesNote } from "./mosaic";

  $effect(() => serverDetails.use());
  // Mosaic display status (G9): a display box's screens, read once as the tab opens.
  $effect(() => {
    if (siteState.has("mosaic")) void mosaicPage.load();
  });
  const mosaicBox = (d: SiteDevice) => mosaicPage.state?.boxes.find((b) => b.machine === d.fingerprint);
  let here = $derived(serverDetails.details ? detailsLines(serverDetails.details) : null);
  const facts = (d: SiteDevice): string[] =>
    d.machine && !d.builtIn ? machineLines(d.machine, { online: d.health !== "offline" && d.health !== "removed", nodeStartedAt: d.nodeStartedAt }) : [];
  const products = (d: SiteDevice): string => (d.services ?? []).map((s) => s.name).filter((n) => n).join(", ");

  let identifying = $state<Record<string, boolean>>({});
  let logsOpen = $state<string | null>(null);
  let removeAsk = $state<string | null>(null);
  let restartOpen = $state<string | null>(null);
  let logs = $state.raw<LogSnapshot[] | null>(null);
  let logText = $state<{ name: string; text: string } | null>(null);
  let logProblem = $state<string | null>(null);

  let list = $derived(sitePage.devices.filter((d) => d.health !== "removed"));
  let removed = $derived(sitePage.devices.filter((d) => d.health === "removed"));

  async function identify(d: SiteDevice) {
    const ok = await sitePage.act(`Identifying ${d.name}`, (s) => s.identify(d.fingerprint, { seconds: 10 }));
    if (!ok) return;
    identifying = { ...identifying, [d.fingerprint]: true };
    setTimeout(() => (identifying = { ...identifying, [d.fingerprint]: false }), 10_000);
  }
  async function identifyAll() {
    for (const d of list) if (d.node && d.health !== "offline") void identify(d);
  }
  const services = (d: SiteDevice): DeviceService[] => d.services ?? [];
  async function restart(d: SiteDevice) {
    if (services(d).length) {
      removeAsk = null;
      logsOpen = null;
      restartOpen = restartOpen === d.fingerprint ? null : d.fingerprint;
      return;
    }
    // No services reported: ask for the service's id.
    const service = await shell.askText(`Restart a service on ${d.name}`, "", "Restart", "Service");
    if (!service) return;
    await sitePage.act(`Restarting ${d.name}`, (s) => s.restart(d.fingerprint, { service }), sitePage.loadDevices);
  }
  async function restartService(d: SiteDevice, s: DeviceService) {
    const ok = await sitePage.act(`Restarting ${s.name} on ${d.name}`, (c) => c.restart(d.fingerprint, { service: s.id }), sitePage.loadDevices);
    if (ok) restartOpen = null;
  }
  async function toggleLogs(d: SiteDevice) {
    removeAsk = null;
    restartOpen = null;
    logText = null;
    logProblem = null;
    if (logsOpen === d.fingerprint) {
      logsOpen = null;
      return;
    }
    logsOpen = d.fingerprint;
    logs = null;
    try {
      logs = (await sitePage.client()?.logs(d.fingerprint)) ?? [];
    } catch (e) {
      logProblem = errorText(e);
    }
  }
  async function newSnapshot(d: SiteDevice) {
    if (await sitePage.act(`Fetching logs from ${d.name}`, (s) => s.takeLogs(d.fingerprint))) {
      logs = (await sitePage.client()?.logs(d.fingerprint).catch(() => logs)) ?? logs;
    }
  }
  async function openFile(d: SiteDevice, snapshot: string, file: string) {
    try {
      const text = (await sitePage.client()?.logFile(d.fingerprint, snapshot, file, sitePage.auth())) ?? "";
      logText = { name: `${snapshot} / ${file}`, text };
    } catch (e) {
      logProblem = errorText(e);
    }
  }
  async function remove(d: SiteDevice) {
    removeAsk = null;
    await sitePage.act(`Removing ${d.name}`, (s) => s.removeDevice(d.fingerprint), () => Promise.all([sitePage.loadDevices(), sitePage.loadEvents()]));
  }
  async function setRelay(d: SiteDevice, on: boolean) {
    await sitePage.act(on ? `Letting ${d.name} relay pairings` : `Stopping ${d.name} relaying pairings`, (s) => s.setRelay(d.fingerprint, { on }), sitePage.loadDevices);
  }
  async function allow(d: SiteDevice) {
    await sitePage.act(`Letting ${d.name} ask again`, (s) => s.allowDevice(d.fingerprint), sitePage.loadDevices);
  }
</script>

<div class="st-head">
  <h1>Devices</h1>
  <span class="sub">{deviceSummary(sitePage.devices)}</span>
  <span class="st-grow"></span>
  <Button size="sm" onclick={identifyAll} disabled={!list.some((d) => d.node && d.health !== "offline")}>Identify all</Button>
</div>

{#if sitePage.problems.devices}<p class="st-problem">Couldn't read the devices: {sitePage.problems.devices}</p>{/if}

{#if here}
  <section class="here" aria-label="This Vesper" data-testid="this-vesper">
    <span class="st-strong">{here[0]}</span>
    <span class="st-meta">{here.slice(1).join(" · ")}</span>
  </section>
{/if}

<div class="st-table-wrap">
  <table class="st" data-testid="site-devices">
    <thead><tr><th class="dev">Device</th><th>Status</th><th>Version</th><th aria-label="Actions"></th></tr></thead>
    <tbody>
      {#each list as d (d.fingerprint)}
        {@const off = d.health === "offline" || d.health === "restarting" || !d.node}
        {@const f = facts(d)}
        <tr class:ident={identifying[d.fingerprint]}>
          <td>
            <span class="st-two">
              <span class="st-strong">{deviceName(d)}</span>
              <span class="st-meta"
                >{deviceKindLine(d)}{#if relayToggle(d)}
                  <button
                    type="button"
                    class="relay"
                    onclick={() => setRelay(d, !d.relay)}
                    title={d.relay
                      ? "Stop showing the site's pairing codes here: pairings through it wait for an admin's approval"
                      : "Show the site's pairing codes here, so a computer typing one pairs without approval"}
                    >{d.relay ? "Relay off" : "Relay on"}</button
                  >{/if}</span
              >
              {#if !d.builtIn && products(d)}<span class="st-meta">Runs {products(d)}</span>{/if}
              {#if mosaicBox(d)}{@const mb = mosaicBox(d)!}<span class="st-meta" class:mwarn={mb.vesper || !mb.reachable} data-testid="device-mosaic">{boxSummary(mb, mosaicPage.state?.channels ?? [])}</span>{/if}
              {#if f.length}
                <span class="st-meta facts" data-testid="device-machine" title={f.join("\n")}>
                  {#each f as x, i (i)}<span class="fact">{x}{i < f.length - 1 ? " ·" : ""}</span>{" "}{/each}
                </span>
              {/if}
            </span>
          </td>
          <td class="status">
            <StatusDot status={HEALTH[d.health]?.status ?? "off"} label={healthText(d)} />
            {#each d.problems.filter((p) => p.text !== d.reason) as p (p.problem)}
              <div class="st-meta problem">{p.text}</div>
            {/each}
          </td>
          <td class="ver">
            {d.version ?? "–"}
            {#if updatesNote(d)}<div class="st-meta managed" data-testid="device-updates" title="Vesper's package installs and updates this machine's IDN Node; the Site Manager leaves it out of its updates">{updatesNote(d)}</div>{/if}
          </td>
          <td class="actions">
            {#if d.node}
              <Button size="sm" onclick={() => identify(d)} disabled={off}>{identifying[d.fingerprint] ? "Identifying…" : "Identify"}</Button>
              <Button size="sm" onclick={() => restart(d)} disabled={off} aria-expanded={services(d).length ? restartOpen === d.fingerprint : undefined}>Restart</Button>
              <Button size="sm" onclick={() => toggleLogs(d)} aria-expanded={logsOpen === d.fingerprint}>Logs</Button>
            {/if}
            {#if !d.builtIn}
              <Button size="sm" onclick={() => ((removeAsk = removeAsk === d.fingerprint ? null : d.fingerprint), (logsOpen = null), (restartOpen = null))} aria-expanded={removeAsk === d.fingerprint}>Remove from site</Button>
            {/if}
          </td>
        </tr>
        {#if restartOpen === d.fingerprint && services(d).length}
          <tr>
            <td colspan="4" class="services">
              <span class="st-strong">Restart a service on {d.name}</span>
              <ul data-testid="device-services" aria-label={`Services of ${d.name}`}>
                {#each services(d) as s (s.id)}
                  {@const st = serviceState(d.restarting.includes(s.id) ? "restarting" : s.state)}
                  <li>
                    <span class="svc-name">{s.name}</span>
                    <StatusDot status={st.status} label={st.label} />
                    <Button size="sm" onclick={() => restartService(d, s)} disabled={!canRestartService(s, d.restarting)}>Restart</Button>
                  </li>
                {/each}
              </ul>
            </td>
          </tr>
        {/if}
        {#if logsOpen === d.fingerprint}
          <tr>
            <td colspan="4" class="logs">
              <div class="st-row">
                <span class="st-strong">Logs of {d.name}</span>
                <span class="st-grow"></span>
                <Button size="sm" onclick={() => newSnapshot(d)}>Fetch new logs</Button>
              </div>
              {#if logProblem}<p class="st-problem">{logProblem}</p>{/if}
              {#if logs === null && !logProblem}
                <p class="st-empty">Reading…</p>
              {:else if logs && !logs.length}
                <p class="st-empty">No log snapshots yet: fetch new logs.</p>
              {:else if logs}
                <div class="snaps">
                  {#each logs as s (s.snapshot)}
                    <span class="st-meta">{s.snapshot}</span>
                    {#each s.files as f (f.name)}
                      <button type="button" class="file" onclick={() => openFile(d, s.snapshot, f.name)}>{f.name} <span class="st-meta">{Math.ceil(f.size / 1000)} KB</span></button>
                    {/each}
                  {/each}
                </div>
              {/if}
              {#if logText}
                <span class="st-meta">{logText.name}</span>
                <pre>{logText.text}</pre>
              {/if}
            </td>
          </tr>
        {/if}
        {#if removeAsk === d.fingerprint}
          <tr>
            <td colspan="4">
              <div class="st-confirm" role="alertdialog" aria-label={`Remove ${d.name} from the site`}>
                <span class="st-two st-grow">
                  <span class="st-strong">Remove {d.name} from the site?</span>
                  <span class="st-meta">It stops working with the site right away and will need to join again.</span>
                </span>
                <Button size="sm" onclick={() => (removeAsk = null)}>Cancel</Button>
                <Button size="sm" variant="danger" onclick={() => remove(d)}>Remove from site</Button>
              </div>
            </td>
          </tr>
        {/if}
      {:else}
        <tr><td colspan="4" class="st-empty">No devices in the site yet.</td></tr>
      {/each}
    </tbody>
  </table>
</div>

{#if removed.length}
  <h2 class="st-section">Removed</h2>
  {#each removed as d (d.fingerprint)}
    <div class="st-row">
      <span>{d.name}</span>
      <span class="st-meta">removed or refused</span>
      <Button size="sm" onclick={() => allow(d)}>Let it ask again</Button>
    </div>
  {/each}
{/if}

<p class="st-note">Each machine's details come from its IDN Node, which reports every 30 s; older Nodes don't report them. Roles appear here once the site reports them. A Vesper box that joins is set to relay pairings: it shows the site's pairing codes, so a computer typing one there pairs without approval.</p>

<style>
  .mwarn {
    color: var(--warn);
  }
  tr.ident {
    background: #24223a;
  }
  .ver {
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .managed {
    font-variant-numeric: normal;
  }
  .problem {
    margin-top: 2px;
  }
  /* A machine's details: each fact whole on a line where it fits. */
  .fact {
    display: inline-block;
    max-width: 100%;
  }
  /* (Room for them: the status's reason wraps instead.) */
  th.dev {
    width: 40%;
  }
  td.status :global(.ui-status .label) {
    white-space: normal;
  }
  .logs,
  .services {
    background: var(--surface-sunk);
  }
  .services ul {
    list-style: none;
    margin: 8px 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .services li {
    display: flex;
    align-items: center;
    gap: 16px;
  }
  .svc-name {
    min-width: 160px;
    color: var(--text);
  }
  .services li :global(.ui-status) {
    min-width: 170px;
  }
  .snaps {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 10px;
    margin: 8px 0;
  }
  /* The relay toggle: a small link in the kind line (the actions stay one row at 1366). */
  .relay {
    margin-left: 8px;
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    color: var(--accent-link);
    cursor: pointer;
  }
  .relay:hover {
    text-decoration: underline;
  }
  .file {
    padding: 3px 8px;
    border-radius: var(--r-sm);
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    color: var(--accent-link);
    cursor: pointer;
  }
  pre {
    margin: 4px 0 0;
    max-height: 260px;
    overflow: auto;
    padding: 10px;
    border-radius: var(--r);
    background: var(--surface-well);
    font-family: var(--mono);
    font-size: 12px;
    color: var(--text-2);
    user-select: text;
  }
  .here {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0 0 12px;
    padding: 10px 12px;
    border-radius: var(--r-card);
    background: var(--surface-raised);
    border: 1px solid var(--line-strong);
  }
</style>
