<script lang="ts">
  // Site ▸ Displays (Vesper Mosaic, docs/MOSAIC-PLAN.md): every screen of the site's display
  // boxes (IDN Nodes running IDN's display service) as a grid with a thumbnail, what it shows and
  // its state; click one to give it a source or a channel, identify it, put a test pattern up
  // and change its settings. Identify all / test pattern on all. The channels (site-wide named
  // feeds: create, rename, set the source, delete). This server's own displays, read-only: they
  // are Vesper's outputs (Configure Screens). A machine where a Vesper server and the display
  // service both run is flagged: two programs would drive the same screens. Video walls and
  // their designer (WallsSection): the grid's screens drag into a wall's cells. A box's address
  // is set in the screen's panel (any box: one that moved keeps its screens listed, X1-3) or,
  // for a box never read, in "Not reached yet". It comes from the site (where the Site Manager
  // sees the box's IDN Node); one typed here overrides it (M3-2).
  import { onDestroy, onMount } from "svelte";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import MonitorIcon from "lucide-svelte/icons/monitor";
  import type { MosaicFallback, MosaicFit, MosaicScreenSettings, MosaicServiceMode } from "$protocol";
  import { Button, Select, StatusDot } from "../../ui";
  import { ICON_SIZE, ICON_STROKE } from "../../ui/types";
  import { mosaicPage } from "../../state/mosaic.svelte";
  import { shell } from "../../state/shell.svelte";
  import { addressNote, boxLogoNote, logoNote, serviceNote, updatesNote, boxesWithoutScreens, conflicts, displaySources, displayTiles, editableSettings, feedChoices, feedLabel, feedValue, ownDisplays, parseFeed, sameSettings, SCREEN_DRAG, screenLabel, screenStatus, settingsToSave, type DisplayTile } from "./mosaic";
  import WallsSection from "./WallsSection.svelte";

  onMount(() => mosaicPage.open());
  onDestroy(() => mosaicPage.close());

  let st = $derived(mosaicPage.state);
  let tiles = $derived(displayTiles(st));
  let channels = $derived(st?.channels ?? []);
  let waiting = $derived(boxesWithoutScreens(st));
  let clashes = $derived(conflicts(st));
  let own = $derived(ownDisplays(mosaicPage.displays, mosaicPage.outputs, mosaicPage.screens));
  let sources = $derived(displaySources(mosaicPage.sources));
  let boxCount = $derived(new Set(tiles.map((t) => t.box.machine)).size + waiting.length);
  let anyPattern = $derived(tiles.some((t) => t.screen.testPattern));
  let walls = $derived(st?.walls ?? []);
  /** "Front · R1 C2" for a screen that is a wall's tile. */
  function tileOf(t: DisplayTile): string | null {
    const w = t.screen.wall ? walls.find((x) => x.id === t.screen.wall) : undefined;
    const c = w?.cells.find((x) => x.machine === t.box.machine && x.screen === t.screen.number);
    return w && c ? `${w.name} · R${c.row + 1} C${c.column + 1}` : null;
  }

  let selKey = $state<string | null>(null);
  let sel = $derived<DisplayTile | null>(tiles.find((t) => t.key === selKey) ?? null);
  /** The settings being edited, and the server's settings they were copied from (H08-1): a
   * screen selected again starts from the server's settings, and so does an unchanged draft
   * when they change (another admin, a wall made or deleted). Never the wall's tile. */
  let draft = $state<MosaicScreenSettings | null>(null);
  let draftBase = $state<MosaicScreenSettings | null>(null);
  let draftKey = $state<string | null>(null);
  $effect(() => {
    if (!sel) {
      draftKey = null;
      draft = draftBase = null;
      return;
    }
    const cur = sel.screen.settings;
    const stale = draftKey !== sel.key || !draft || !draftBase;
    if (stale || (!sameSettings(cur, draftBase!) && sameSettings(draft!, draftBase!))) {
      draft = editableSettings(cur);
      draftBase = editableSettings(cur);
      draftKey = sel.key;
    }
  });
  let broken = $state<Record<string, number>>({});

  let newName = $state("");
  let newSource = $state("");
  let renaming = $state<string | null>(null);
  let renameTo = $state("");
  let addresses = $state<Record<string, string>>({});

  const FIT: { value: MosaicFit; label: string }[] = [
    { value: "fit", label: "Fit (black around)" },
    { value: "fill", label: "Fill (edges cut)" },
    { value: "stretch", label: "Stretch" },
  ];
  const ROTATION = [0, 90, 180, 270].map((r) => ({ value: String(r), label: `${r}°` }));
  const FALLBACK: { value: MosaicFallback; label: string }[] = [
    { value: "logo", label: "The site's logo" },
    { value: "last", label: "The last picture" },
    { value: "black", label: "Black" },
  ];

  function serviceChoices(headlessOk: boolean, current: MosaicServiceMode | null | undefined) {
    const o: { value: string; label: string }[] = [];
    if (!current) o.push({ value: "", label: "Not switched from Vesper" });
    if (headlessOk || current === "headless") o.push({ value: "headless", label: "Headless from boot (Linux, recommended)" });
    o.push({ value: "desktop", label: "In the desktop session" }, { value: "off", label: "Off" });
    return o;
  }
  let logoInput = $state<HTMLInputElement | null>(null);
  let logoUrl = $derived(st?.logo ? mosaicPage.logoUrl() : null);

  function choose(t: DisplayTile, v: string) {
    void mosaicPage.setFeed(t.box.machine, t.screen.number, parseFeed(v));
  }
  async function saveSettings() {
    if (!sel || !draft || !draftBase) return;
    const key = sel.key;
    const body = settingsToSave(draftBase, draft, sel.screen.settings);
    const r = await mosaicPage.setSettings(sel.box.machine, sel.screen.number, body);
    // Saved: the draft is what the server has now, so its next change is picked up.
    if (r !== undefined && draftKey === key) {
      draft = { ...body };
      draftBase = { ...body };
    }
  }
  async function create() {
    if (!newName.trim()) return;
    const c = await mosaicPage.createChannel(newName.trim(), newSource);
    if (c) {
      newName = "";
      newSource = "";
    }
  }
  const sourceChoices = $derived([{ value: "", label: "No source" }, ...sources.map((s) => ({ value: s, label: s }))]);
  function sourceChoicesFor(current: string) {
    return current && !sources.includes(current) ? [...sourceChoices, { value: current, label: current }] : sourceChoices;
  }
