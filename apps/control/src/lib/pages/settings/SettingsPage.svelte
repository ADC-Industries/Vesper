<script lang="ts">
  // The Settings page (plan 4.10; Ctrl+,, the Oriel menu's Settings pane, `#/settings`): a page
  // like the others, with its toolbar (Close, back to where the operator came from) and
  // sections General (Oriel's preferences and their backup to the Codex), Audio, Captions, Network & IDN
  // (with Control surfaces: Remote API compatibility), Recording & streaming (the server's
  // recording and stream targets),
  // Media (media preparation), Reports (CCLI usage),
  // Keyboard (the shortcut table, read-only), Bible (once the server has scripture, G11) and
  // About. The section is `shell.settingsSection`.
  import { onMount } from "svelte";
  import type { OutputTransportInfo } from "$protocol";
  import { PROTOCOL_VERSION } from "$protocol";
  import { Button } from "../../ui";
  import PageFrame from "../PageFrame.svelte";
  import { APP_FULL_NAME } from "../../config";
  import { isTauri } from "../../tauri";
  import { session } from "../../state/session.svelte";
  import { shell } from "../../state/shell.svelte";
  import { showView } from "../../state/show.svelte";
  import { siteState, sitePage, orielPrefs } from "../../state/site.svelte";
  import { SHORTCUT_GROUPS, rowsOf } from "../../shell/shortcuts";
  import Logo from "../../components/Logo.svelte";
  import AudioSettings from "./AudioSettings.svelte";
  import CaptionsSettings from "./CaptionsSettings.svelte";
  import StreamingSettings from "./StreamingSettings.svelte";
  import ControlSurfaces from "./ControlSurfaces.svelte";
  import MediaPrepSettings from "./MediaPrepSettings.svelte";
  import TranslationSettings from "./TranslationSettings.svelte";
  import CcliReport from "../reports/CcliReport.svelte";
  // Settings ▸ Bible loads with the Bible page's chunk.
  const bibleSettings = () => import("../bible/BibleSettings.svelte");
  import { formatWhen } from "../site/logic";
  import ChunkError from "../../components/ChunkError.svelte";

  const SECTIONS = [
    { id: "General", label: "General" },
    { id: "Audio", label: "Audio" },
    { id: "Captions", label: "Captions" },
    { id: "Network", label: "Network & IDN" },
    { id: "Streaming", label: "Recording & streaming" },
    { id: "Media", label: "Media" },
    { id: "Reports", label: "Reports" },
    { id: "Keyboard", label: "Keyboard" },
    { id: "Bible", label: "Bible" },
    { id: "Translation", label: "Translation" },
    { id: "About", label: "About" },
  ];
  const desktop = isTauri();
  /** Why a section can't be opened (Bible: the server has no scripture), else null. */
  function sectionOff(s: { id: string }): string | null {
    if (s.id === "Media" && !siteState.has("codex")) return "This server has no Codex library (update the server)";
    if (s.id === "Translation" && !siteState.has("translate")) return "This server has no machine translation (update the server)";
    return s.id === "Bible" && !siteState.has("scripture") ? "This server has no Bible yet (update the server)" : null;
  }
  // A section this Oriel doesn't have (an old link or menu): General.
  let section = $derived(SECTIONS.some((s) => s.id === shell.settingsSection && !sectionOff(s)) ? shell.settingsSection : "General");
  let transports = $state.raw<OutputTransportInfo[] | null>(null);
  let others = $derived(sitePage.backups.filter((b) => b.set.product === "oriel" && !b.yours && b.latest));

  onMount(() => {
    void session.conn?.rest
      .outputTransports()
      .then((t) => (transports = t))
      .catch(() => (transports = []));
    void orielPrefs.load();
    if (siteState.has("codex")) void sitePage.loadBackups();
  });

  function transportLabel(t: OutputTransportInfo): string {
    return /idn|ndi/i.test(t.kind) ? "IDN Open Protocol (compatible with NDI®)" : t.label;
  }
</script>

