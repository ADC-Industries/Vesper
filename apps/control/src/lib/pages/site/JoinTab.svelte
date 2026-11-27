<script lang="ts">
  // Site ▸ Join requests: how machines join (the join mode, the setup window, the site code in
  // the toolbar), machines asking to join (compare the code, name, Approve / Deny), Oriel
  // computers waiting for approval (Approve, Deny with a reason) and "Recently".
  import type { JoinMode, JoinRequestInfo, PairingRequest } from "$protocol";
  import { Button } from "../../ui";
  import { sitePage } from "../../state/site.svelte";
  import { JOIN_MODES, ago, eventLine, isJoinEvent, spacedCode } from "./logic";

  let names = $state<Record<number, string>>({});
  let status = $derived(sitePage.status);
  let recent = $derived(sitePage.events.filter(isJoinEvent).slice(0, 8));

  async function setMode(m: JoinMode, input: HTMLInputElement) {
    if (!status || status.joinMode === m) return;
    await sitePage.act("Changing how devices join", (s) => s.saveSettings({ joinMode: m }, status.revision), sitePage.loadStatus);
    // The radios show the site's mode, also when the change failed or the unlock was cancelled
    // (the browser keeps the clicked one, and the same mode back doesn't redraw them).
    const now = sitePage.status?.joinMode;
    for (const r of input.form?.elements ?? document.getElementsByName(input.name)) if (r instanceof HTMLInputElement && r.name === input.name) r.checked = r.value === now;
  }
  async function window_(open: boolean) {
    await sitePage.act(open ? "Opening the setup window" : "Closing the setup window", (s) => (open ? s.openWindow(600) : s.closeWindow()), sitePage.loadStatus);
  }
  const refresh = () => Promise.all([sitePage.loadJoin(), sitePage.loadDevices(), sitePage.loadEvents(), sitePage.loadSecurity()]);
  async function approve(r: JoinRequestInfo) {
    const name = (names[r.id] ?? r.name).trim();
    await sitePage.act(`Approving ${r.name}`, (s) => s.approveJoin(r.id, name ? { name } : {}), refresh);
  }
  async function deny(r: JoinRequestInfo) {
    await sitePage.act(`Refusing ${r.name}`, (s) => s.denyJoin(r.id), refresh);
  }
  async function approvePairing(p: PairingRequest) {
    await sitePage.act(`Approving ${p.client}`, (s) => s.approvePairing(p.id), refresh);
  }
  let denying = $state<string | null>(null);
  let reason = $state("");
  async function denyPairing(p: PairingRequest) {
    const r = reason.trim();
    denying = null;
    reason = "";
    await sitePage.act(`Refusing ${p.client}`, (s) => s.denyPairing(p.id, r ? { reason: r } : {}), refresh);
  }
</script>

<div class="st-head"><h1>Join requests</h1></div>