</script>

<div class="st-head">
  <h1>Displays</h1>
  <span class="sub">{tiles.length} screen{tiles.length === 1 ? "" : "s"} on {boxCount} display box{boxCount === 1 ? "" : "es"} · {channels.length} channel{channels.length === 1 ? "" : "s"}</span>
  <span class="st-grow"></span>
  <Button size="sm" onclick={() => mosaicPage.identifyAll()} disabled={!tiles.length}>Identify all</Button>
  <Button size="sm" onclick={() => mosaicPage.testPatternAll(!anyPattern)} disabled={!tiles.length}>{anyPattern ? "Clear test patterns" : "Test pattern on all"}</Button>
</div>

{#if mosaicPage.problem}<p class="st-problem">Couldn't read the displays: {mosaicPage.problem}</p>{/if}

{#if st && !st.managed}
  <p class="st-note" data-testid="mosaic-elsewhere">The site's displays are managed on the site's server's Site page.</p>
{/if}

{#each clashes as b (b.machine)}
  <div class="conflict" role="status" data-testid="mosaic-conflict">
    <TriangleAlert size={ICON_SIZE} strokeWidth={ICON_STROKE} aria-hidden="true" />
    <span><b>{b.name}</b>: {b.conflict}</span>
  </div>
{/each}

{#if mosaicPage.loaded && st?.managed && tiles.length === 0 && waiting.length === 0}
  <p class="st-note" data-testid="mosaic-empty">
    No display box in the site yet. A display box is a small computer behind a projector or TV running the IDN Node with its display service on (Launcher role "display box (Mosaic)"); once it has joined the site, its screens appear here.
  </p>
{/if}

{#if tiles.length}
  <div class="grid" data-testid="mosaic-grid">
    {#each tiles as t (t.key)}
      {@const s = screenStatus(t.screen)}
      {@const url = t.screen.showing && !t.box.vesper && t.box.reachable ? mosaicPage.thumbnail(t.box.machine, t.screen.number) : null}
      {@const inWall = tileOf(t)}
      <button
        type="button"
        class="tile"
        class:cur={selKey === t.key}
        aria-pressed={selKey === t.key}
        onclick={() => (selKey = selKey === t.key ? null : t.key)}
        draggable="true"
        ondragstart={(e) => {
          e.dataTransfer?.setData(SCREEN_DRAG, t.key);
          e.dataTransfer?.setData("text/plain", t.key);
        }}
        data-testid="mosaic-tile"
      >
        <span class="thumb">
          {#if url && broken[t.key] !== mosaicPage.tick}
            <img src={url} alt="" loading="lazy" onerror={() => (broken = { ...broken, [t.key]: mosaicPage.tick })} />
          {:else}
            <span class="none">{t.screen.status === "offline" ? "Offline" : t.screen.showing ? "No picture" : "Nothing shown"}</span>
          {/if}
          {#if t.screen.testPattern}<span class="tp">TEST</span>{/if}
        </span>
        <span class="info">
          <span class="nm">{t.screen.name || `Screen ${t.screen.number}`}</span>
          <span class="st-meta">{t.box.name} · screen {t.screen.number}{t.screen.width ? ` · ${t.screen.width}×${t.screen.height}` : ""}</span>
          <span class="feed" title={feedLabel(t.screen.feed, channels)}>{feedLabel(t.screen.feed, channels)}</span>
          {#if inWall}<span class="wall-of" data-testid="mosaic-tile-wall">Wall {inWall}</span>{/if}
          <span title={s.title}><StatusDot status={s.status} label={s.label} /></span>
        </span>
      </button>
    {/each}
  </div>
{/if}

{#if sel && draft}
  <div class="st-card detail" data-testid="mosaic-detail">
    <div class="st-row">
      <h2>{screenLabel(sel)}</h2>
      <span class="st-grow"></span>
      <Button size="sm" onclick={() => sel && mosaicPage.identify(sel.box.machine, sel.screen.number)}>Identify</Button>
      <Button size="sm" onclick={() => sel && mosaicPage.testPattern(sel.box.machine, sel.screen.number, !sel.screen.testPattern)}>{sel.screen.testPattern ? "Test pattern off" : "Test pattern"}</Button>
    </div>
    {#if sel.screen.problem}<p class="st-problem">{sel.screen.problem}</p>{/if}
    <div class="st-row">
      <span class="lbl">Shows</span>
      <Select label="What this screen shows" options={feedChoices(sources, channels, sel.screen.feed)} value={feedValue(sel.screen.feed)} onchange={(v) => sel && choose(sel, v)} disabled={!!sel.screen.wall} data-testid="mosaic-feed" />
      {#if tileOf(sel)}<span class="st-meta">A tile of the wall {tileOf(sel)}: it shows the wall's feed (Video walls, below).</span>{/if}
    </div>
    <div class="settings">
      <label>Picture<Select label="Picture" options={FIT} bind:value={draft.fit} /></label>
      <label>Rotation<Select label="Rotation" options={ROTATION} value={String(draft.rotation)} onchange={(v) => draft && (draft.rotation = Number(v))} /></label>
      <label>Overscan %<input type="number" min="0" max="20" step="0.5" bind:value={draft.overscan} /></label>
      <label>When the feed goes<Select label="When the feed goes" options={FALLBACK} bind:value={draft.fallback} /></label>
      <label>After (s)<input type="number" min="0" max="3600" bind:value={draft.fallbackAfterSeconds} /></label>
      <Button size="sm" variant="primary" onclick={saveSettings}>Save settings</Button>
    </div>
    <div class="st-row" data-testid="mosaic-box-address">
      <span class="lbl">Address</span>
      <input type="text" class="addr" placeholder="From the site" value={addresses[sel.box.machine] ?? (sel.box.addressFrom === "admin" ? sel.box.address : "") ?? ""} oninput={(e) => sel && (addresses = { ...addresses, [sel.box.machine]: e.currentTarget.value })} aria-label={`${sel.box.name}'s address (override)`} />
      <Button size="sm" onclick={() => sel && mosaicPage.setAddress(sel.box.machine, addresses[sel.box.machine] ?? (sel.box.addressFrom === "admin" ? (sel.box.address ?? "") : ""))}>Set address</Button>
      {#if sel.box.addressFrom === "admin"}<Button size="sm" onclick={() => sel && ((addresses = { ...addresses, [sel.box.machine]: "" }), mosaicPage.setAddress(sel.box.machine, ""))}>Use the site's</Button>{/if}
      <span class="st-meta addr-from" data-testid="mosaic-address-from">{sel.box.address ? `${sel.box.address} · ` : ""}{addressNote(sel.box)}</span>
    </div>
    <div class="st-row" data-testid="mosaic-box-service">
      <span class="lbl">Service</span>
      <Select label={`${sel.box.name}'s display service`} options={serviceChoices(sel.box.headlessOk ?? false, sel.box.serviceMode)} value={sel.box.serviceMode ?? ""} onchange={(v) => sel && v && mosaicPage.setService(sel.box.machine, v as MosaicServiceMode)} />
      <span class="st-meta">{serviceNote(sel.box)}</span>
    </div>
    {#if updatesNote(sel.box)}
      <div class="st-row" data-testid="mosaic-box-updates">
        <span class="lbl">IDN Node</span>
        <span class="st-meta" title="Vesper's package installs and updates this box's IDN Node; the Site Manager leaves it out of its updates">{updatesNote(sel.box)}</span>
      </div>
    {/if}
    {#if st?.logo}
      <div class="st-row" data-testid="mosaic-box-logo">
        <span class="lbl">Logo</span>
        <StatusDot status={sel.box.logoState === "onBox" ? "ok" : sel.box.logoState === "failed" ? "error" : sel.box.logoState === "sending" ? "info" : "off"} label={boxLogoNote(sel.box)} />
      </div>
    {/if}
  </div>
{/if}

{#if waiting.length}
  <h2 class="st-section">Not reached yet</h2>
  <div class="st-table-wrap">
    <table class="st" data-testid="mosaic-waiting">
      <thead><tr><th>Display box</th><th>Why</th><th>Address</th><th aria-label="Actions"></th></tr></thead>
      <tbody>
        {#each waiting as b (b.machine)}
          <tr>
            <td><span class="st-two"><span class="st-strong">{b.name}</span><span class="st-meta">Display service: {b.service || "added by hand"}</span></span></td>
            <td class="why">{b.problem ?? "Not read yet"}</td>
            <td><span class="st-two"><input type="text" placeholder={b.address ?? "From the site"} value={addresses[b.machine] ?? (b.addressFrom === "admin" ? b.address : "") ?? ""} oninput={(e) => (addresses = { ...addresses, [b.machine]: e.currentTarget.value })} aria-label={`${b.name}'s address (override)`} /><span class="st-meta">{addressNote({ ...b, reachable: true })}</span></span></td>
            <td class="actions"><Button size="sm" onclick={() => mosaicPage.setAddress(b.machine, addresses[b.machine] ?? (b.addressFrom === "admin" ? (b.address ?? "") : ""))}>Set address</Button></td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

{#if st?.managed && st.machines?.length}
  <h2 class="st-section">Other machines of the site</h2>
  <p class="st-note">Machines with an IDN Node that aren't display boxes yet. "Use as a display box" switches their IDN display service on; headless (no desktop, from boot) is recommended for a dedicated Linux box.</p>
  <div class="st-table-wrap">
    <table class="st" data-testid="mosaic-machines">
      <thead><tr><th>Machine</th><th>Health</th><th aria-label="Actions"></th></tr></thead>
      <tbody>
        {#each st.machines as m (m.machine)}
          <tr>
            <td><span class="st-strong">{m.name}</span></td>
            <td><StatusDot status={m.health === "online" ? "ok" : m.health === "offline" ? "error" : "warn"} label={m.health === "needsAttention" ? "Needs attention" : m.health.charAt(0).toUpperCase() + m.health.slice(1)} /></td>
            <td class="actions">
              {#if m.headlessOk}<Button size="sm" variant="primary" onclick={() => mosaicPage.setService(m.machine, "headless")}>Use as a display box (headless)</Button>{/if}
              <Button size="sm" onclick={() => mosaicPage.setService(m.machine, "desktop")}>{m.headlessOk ? "With a desktop" : "Use as a display box"}</Button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<WallsSection {tiles} {channels} {sources} {walls} />

{#if st?.managed}
  <h2 class="st-section">Site logo</h2>
  <div class="st-card" data-testid="mosaic-logo"><div class="logo">
    <span class="logo-preview">
      {#if logoUrl}<img src={logoUrl} alt="The site's logo" />{:else}<span class="none">No logo</span>{/if}
    </span>
    <span class="st-two logo-text">
      <span>What every display box shows when its feed goes (after the fallback delay). Sent to each box's media folder by its IDN Node, so it shows while this server is down too.</span>
      <span class="st-meta" data-testid="mosaic-logo-note">{logoNote(st)}</span>
      {#if st.logoProblem}<span class="st-problem">{st.logoProblem}</span>{/if}
    </span>
    <input bind:this={logoInput} type="file" accept="image/png,image/jpeg" class="hidden-file" onchange={(e) => { const f = e.currentTarget.files?.[0]; if (f) void mosaicPage.setLogo(f); e.currentTarget.value = ""; }} />
    <Button size="sm" onclick={() => logoInput?.click()}>{st.logo ? "Change…" : "Choose a logo…"}</Button>
    {#if st.logo}<Button size="sm" onclick={() => mosaicPage.removeLogo()}>Remove</Button>{/if}
  </div></div>
{/if}

<h2 class="st-section">Channels</h2>
<p class="st-note">A channel is a named feed for the whole site ("Main", "Lobby"): screens follow it, and changing its source moves every one of them. Each channel is an IDN router on this server, so a new source is one switch for all its screens.</p>
{#if st?.routingProblem}<p class="st-problem" data-testid="mosaic-routing-problem">{st.routingProblem}</p>{/if}
<div class="st-table-wrap">
  <table class="st" data-testid="mosaic-channels">
    <thead><tr><th>Channel</th><th>Source</th><th>Screens</th><th aria-label="Actions"></th></tr></thead>
    <tbody>
      {#each channels as c (c.id)}
        <tr>
          <td>
            {#if renaming === c.id}
              <input type="text" bind:value={renameTo} aria-label="New name" onkeydown={(e) => e.key === "Enter" && (mosaicPage.renameChannel(c.id, renameTo), (renaming = null))} />
            {:else}
              <span class="st-strong">{c.name}</span>
            {/if}
          </td>
          <td>
            <span class="st-two">
              <Select label={`${c.name}'s source`} options={sourceChoicesFor(c.source)} value={c.source} onchange={(v) => mosaicPage.setChannelSource(c.id, v)} />
              <span class="st-meta" data-testid="mosaic-channel-router">{c.router ? `Router ${c.router}` : "Set on each screen"}</span>
            </span>
          </td>
          <td class="num">{c.screens}</td>
          <td class="actions">
            {#if renaming === c.id}
              <Button size="sm" variant="primary" onclick={() => (mosaicPage.renameChannel(c.id, renameTo), (renaming = null))}>Save</Button>
              <Button size="sm" onclick={() => (renaming = null)}>Cancel</Button>
            {:else}
              <Button size="sm" onclick={() => ((renaming = c.id), (renameTo = c.name))}>Rename</Button>
              <span title={c.screens ? "Screens follow it: give them another feed first" : undefined}>
                <Button size="sm" onclick={() => mosaicPage.deleteChannel(c.id)} disabled={c.screens > 0}>Delete</Button>
              </span>
            {/if}
          </td>
        </tr>
      {/each}
      <tr class="new">
        <td><input type="text" placeholder="New channel name" bind:value={newName} aria-label="New channel name" onkeydown={(e) => e.key === "Enter" && create()} /></td>
        <td><Select label="New channel's source" options={sourceChoices} bind:value={newSource} /></td>
        <td></td>
        <td class="actions"><Button size="sm" variant="primary" onclick={create} disabled={!newName.trim()}>Create channel</Button></td>
      </tr>
    </tbody>
  </table>
</div>

{#if own.length}
  <h2 class="st-section">On this server: driven by Vesper</h2>
  <div class="st-card own" data-testid="mosaic-own">
    <div class="st-row">
      <span class="st-note">These displays are this server's own outputs (frame-accurate, with layouts, Fill/Key and corner pin). They are set up in Configure Screens, not here.</span>
      <span class="st-grow"></span>
      <Button size="sm" onclick={() => shell.go("screens")}>Configure Screens</Button>
    </div>
    <ul>
      {#each own as d (d.id)}
        <li>
          <MonitorIcon size={ICON_SIZE} strokeWidth={ICON_STROKE} aria-hidden="true" />
          <span class="st-strong">{d.name}</span>
          <span class="st-meta">{d.connected ? (d.drivenBy.length ? d.drivenBy.join(", ") : "Not used by an output") : "Nothing plugged in"}</span>
        </li>
      {/each}
    </ul>
  </div>
{/if}

<style>
  .conflict {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: var(--r);
    border: 1px solid var(--warn);
    color: var(--text);
  }
  .conflict :global(svg) {
    color: var(--warn);
    flex: none;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 10px;
  }
  .tile {
    display: flex;
    flex-direction: column;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r-card);
    background: var(--surface-panel);
    color: var(--text);
    text-align: left;
    cursor: pointer;
    overflow: hidden;
    font: inherit;
  }
  .tile:hover {
    border-color: var(--line-strong);
  }
  .tile.cur {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .thumb {
    position: relative;
    aspect-ratio: 16 / 9;
    background: var(--surface-well);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .none {
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .tp {
    position: absolute;
    top: 6px;
    left: 6px;
    padding: 1px 6px;
    border-radius: 4px;
    background: var(--info, var(--accent));
    color: var(--text-strong);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.06em;
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 8px 10px 10px;
    min-width: 0;
  }
  .nm {
    font-weight: 600;
  }
  .feed {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--text-2);
  }
  .detail {
    margin-top: 4px;
  }
  .wall-of {
    color: var(--accent);
    font-size: var(--fs-sm);
  }
  .lbl {
    color: var(--text-label);
    min-width: 48px;
  }
  .settings {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 10px 14px;
  }
  .settings label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .settings input {
    width: 90px;
  }
  .addr {
    width: 200px;
  }
  .logo {
    display: flex;
    flex-direction: row;
    align-items: center;
    text-align: left;
    gap: 14px;
    flex-wrap: wrap;
  }
  .logo-text {
    flex: 1 1 320px;
    min-width: 0;
  }
  .logo-preview {
    width: 160px;
    aspect-ratio: 16 / 9;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--surface-well);
    border-radius: var(--r);
    overflow: hidden;
  }
  .logo-preview img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
  .hidden-file {
    display: none;
  }
  td.num {
    font-variant-numeric: tabular-nums;
  }
  td.why {
    color: var(--text-2);
  }
  .own ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .own li {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .own li :global(svg) {
    color: var(--text-label);
  }
</style>