<PageFrame page="settings">
  <div class="body">
    <nav aria-label="Settings sections">
      {#each SECTIONS as s (s.id)}
        <button type="button" class:cur={s.id === section} disabled={!!sectionOff(s)} title={sectionOff(s) ?? undefined} aria-current={s.id === section ? "page" : undefined} onclick={() => (shell.settingsSection = s.id)}>{s.label}</button>
      {/each}
    </nav>
    <div class="main">
      {#if section === "General"}
        <h1>General</h1>
        <section class="card">
          <h2>This computer</h2>
          <dl>
            <dt>Connected to</dt><dd>{session.server?.name ?? "–"} <span class="meta">{session.baseUrl}</span></dd>
            <dt>This client</dt><dd>{session.server?.auth === "local" ? "On the server machine (trusted)" : "Paired with the site"}</dd>
            <dt>App</dt><dd>{desktop ? "Desktop app" : "Browser"}</dd>
          </dl>
        </section>
        <section class="card">
          <h2>Slides and panels</h2>
          <div class="row">
            <span>Slide thumbnails</span>
            <Button size="sm" onclick={() => showView.thumbSmaller()}>Smaller</Button>
            <span class="meta num">{showView.thumbPx} px</span>
            <Button size="sm" onclick={() => showView.thumbLarger()}>Larger</Button>
          </div>
          <div class="row"><span>Panel sizes</span><Button size="sm" onclick={() => showView.resetLayout()}>Reset panel sizes</Button></div>
        </section>
        <section class="card" data-testid="prefs-backup">
          <h2>Oriel's settings backup</h2>
          {#if !siteState.has("codex")}
            <span class="meta">This server has no Codex: Oriel's settings stay on this computer.</span>
          {:else}
            <span class="meta">Panel sizes, views, bins and presets are saved to the site's Codex a few seconds after they change, so a new computer can take them over.</span>
            <div class="row">
              <span>Last saved</span>
              <span>{orielPrefs.lastSaved ? formatWhen(orielPrefs.lastSaved) : orielPrefs.backup ? formatWhen(orielPrefs.backup.snapshot.takenAt) : "Not yet"}</span>
            </div>
            {#if orielPrefs.problem}<span class="err">Not saved: {orielPrefs.problem}</span>{/if}
            {#if orielPrefs.elsewhere}<div class="row"><Button size="sm" onclick={() => orielPrefs.useCodexBox()}>Use the Codex box</Button></div>{/if}
            <div class="row">
              <Button size="sm" variant="primary" onclick={() => orielPrefs.restoreOwn()} disabled={!orielPrefs.backup}>Restore my Oriel settings</Button>
              {#each others as b (b.set.id)}
                <Button size="sm" onclick={() => orielPrefs.restoreFrom(b.set.id, b.latest!.id)}>Take over {b.set.name ?? "another computer"}'s settings</Button>
              {/each}
            </div>
            <span class="meta">Restoring reloads Oriel; the show keeps running on the server.</span>
          {/if}
        </section>
      {:else if section === "Audio"}
        <h1>Audio</h1>
        <section class="card"><AudioSettings /></section>
      {:else if section === "Captions"}
        <h1>Captions</h1>
        <section class="card"><CaptionsSettings /></section>
      {:else if section === "Network"}
        <h1>Network & IDN</h1>
        <section class="card">
          <h2>Server</h2>
          <dl>
            <dt>Name</dt><dd>{session.server?.name ?? "–"}</dd>
            <dt>Address</dt><dd>{session.baseUrl}</dd>
            <dt>Version</dt><dd>{session.server?.version ?? "–"}</dd>
            <dt>Protocol</dt><dd>{session.server?.protocolVersion ?? "–"} (Oriel speaks {PROTOCOL_VERSION})</dd>
            <dt>Codex</dt><dd>{siteState.codex ? `${siteState.codex.role === "host" ? "Holds the site's Codex" : "Show box"} · ${siteState.codex.collectionName}` : "–"}</dd>
          </dl>
        </section>
        <section class="card">
          <h2>Output transports</h2>
          {#if transports === null}
            <span class="meta">Reading…</span>
          {:else if !transports.length}
            <span class="meta">The server doesn't report its output transports.</span>
          {:else}
            <dl>
              {#each transports as t (t.kind)}
                <dt>{transportLabel(t)}</dt>
                <dd>{t.available ? "Available" : t.supported ? "Not available" : "Not supported"}{t.version ? ` · library ${t.version}` : ""}{t.detail ? ` · ${t.detail}` : ""}</dd>
              {/each}
            </dl>
          {/if}
        </section>
        {#if siteState.has("remoteCompat")}
          <section class="card" data-testid="control-surfaces">
            <h2>Control surfaces</h2>
            <ControlSurfaces />
          </section>
        {/if}
      {:else if section === "Streaming"}
        <h1>Recording & streaming</h1>
        <StreamingSettings />
      {:else if section === "Media"}
        <h1>Media</h1>
        <MediaPrepSettings />
      {:else if section === "Reports"}
        <h1>Reports</h1>
        <CcliReport />
      {:else if section === "Keyboard"}
        <h1>Keyboard</h1>
        <p class="meta">The shortcuts can't be changed yet. Keys are matched by their place on the keyboard, so they work on every layout.</p>
        {#each SHORTCUT_GROUPS as g (g)}
          <section class="card keys">
            <h2>{g}</h2>
            {#each rowsOf(g) as r (r.action)}
              <div class="krow" class:dim={r.desktopOnly && !desktop}>
                <span>{r.action}{#if r.note}<span class="meta">&nbsp;· {r.note}</span>{/if}</span>
                <span>{#each r.keys as k, i (k)}{#if i > 0}<span class="meta">&nbsp;or&nbsp;</span>{/if}<kbd>{k}</kbd>{/each}</span>
              </div>
            {/each}
          </section>
        {/each}
      {:else if section === "Translation"}
        <h1>Translation</h1>
        <TranslationSettings />
      {:else if section === "Bible"}
        <h1>Bible</h1>
        {#await bibleSettings() then m}<m.default />{:catch e}<ChunkError error={e} what="The Bible settings" />{/await}
      {:else if section === "About"}
        <h1>About</h1>
        <section class="card about">
          <Logo size={56} />
          <h2>{APP_FULL_NAME}</h2>
          <span>Open-source presentation software for churches.</span>
          <dl>
            <dt>Protocol</dt><dd>{PROTOCOL_VERSION}</dd>
            <dt>Server</dt><dd>{session.server ? `${session.server.name} (${session.server.version})` : "not connected"}</dd>
            <dt>Network video</dt><dd>IDN Open Protocol (compatible with NDI®)</dd>
            <dt>Font</dt><dd>Inter, SIL Open Font License</dd>
          </dl>
          <div class="row"><Button size="sm" onclick={() => (shell.showDiagnostics = true)}>Diagnostics…</Button></div>
        </section>
      {/if}
    </div>
  </div>
</PageFrame>

<style>
  .body {
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
  }
  nav button {
    padding: 9px 10px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
    text-align: left;
    font-size: var(--fs);
    cursor: pointer;
  }
  nav button:hover:not(:disabled) {
    background: var(--surface-raised);
  }
  nav button.cur {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  nav button:disabled {
    color: var(--text-disabled);
    cursor: default;
  }
  .main {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 16px 20px 28px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    max-width: 920px;
  }
  .main > :global(*) {
    flex-shrink: 0;
  }
  h1 {
    margin: 0;
    font-size: 18px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px;
    border-radius: var(--r-card);
    background: var(--surface-panel);
    border: 1px solid var(--line);
  }
  h2 {
    margin: 0;
    font-size: var(--fs);
  }
  dl {
    display: grid;
    grid-template-columns: 160px 1fr;
    gap: 8px 16px;
    margin: 0;
  }
  dt {
    color: var(--text-label);
  }
  dd {
    margin: 0;
    user-select: text;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
  }
  .row > span:first-child {
    min-width: 140px;
    color: var(--text-label);
  }
  .meta {
    color: var(--text-meta);
  }
  p.meta {
    margin: 0;
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  .err {
    color: var(--danger-text);
  }
  .krow {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    padding: 4px 0;
    border-bottom: 1px solid var(--surface-raised);
  }
  .krow:last-child {
    border-bottom: 0;
  }
  .krow.dim {
    opacity: 0.55;
  }
  kbd {
    display: inline-block;
    min-width: 20px;
    padding: 1px 6px;
    border-radius: var(--r-xs);
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    font-family: var(--font);
    font-size: var(--fs-sm);
    text-align: center;
  }
  .about {
    align-items: center;
    text-align: center;
  }
  .about dl {
    text-align: left;
    margin-top: 8px;
  }
</style>
