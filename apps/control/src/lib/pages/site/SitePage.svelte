<script lang="ts">
  // The Site page (plan 4.9): the toolbar (the site, device counts, the site code and New code),
  // the tabs on the left (Devices, Join requests, Displays, Codex, Backups, Security, with badges) and
  // the current tab (the unlock prompt is in the main view, so the Library uses it too). Updates (G10) shows when the server has the update channel (`updates`); Signage with `signage`.
  import Monitor from "lucide-svelte/icons/monitor";
  import LogIn from "lucide-svelte/icons/log-in";
  import BookOpen from "lucide-svelte/icons/book-open";
  import Download from "lucide-svelte/icons/download";
  import Shield from "lucide-svelte/icons/shield";
  import CircleArrowUp from "lucide-svelte/icons/circle-arrow-up";
  import MonitorPlay from "lucide-svelte/icons/monitor-play";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import Tv from "lucide-svelte/icons/tv";
  import { onDestroy, onMount } from "svelte";
  import PageFrame from "../PageFrame.svelte";
  import { Button } from "../../ui";
  import { ICON_SIZE, ICON_STROKE } from "../../ui/types";
  import { session } from "../../state/session.svelte";
  import { siteState, sitePage } from "../../state/site.svelte";
  import { updatesPage } from "../../state/updates.svelte";
  import { attentionCount } from "./updates";
  import { apiWarning, damagedWarning, deviceSummary, mmss, spacedCode, type SiteTab } from "./logic";
  import DevicesTab from "./DevicesTab.svelte";
  import JoinTab from "./JoinTab.svelte";
  import CodexTab from "./CodexTab.svelte";
  import BackupsTab from "./BackupsTab.svelte";
  import SecurityTab from "./SecurityTab.svelte";
  import UpdatesTab from "./UpdatesTab.svelte";
  import DisplaysTab from "./DisplaysTab.svelte";
  import SignageTab from "./SignageTab.svelte";

  onMount(() => sitePage.open());
  onDestroy(() => sitePage.close());

  let now = $state(Date.now());
  const tick = setInterval(() => (now = Date.now()), 1000);
  onDestroy(() => clearInterval(tick));

  let attention = $derived(sitePage.devices.filter((d) => d.health === "needsAttention").length);
  const ALL_TABS: { id: SiteTab; name: string; icon: typeof Monitor }[] = [
    { id: "devices", name: "Devices", icon: Monitor },
    { id: "join", name: "Join requests", icon: LogIn },
    { id: "displays", name: "Displays", icon: MonitorPlay },
    { id: "signage", name: "Signage", icon: Tv },
    { id: "codex", name: "Codex", icon: BookOpen },
    { id: "backups", name: "Backups", icon: Download },
    { id: "updates", name: "Updates", icon: CircleArrowUp },
    { id: "security", name: "Security", icon: Shield },
  ];
  // Updates only on a server with the update channel; Displays with Mosaic (`mosaic`).
  let TABS = $derived(ALL_TABS.filter((t) => (t.id !== "updates" || siteState.has("updates")) && (t.id !== "displays" || siteState.has("mosaic")) && (t.id !== "signage" || siteState.has("signage"))));
  function badge(id: SiteTab): number {
    if (id === "join") return sitePage.joinRequests.length + sitePage.pairings.length;
    if (id === "devices") return attention;
    if (id === "updates") return attentionCount(updatesPage.launchers);
    if (id === "backups") return sitePage.restores.filter((r) => r.state === "queued" || r.state === "applying" || r.state === "failed").length;
    return 0;
  }

  let code = $derived(sitePage.joinCode);
  let left = $derived(code ? Math.max(0, code.expiresInSeconds - Math.floor((now - sitePage.joinCodeAt) / 1000)) : 0);
  let codesOff = $derived(sitePage.status?.joinMode === "approval");
  // In another server's site: its devices, joins and codes are managed at that site's server.
  let siteServer = $derived(
    sitePage.status && !sitePage.status.running && sitePage.membership?.state === "enrolled" ? (sitePage.membership.server ?? "the site's server") : null,
  );
  // The Site Manager runs, but without its admin panel (its port taken): on every tab.
  let apiWarn = $derived(apiWarning(sitePage.status));
  // Started from damaged site settings: locked down until the access is set again (Security).
  let damagedWarn = $derived(damagedWarning(sitePage.status));
