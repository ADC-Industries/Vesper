<script lang="ts">
  // Site ▸ Signage (docs/SIGNAGE-PLAN.md): the site's signage channels, rendered by the site's
  // server, each a screen of it with an IDN output named after it (Mosaic screens follow it like
  // any source). Channels with a live thumbnail, now/next and health; the selected channel's
  // layout, default playlist, ticker and its week calendar (SignageCalendar); the playlists
  // (SignagePlaylists), the feeds and settings (SignageFeeds); and the Override button: an urgent
  // message on all or chosen channels at once, confirmed first, cleared here.
  import { onDestroy, onMount, untrack } from "svelte";
  import Siren from "lucide-svelte/icons/siren";
  import type { SignageLayout, SignageOverrideStyle } from "$protocol";
  import { Button, Dialog, Segmented, Select, StatusDot } from "../../ui";
  import { ICON_SIZE, ICON_STROKE } from "../../ui/types";
  import { signagePage } from "../../state/signage.svelte";
  import { LAYOUTS, channelStatus, layoutZones, nowLabel } from "./signage";
  import SignageCalendar from "./SignageCalendar.svelte";
  import SignagePlaylists from "./SignagePlaylists.svelte";
  import SignageFeeds from "./SignageFeeds.svelte";

  onMount(() => signagePage.open());
  onDestroy(() => signagePage.close());

  let st = $derived(signagePage.state);
  let channels = $derived(st?.channels ?? []);
  let playlists = $derived(st?.playlists ?? []);
  let feeds = $derived(st?.feeds ?? []);
  let selId = $state<string | null>(null);
  let sel = $derived(channels.find((c) => c.id === selId) ?? channels[0] ?? null);
  let broken = $state<Record<string, number>>({});

  let newName = $state("");
  async function create() {
    if (!newName.trim()) return;
    const c = await signagePage.createChannel(newName.trim());
    if (c) {
      newName = "";
      selId = c.id;
    }
  }

  let rename = $state("");
  let ticker = $state("");
  // The drafts start from the selected channel and follow the server's value only while not
  // being edited: the 4 s poll never wipes what the admin is typing (H10-2).
  let draftFor: string | null = null;
  let seenName = "";
  let seenTicker = "";
  $effect(() => {
    const id = sel?.id ?? null;
    const name = sel?.name ?? "";
    const tick = sel?.ticker ?? "";
    untrack(() => {
      if (id !== draftFor) {
        draftFor = id;
        rename = name;
        ticker = tick;
      } else {
        if (name !== seenName && rename === seenName) rename = name;
        if (tick !== seenTicker && ticker === seenTicker) ticker = tick;
      }
      seenName = name;
      seenTicker = tick;
    });
  });
  let confirmDelete = $state(false);

  // The override dialog.
  let overOpen = $state(false);
  let overText = $state("");
  let overStyle = $state<SignageOverrideStyle>("urgent");
  let overAll = $state(true);
  let overChosen = $state<string[]>([]);
  let overFor = $state("0");
  let overConfirm = $state(false);
  const DURATIONS = [
    { value: "0", label: "Until cleared" },
    { value: "300", label: "5 minutes" },
    { value: "900", label: "15 minutes" },
    { value: "3600", label: "1 hour" },
  ];
  function openOverride() {
    overText = "";
    overStyle = "urgent";
    overAll = true;
    overChosen = [];
    overFor = "0";
    overConfirm = false;
    overOpen = true;
  }
  async function pushOverride() {
    const seconds = Number(overFor);
    const r = await signagePage.pushOverride({ text: overText.trim(), style: overStyle, channels: overAll ? [] : overChosen, ...(seconds ? { seconds } : {}) });
    if (r) overOpen = false;
  }
  let over = $derived(st?.override ?? null);
  let overNames = $derived(over ? (over.channels.length ? over.channels.map((id) => channels.find((c) => c.id === id)?.name ?? "a deleted channel").join(", ") : "every channel") : "");
  const playlistChoices = $derived([{ value: "", label: "Nothing (zones only)" }, ...playlists.map((p) => ({ value: p.id, label: p.name }))]);
  const feedChoices = $derived([{ value: "", label: "No feed" }, ...feeds.map((f) => ({ value: f.id, label: f.name }))]);
</script>

<div class="st-head">
  <h1>Signage</h1>
  <span class="sub">{channels.length} channel{channels.length === 1 ? "" : "s"} · {playlists.length} playlist{playlists.length === 1 ? "" : "s"} · {feeds.length} feed{feeds.length === 1 ? "" : "s"}</span>
  <span class="st-grow"></span>
  <Button variant="danger" icon={Siren} onclick={openOverride} disabled={!channels.length || !st?.managed} data-testid="signage-override">Override</Button>
</div>