<section class="st-card" aria-labelledby="join-modes-h">
  <h2 id="join-modes-h">How new devices can join</h2>
  <span class="st-meta">Pick one for the whole site.</span>
  <div role="radiogroup" aria-labelledby="join-modes-h" class="modes">
    {#each JOIN_MODES as m (m.value)}
      <label class="mode">
        <input type="radio" name="join-mode" value={m.value} checked={status?.joinMode === m.value} onchange={(e) => setMode(m.value, e.currentTarget)} disabled={!status} />
        <span class="st-two"><span class="st-strong">{m.name}</span><span class="st-meta">{m.desc}</span></span>
      </label>
    {/each}
  </div>
  <span class="st-meta">Site codes (shown above) work once, expire after 10 minutes, and stop working after 5 wrong tries.</span>
  <div class="st-row">
    {#if status?.pairingOpen}
      <span class="open">The setup window is open: devices without a code may ask now.</span>
      <Button size="sm" onclick={() => window_(false)}>Close the setup window</Button>
    {:else}
      <span class="st-meta">Approval needs the setup window open.</span>
      <Button size="sm" onclick={() => window_(true)}>Open a 10-minute setup window</Button>
    {/if}
  </div>
</section>

<p class="st-note">A device that asks to join shows a 6-digit code on its own screen. Check it matches the code here before you approve.</p>
{#if sitePage.problems.joinRequests}<p class="st-problem">Couldn't read the join requests: {sitePage.problems.joinRequests}</p>{/if}

{#each sitePage.joinRequests as r (r.id)}
  <div class="request" data-testid="join-request">
    <div class="who st-two">
      <span class="st-strong">{r.name}</span>
      <span class="st-meta">{r.kind === "output" ? "Output machine" : r.kind === "input" ? "Input machine" : "App"}{r.address ? ` · ${r.address}` : ""} · asked {ago(r.askedAt)}{r.byCode ? " · typed the site code" : ""}</span>
    </div>
    {#if r.code}
      <div class="code">
        <span class="lbl">JOIN CODE</span>
        <span class="digits" aria-label={`Join code ${r.code.split("").join(" ")}`}>{spacedCode(r.code)}</span>
        <span class="st-meta">Check this matches the code on that device</span>
      </div>
    {/if}
    <label class="name">Name<input type="text" value={names[r.id] ?? r.name} oninput={(e) => (names = { ...names, [r.id]: e.currentTarget.value })} /></label>
    <Button size="sm" onclick={() => deny(r)}>Deny</Button>
    <Button size="sm" variant="primary" onclick={() => approve(r)}>Approve</Button>
  </div>
{/each}

<h2 class="st-section">Oriel computers and Launchers waiting for approval</h2>
{#if sitePage.problems.pairings}<p class="st-problem">Couldn't read them: {sitePage.problems.pairings}</p>{/if}
{#each sitePage.pairings as p (p.id)}
  <div class="request" data-testid="pairing-request">
    <div class="who st-two">
      <span class="st-strong">{p.client}</span>
      <span class="st-meta">{p.launcher ? "Vesper Launcher: reports and updates for this machine only" : "Oriel"} · asked at {p.via}{p.relayed ? " (relayed)" : ""}{p.address ? ` · ${p.address}` : ""} · {ago(p.askedAt)}</span>
    </div>
    {#if denying === p.id}
      <label class="name">Reason<input type="text" bind:value={reason} placeholder="Told to that computer (optional)" onkeydown={(e) => e.key === "Enter" && denyPairing(p)} /></label>
      <Button size="sm" onclick={() => (denying = null)}>Cancel</Button>
      <Button size="sm" variant="danger" onclick={() => denyPairing(p)}>Deny</Button>
    {:else}
      <Button size="sm" onclick={() => ((denying = p.id), (reason = ""))}>Deny…</Button>
      <Button size="sm" variant="primary" onclick={() => approvePairing(p)}>Approve</Button>
    {/if}
  </div>
{:else}
  <p class="st-empty">No Oriel computers are waiting.</p>
{/each}
{#if !sitePage.joinRequests.length}<p class="st-empty">No devices are waiting to join.</p>{/if}

{#if recent.length}
  <h2 class="st-section">Recently</h2>
  <div class="recent">
    {#each recent as e, i (i)}<span class:warn={e.level === "warning" || e.level === "error"}>{eventLine(e)}</span>{/each}
  </div>
{/if}

<style>
  .modes {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .mode {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    cursor: pointer;
  }
  .mode input {
    margin-top: 3px;
    accent-color: var(--accent);
  }
  .open {
    color: var(--ok-text);
  }
  .request {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 12px;
    padding: 12px;
    border-radius: var(--r-card);
    background: var(--surface-panel);
    border: 1px solid var(--line);
  }
  .who {
    flex: 1 1 240px;
  }
  .code {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    padding: 6px 14px;
    border-radius: var(--r-card);
    background: var(--surface-app);
    border: 1px solid #3d3a66;
  }
  .lbl {
    font-size: 9.5px;
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--text-label);
  }
  .digits {
    font-family: var(--mono);
    font-size: 22px;
    font-weight: 700;
    letter-spacing: 0.12em;
    line-height: 1.1;
    font-variant-numeric: tabular-nums;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .name input {
    width: 200px;
  }
  .recent {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .recent .warn {
    color: var(--live);
  }
</style>