</script>

<PageFrame page="site">
  {#snippet controls()}
    <span class="summary" data-testid="site-summary">
      {session.server?.name ?? "This site"}{sitePage.devices.length ? ` · ${deviceSummary(sitePage.devices)}` : ""}
    </span>
    <span class="grow"></span>
    {#if siteServer}
      <span class="meta">In the site at {siteServer}</span>
    {:else if codesOff}
      <span class="meta">Site codes are off · devices ask to join</span>
    {:else if code?.code && left > 0}
      <span class="meta">Site code</span>
      <span class="code" aria-label={`Site code ${code.code.split("").join(" ")}`}>{spacedCode(code.code)}</span>
      <span class="meta tab-nums">· single use · expires in {mmss(left)} ·</span>
      <Button size="sm" onclick={() => sitePage.newCode()}>New code</Button>
    {:else}
      <span class="meta">No site code</span>
      <Button size="sm" onclick={() => sitePage.newCode()}>New code</Button>
    {/if}
  {/snippet}

  <div class="site-page">
    <nav aria-label="Site sections">
      {#each TABS as t (t.id)}
        {@const n = badge(t.id)}
        <button type="button" class:cur={sitePage.tab === t.id} aria-current={sitePage.tab === t.id ? "page" : undefined} onclick={() => (sitePage.tab = t.id)} data-testid="site-tab-{t.id}">
          <t.icon size={ICON_SIZE} strokeWidth={ICON_STROKE} aria-hidden="true" />
          <span class="name">{t.name}</span>
          {#if n > 0}<span class="badge">{n}</span>{/if}
        </button>
      {/each}
    </nav>
    <main>
      {#if apiWarn}
        <div class="api-warn" role="status" data-testid="site-api-error">
          <TriangleAlert size={ICON_SIZE} strokeWidth={ICON_STROKE} aria-hidden="true" />
          <span>{apiWarn}</span>
        </div>
      {/if}
      {#if damagedWarn}
        <div class="api-warn" role="status" data-testid="site-settings-damaged">
          <TriangleAlert size={ICON_SIZE} strokeWidth={ICON_STROKE} aria-hidden="true" />
          <span class="grow">{damagedWarn}</span>
          {#if sitePage.tab !== "security"}
            <Button size="sm" onclick={() => (sitePage.tab = "security")}>Open Security</Button>
          {/if}
        </div>
      {/if}
      {#if siteServer && (sitePage.tab === "devices" || sitePage.tab === "join")}
        <div class="st-head"><h1>{sitePage.tab === "devices" ? "Devices" : "Join requests"}</h1></div>
        <p class="st-note" data-testid="site-elsewhere">
          This server is part of the site at {siteServer}. The site's devices, join requests and site codes are managed on that server's Site page.
        </p>
      {:else if sitePage.tab === "devices"}
        <DevicesTab />
      {:else if sitePage.tab === "join"}
        <JoinTab />
      {:else if sitePage.tab === "codex"}
        <CodexTab />
      {:else if sitePage.tab === "backups"}
        <BackupsTab />
      {:else if sitePage.tab === "displays" && siteState.has("mosaic")}
        <DisplaysTab />
      {:else if sitePage.tab === "signage" && siteState.has("signage")}
        <SignageTab />
      {:else if sitePage.tab === "updates" && siteState.has("updates")}
        <UpdatesTab />
      {:else}
        <SecurityTab />
      {/if}
    </main>
  </div>
</PageFrame>

<style>
  .summary {
    color: var(--text-label);
  }
  .grow {
    flex: 1;
  }
  .meta {
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .tab-nums,
  .code {
    font-variant-numeric: tabular-nums;
  }
  .code {
    font-size: 16px;
    font-weight: 800;
    letter-spacing: 0.12em;
  }
  .site-page {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  nav {
    flex: 0 0 220px;
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    background: var(--surface-panel);
    border-right: 1px solid var(--line);
    overflow-y: auto;
  }
  nav button {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 10px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
    text-align: left;
    font-size: var(--fs);
    cursor: pointer;
  }
  nav button:hover {
    background: var(--surface-raised);
  }
  nav button.cur {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .name {
    flex: 1;
  }
  .badge {
    min-width: 18px;
    padding: 1px 6px;
    border-radius: 9px;
    background: var(--accent);
    color: var(--text-strong);
    font-size: var(--fs-xs);
    font-weight: 700;
    text-align: center;
  }
  main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 16px 20px 28px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  main > :global(*) {
    flex-shrink: 0;
  }
  .api-warn {
    display: flex;
    align-items: flex-start;
    gap: 10px;
    padding: 9px 12px;
    border: 1px solid var(--live);
    border-radius: var(--r-card);
    background: var(--live-soft);
    color: var(--text-strong);
    line-height: 1.4;
  }
  .api-warn :global(svg) {
    flex: none;
    margin-top: 1px;
    color: var(--live);
  }

  /* Shared by the tabs. */
  .site-page :global(.st-head) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
  }
  .site-page :global(.st-head h1) {
    margin: 0;
    font-size: 18px;
  }
  .site-page :global(.st-head .sub) {
    color: var(--text-label);
  }
  .site-page :global(.st-grow) {
    flex: 1;
  }
  .site-page :global(.st-card) {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px;
    border-radius: var(--r-card);
    background: var(--surface-panel);
    border: 1px solid var(--line);
  }
  .site-page :global(.st-card h2) {
    margin: 0;
    font-size: var(--fs);
  }
  .site-page :global(h2.st-section) {
    margin: 6px 0 0;
    font-size: var(--fs);
    color: var(--text-label);
  }
  .site-page :global(.st-note) {
    margin: 0;
    color: var(--text-label);
    line-height: 1.5;
  }
  .site-page :global(.st-meta) {
    color: var(--text-meta);
    font-size: 11.5px;
  }
  .site-page :global(.st-problem) {
    margin: 0;
    color: var(--danger-text);
  }
  .site-page :global(.st-table-wrap) {
    overflow-x: auto;
    border: 1px solid var(--line);
    border-radius: var(--r-card);
  }
  .site-page :global(table.st) {
    width: 100%;
    border-collapse: collapse;
  }
  .site-page :global(table.st th) {
    padding: 8px 10px;
    text-align: left;
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-label);
    border-bottom: 1px solid var(--line);
    white-space: nowrap;
    background: var(--surface-panel);
  }
  .site-page :global(table.st td) {
    padding: 9px 10px;
    border-bottom: 1px solid var(--surface-raised);
    vertical-align: middle;
  }
  .site-page :global(table.st tr:last-child td) {
    border-bottom: 0;
  }
  .site-page :global(td.actions) {
    white-space: nowrap;
    text-align: right;
  }
  .site-page :global(td.actions > *) {
    margin-left: 4px;
  }
  .site-page :global(.st-two) {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .site-page :global(.st-strong) {
    font-weight: 600;
  }
  .site-page :global(.st-row) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
  }
  .site-page :global(.st-confirm) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    padding: 12px;
    border-radius: var(--r-card);
    border: 1px solid var(--danger-bg);
    background: var(--surface-panel);
  }
  .site-page :global(.st-empty) {
    margin: 0;
    color: var(--text-meta);
  }
  .site-page :global(.st-mono) {
    font-family: var(--mono);
  }
</style>