{#if signagePage.problem}<p class="st-problem">Couldn't read the signage: {signagePage.problem}</p>{/if}
{#if st && !st.managed}
  <p class="st-note" data-testid="signage-elsewhere">The site's signage runs on the site's server: open its Site page.</p>
{/if}

{#if over}
  <div class="override" role="status" data-testid="signage-override-on">
    <Siren size={ICON_SIZE} strokeWidth={ICON_STROKE} aria-hidden="true" />
    <span class="st-grow"><b>Override showing</b> on {overNames}: “{over.text}”{over.until ? ` · until ${new Date(over.until).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}` : ""}</span>
    <Button size="sm" onclick={() => signagePage.clearOverride()} data-testid="signage-override-clear">Clear override</Button>
  </div>
{/if}

{#if signagePage.loaded && st?.managed && channels.length === 0}
  <p class="st-note" data-testid="signage-empty">
    No signage channel yet. A channel ("Lobby") is a picture this server makes from a schedule of playlists, with a clock, a countdown, the weather and a ticker around it, sent on the network as an IDN source named after it. Point Mosaic's displays at it in Displays.
  </p>
{/if}

{#if channels.length}
  <div class="channels" data-testid="signage-channels">
    {#each channels as c (c.id)}
      {@const s = channelStatus(c.now)}
      {@const url = signagePage.thumbnail(c.source)}
      <button type="button" class="chan" class:cur={sel?.id === c.id} aria-pressed={sel?.id === c.id} onclick={() => (selId = c.id)} data-testid="signage-channel">
        <span class="thumb">
          {#if url && broken[c.id] !== signagePage.tick}
            <img src={url} alt="" loading="lazy" onerror={() => (broken = { ...broken, [c.id]: signagePage.tick })} />
          {:else}
            <span class="none">No picture yet</span>
          {/if}
          {#if c.now?.overridden}<span class="ov">OVERRIDE</span>{/if}
        </span>
        <span class="info">
          <span class="nm">{c.name}</span>
          <span class="st-meta" title={c.source}>{c.source || "IDN source starting…"}</span>
          <span class="line">Now: {c.now?.item ?? "—"}</span>
          <span class="line st-meta">Next: {c.now?.next ?? "—"}</span>
          <span class="line st-meta">{nowLabel(c, playlists)}</span>
          <span title={s.label}><StatusDot status={s.status} label={s.label} /></span>
        </span>
      </button>
    {/each}
  </div>
{/if}

{#if st?.managed}
  <div class="st-row newch">
    <input type="text" placeholder="New channel name (e.g. Lobby)" bind:value={newName} onkeydown={(e) => e.key === "Enter" && create()} aria-label="New channel name" data-testid="signage-new-name" />
    <Button size="sm" variant="primary" onclick={create} disabled={!newName.trim()}>New channel</Button>
  </div>
{/if}

{#if sel}
  <div class="st-card" data-testid="signage-detail">
    <div class="st-row">
      <h2>{sel.name}</h2>
      <span class="st-meta">screen “Signage: {sel.name}” · IDN source {sel.source || "starting"}</span>
      <span class="st-grow"></span>
      {#if confirmDelete}
        <span>Delete {sel.name}, its screen and output?</span>
        <Button size="sm" variant="danger" onclick={async () => { confirmDelete = false; if (sel) await signagePage.deleteChannel(sel.id); }}>Delete</Button>
        <Button size="sm" onclick={() => (confirmDelete = false)}>Keep</Button>
      {:else}
        <Button size="sm" onclick={() => (confirmDelete = true)}>Delete…</Button>
      {/if}
    </div>
    <div class="fields">
      <label>Name<input type="text" bind:value={rename} onchange={() => sel && rename.trim() && rename.trim() !== sel.name && signagePage.putChannel(sel.id, { name: rename.trim() })} /></label>
      <label>Default playlist<Select label="Default playlist" options={playlistChoices} value={sel.defaultPlaylist ?? ""} onchange={(v) => sel && signagePage.putChannel(sel.id, { defaultPlaylist: v })} data-testid="signage-default" /></label>
      <label class="wide">Ticker<input type="text" bind:value={ticker} placeholder="Text that scrolls along the bottom" onchange={() => sel && signagePage.putChannel(sel.id, { ticker })} /></label>
      <label>Ticker feed<Select label="Ticker feed" options={feedChoices} value={sel.tickerFeed ?? ""} onchange={(v) => sel && signagePage.putChannel(sel.id, { tickerFeed: v })} /></label>
    </div>
    <div class="layouts" role="radiogroup" aria-label="Zone layout" data-testid="signage-layouts">
      {#each LAYOUTS as l (l.value)}
        {@const z = layoutZones(l.value)}
        <button type="button" role="radio" aria-checked={sel.layout === l.value} class="layout" class:cur={sel.layout === l.value} onclick={() => sel && signagePage.putChannel(sel.id, { layout: l.value as SignageLayout })}>
          <span class="pic">
            <span class="z main" style:left="{z.main[0]}%" style:top="{z.main[1]}%" style:width="{z.main[2]}%" style:height="{z.main[3]}%"></span>
            {#if z.sidebar}<span class="z side" style:left="{z.sidebar[0]}%" style:top="{z.sidebar[1]}%" style:width="{z.sidebar[2]}%" style:height="{z.sidebar[3]}%"></span>{/if}
            {#if z.ticker}<span class="z tick" style:left="{z.ticker[0]}%" style:top="{z.ticker[1]}%" style:width="{z.ticker[2]}%" style:height="{z.ticker[3]}%"></span>{/if}
          </span>
          <span class="st-meta">{l.label}</span>
        </button>
      {/each}
    </div>
    <SignageCalendar channel={sel} {playlists} />
  </div>
{/if}

{#if st?.managed}
  <SignagePlaylists {playlists} {feeds} />
  <SignageFeeds {feeds} settings={st.settings} weather={st.weather ?? null} />
{/if}

{#if overOpen}
  <Dialog title="Override signage" onclose={() => (overOpen = false)} width={520}>
    <div class="ovform" data-testid="signage-override-dialog">
      <label>Message<textarea rows="3" bind:value={overText} placeholder="Please leave the building by the nearest exit" maxlength="500"></textarea></label>
      <div class="st-row"><span class="lbl">Look</span><Segmented label="Look" options={[{ value: "urgent", label: "Urgent (red)" }, { value: "info", label: "Notice (blue)" }]} value={overStyle} onchange={(v) => (overStyle = v as SignageOverrideStyle)} /></div>
      <div class="st-row">
        <span class="lbl">Channels</span>
        <label class="chk"><input type="checkbox" bind:checked={overAll} /> All channels</label>
        {#if !overAll}
          {#each channels as c (c.id)}
            <label class="chk"><input type="checkbox" checked={overChosen.includes(c.id)} onchange={(e) => (overChosen = e.currentTarget.checked ? [...overChosen, c.id] : overChosen.filter((x) => x !== c.id))} /> {c.name}</label>
          {/each}
        {/if}
      </div>
      <div class="st-row"><span class="lbl">For</span><Select label="How long" options={DURATIONS} bind:value={overFor} /></div>
      {#if overConfirm}
        <p class="st-problem">This replaces what {overAll ? "every channel" : "the chosen channels"} show{overAll ? "s" : ""} at once. Show it?</p>
      {/if}
    </div>
    {#snippet footer()}
      <Button onclick={() => (overOpen = false)}>Cancel</Button>
      {#if overConfirm}
        <Button variant="danger" onclick={pushOverride} data-testid="signage-override-go">Show it now</Button>
      {:else}
        <Button variant="danger" onclick={() => (overConfirm = true)} disabled={!overText.trim() || (!overAll && !overChosen.length)}>Override…</Button>
      {/if}
    {/snippet}
  </Dialog>
{/if}

<style>
  .override {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-radius: var(--r-card);
    background: var(--danger-bg);
    color: var(--text-strong);
  }
  .channels {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(250px, 1fr));
    gap: 12px;
  }
  .chan {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border: 1px solid var(--line);
    border-radius: var(--r-card);
    background: var(--surface-panel);
    color: inherit;
    text-align: left;
    cursor: pointer;
    min-width: 0;
  }
  .chan.cur {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .thumb {
    position: relative;
    aspect-ratio: 16 / 9;
    background: #000;
    border-radius: var(--r);
    overflow: hidden;
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
  .ov {
    position: absolute;
    top: 6px;
    left: 6px;
    padding: 2px 6px;
    border-radius: 4px;
    background: var(--danger-bg);
    color: #fff;
    font-size: 10.5px;
    font-weight: 800;
    letter-spacing: 0.06em;
  }
  .info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .nm {
    font-weight: 700;
  }
  .info > span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .newch input {
    width: 260px;
  }
  .fields {
    display: flex;
    flex-wrap: wrap;
    gap: 10px 14px;
  }
  .fields label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-sm);
    color: var(--text-label);
  }
  .fields label.wide {
    flex: 1;
    min-width: 260px;
  }
  .layouts {
    display: flex;
    flex-wrap: wrap;
    gap: 10px;
  }
  .layout {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 6px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
  .layout.cur {
    border-color: var(--accent);
    background: var(--accent-sel);
  }
  .pic {
    position: relative;
    width: 96px;
    height: 54px;
    background: var(--surface-raised);
  }
  .z {
    position: absolute;
    box-sizing: border-box;
    border: 1px solid var(--surface-panel);
  }
  .z.main {
    background: var(--accent);
    opacity: 0.55;
  }
  .z.side {
    background: var(--text-meta);
  }
  .z.tick {
    background: #e0a43a;
  }
  .ovform {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .ovform label {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ovform label.chk {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  .ovform textarea {
    font: inherit;
    font-size: 15px;
  }
  .lbl {
    min-width: 70px;
    color: var(--text-label);
  }
</style>
