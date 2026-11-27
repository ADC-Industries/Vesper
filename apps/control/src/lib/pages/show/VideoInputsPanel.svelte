<script lang="ts">
  // The bottom bin's Video Inputs tab (G7, docs/LIVE-SOURCES-PLAN.md): the show's logical video
  // inputs ("Camera 1", bound per computer to a source; stage 4), then the network video sources
  // the server sees (IDN Open Protocol, compatible with NDI®), each with a picture refreshed every
  // few seconds while the tab shows, its status, and a switch. Clicking an input or a source puts
  // it on the Video Input layer (under the media layer); an input's menu renames, binds, unbinds
  // or deletes it; "+" adds one by address.
  import Plus from "lucide-svelte/icons/plus";
  import Cctv from "lucide-svelte/icons/cctv";
  import ListFilter from "lucide-svelte/icons/list-filter";
  import MoreHorizontal from "lucide-svelte/icons/more-horizontal";
  import type { LiveSource, LiveSourceStatus, VideoInputEntry } from "$protocol";
  import type { MenuItem } from "../../shell/menu";
  import type { Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import { sourcesState } from "../../state/sources.svelte";
  import { shell } from "../../state/shell.svelte";
  import { useLive } from "../../live";
  import StatusDot from "../../ui/StatusDot.svelte";
  import EmptyState from "../../ui/EmptyState.svelte";
  import type { Status } from "../../ui/types";
  import { inputLine, inputSource, inputStatus, manualSource, sourceHost, sourceName, statusLine, transportTag, visibleSources } from "./sources";

  let { conn }: { conn: Connection } = $props();
  const live = useLive();
  const layers = live.layers;

  let filter = $state("");
  let liveName = $derived($layers?.videoInput?.source ?? null);
  let liveInput = $derived($layers?.videoInput?.input ?? null);
  let list = $derived(visibleSources(sourcesState.list, liveName, filter));
  /** Thumbnail refresh counter (every 3 s while the tab shows and the window is visible). */
  let tick = $state(0);
  let broken = $state<Record<string, number>>({});

  $effect(() => {
    sourcesState.ensure();
    const t = setInterval(() => {
      if (document.visibilityState === "visible") tick++;
    }, 3000);
    return () => clearInterval(t);
  });

  const DOT: Record<LiveSourceStatus | "unbound", Status> = { idle: "off", connecting: "info", live: "ok", lost: "error", unsupported: "warn", unbound: "off" };

  function putInput(v: VideoInputEntry) {
    if (v.id === liveInput) return;
    if (!inputSource(v)) {
      void bindInput(v);
      return;
    }
    void session.run({ type: "triggerVideoInput", input: v.id });
  }

  async function addInput(source: string | null = null) {
    const name = await shell.askText("New Video Input", source ? sourceName(source) : "", "Add", "Name (e.g. Camera 1)");
    if (name === null) return;
    await sourcesState.createInput(name.trim(), source);
  }

  async function bindInput(v: VideoInputEntry) {
    const t = await shell.askText(`Bind “${v.name || "Input"}” to an Address`, inputSource(v) ?? "", "Bind", "Address (ip:port), HOST (name), or omt:ip:port for OMT");
    const src = t ? manualSource(t) : null;
    if (src) await sourcesState.updateInput(v, { source: src });
  }

  function openInputMenu(x: number, y: number, v: VideoInputEntry) {
    const bound = inputSource(v);
    const items: MenuItem[] = [
      { type: "item", id: "live", label: v.id === liveInput ? "Take Off Screen" : "Show on Screen", disabled: !bound, title: bound ? undefined : "Bind it to a source first", run: () => void session.run(v.id === liveInput ? { type: "clearLayer", layer: "videoInput" } : { type: "triggerVideoInput", input: v.id }) },
      {
        type: "item",
        id: "rename",
        label: "Rename…",
        run: async () => {
          const n = await shell.askText("Rename Video Input", v.name ?? "", "Rename");
          if (n && n.trim() && n.trim() !== v.name) await sourcesState.updateInput(v, { name: n.trim() });
        },
      },
      { type: "separator" },
      { type: "header", label: "Bind to a source on this computer" },
      ...sourcesState.list.map((s): MenuItem => ({ type: "item", id: `bind:${s.name}`, label: `${sourceName(s.name)}${transportTag(s) ? ` · ${transportTag(s)}` : ""}${s.name === bound ? " (bound now)" : ""}`, title: s.name, disabled: s.name === bound, run: () => void sourcesState.updateInput(v, { source: s.name }) })),
      { type: "item", id: "bind-address", label: "Address…", run: () => void bindInput(v) },
      { type: "item", id: "unbind", label: "Unbind", disabled: !v.source, run: () => void sourcesState.updateInput(v, { source: null }) },
      { type: "separator" },
      {
        type: "item",
        id: "delete",
        label: "Delete…",
        run: async () => {
          if (await shell.confirm("Delete Video Input", `Delete “${v.name || "Input"}”? Playlist items and slides that use it stop showing it.`)) await sourcesState.deleteInput(v);
        },
      },
    ];
    shell.contextMenu = { x, y, items };
  }

  function inputMenu(e: MouseEvent, v: VideoInputEntry) {
    e.preventDefault();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    if (e.type === "contextmenu") openInputMenu(e.clientX, e.clientY, v);
    else openInputMenu(r.left, r.bottom + 2, v);
  }

  function sourceMenu(e: MouseEvent, s: LiveSource) {
    e.preventDefault();
    shell.contextMenu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { type: "item", id: "show", label: "Show on Screen", run: () => put(s) },
        { type: "item", id: "make-input", label: "Make a Video Input from This Source…", run: () => void addInput(s.name) },
      ],
    };
  }

  function put(s: LiveSource) {
    if (s.name === liveName) return;
    void session.run({ type: "triggerLiveSource", source: s.name });
  }

  async function addByAddress() {
    const t = await shell.askText("Show a Source by Address", "", "Show", "Address (ip:port), HOST (name), or omt:ip:port for OMT");
    const src = t ? manualSource(t) : null;
    if (src) void session.run({ type: "triggerLiveSource", source: src });
  }
</script>

<div class="body" data-testid="inputs-panel">
  <div class="scroll">
  <div class="label">
    <span>Inputs</span>
    <button class="ibtn small" onclick={() => addInput()} title="Add a video input" aria-label="Add a video input" data-testid="add-input"><Plus size={14} /></button>
  </div>
  <div class="grid" data-testid="input-cards">
    {#each sourcesState.inputs as v (v.id)}
      {@const on = v.id === liveInput}
      {@const bound = inputSource(v)}
      <div class="card" class:on class:unbound={!bound} role="group" aria-label={v.name || "Input"} data-testid="input-card" oncontextmenu={(e) => inputMenu(e, v)}>
        <button class="thumb" onclick={() => putInput(v)} title={on ? "On the Video Input layer" : bound ? "Click to show on the Video Input layer" : "Not bound on this computer: click to bind it"} aria-label={`Show ${v.name || "Input"}`}>
          {#if bound && broken[`input:${v.id}`] !== tick}
            <img
              src={conn.rest.sourceThumbnailUrl(bound, 240, tick)}
              alt=""
              draggable="false"
              onerror={() => (broken = { ...broken, [`input:${v.id}`]: tick })}
            />
          {/if}
          <span class="ph" aria-hidden="true"><Cctv size={26} strokeWidth={1.4} /></span>
          {#if on}<span class="badge">LIVE</span>{/if}
        </button>
        <div class="foot">
          <div class="text">
            <span class="name" title={v.name}>{v.name || "Unnamed input"}</span>
            <span class="meta" title={bound ?? undefined}><StatusDot status={DOT[inputStatus(v, sourcesState.list)] ?? "off"} />{inputLine(v, sourcesState.list)}</span>
          </div>
          <button class="ibtn small" onclick={(e) => inputMenu(e, v)} aria-label={`${v.name || "Input"} options`} title="Rename, bind, delete" data-testid="input-menu"><MoreHorizontal size={15} /></button>
        </div>
      </div>
    {:else}
      <p class="hint">{sourcesState.inputsLoaded ? "No video inputs yet. Add one (“+”), or right-click a source below, so playlists and slides can name “Camera 1” and each computer binds it to its own camera." : "Loading inputs…"}</p>
    {/each}
  </div>
  <div class="label"><span>Sources on the network</span></div>
  <div class="grid">
    {#each list as s (s.name)}
      {@const on = s.name === liveName}
      {@const host = sourceHost(s.name)}
      {@const tag = transportTag(s)}
      <div class="card" class:on role="group" aria-label={sourceName(s.name)} data-testid="source-card" oncontextmenu={(e) => sourceMenu(e, s)}>
        <button class="thumb" onclick={() => put(s)} title={on ? "On the Video Input layer" : "Click to show on the Video Input layer"} aria-label={`Show ${sourceName(s.name)}`}>
          {#if broken[s.name] !== tick}
            <img
              src={conn.rest.sourceThumbnailUrl(s.name, 240, tick)}
              alt=""
              draggable="false"
              onerror={() => (broken = { ...broken, [s.name]: tick })}
            />
          {/if}
          <span class="ph" aria-hidden="true"><Cctv size={26} strokeWidth={1.4} /></span>
          {#if on}<span class="badge">LIVE</span>{/if}
          {#if tag}<span class="tport" title="Open Media Transport" data-testid="source-transport">{tag}</span>{/if}
        </button>
        <div class="foot">
          <div class="text">
            <span class="name" title={s.name}>{sourceName(s.name)}</span>
            <span class="meta"><StatusDot status={DOT[s.status] ?? "off"} />{statusLine(s)}{host ? ` · ${host}` : ""}</span>
          </div>
          <button
            class="sw"
            class:on
            role="switch"
            aria-checked={on}
            onclick={() => session.run(on ? { type: "clearLayer", layer: "videoInput" } : { type: "triggerLiveSource", source: s.name })}
            aria-label={`${sourceName(s.name)} on screen`}
            data-testid="source-switch"><span></span></button
          >
        </div>
      </div>
    {:else}
      <div class="tile-empty">
        <EmptyState icon={Cctv} title={sourcesState.loaded ? (filter ? "No sources match the filter" : "No video sources found") : "Looking for video sources…"}>
          {#if sourcesState.error}
            {sourcesState.error}
          {:else}
            Cameras and other computers sending IDN Open Protocol (compatible with NDI®) or OMT video on this network appear here.
          {/if}
        </EmptyState>
      </div>
    {/each}
  </div>
  </div>
  <div class="bottom">
    <button class="ibtn" onclick={addByAddress} title="Show a source by address" aria-label="Show a source by address" data-testid="add-source"><Plus size={16} /></button>
    <span class="meta">{sourcesState.list.length} {sourcesState.list.length === 1 ? "source" : "sources"}{liveName ? ` · ${sourceName(liveName)} on screen` : ""}</span>
    <span class="grow"></span>
    <label class="filter">
      <ListFilter size={14} />
      <input type="search" placeholder="Filter" bind:value={filter} spellcheck="false" aria-label="Filter sources" />
    </label>
  </div>
</div>

<style>
  .body {
    flex: 1;
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .label {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px 0;
    color: var(--text-meta);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }
  .hint {
    grid-column: 1 / -1;
    margin: 0;
    color: var(--text-meta);
    font-size: 12px;
  }
  .ibtn.small {
    width: 22px;
    height: 22px;
  }
  .card.unbound .thumb {
    opacity: 0.6;
  }
  .grid {
    padding: 8px 12px 10px;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(170px, 1fr));
    gap: 10px;
    align-content: start;
  }
  .grid > .tile-empty {
    grid-column: 1 / -1;
  }
  .card {
    display: flex;
    flex-direction: column;
    border: 2px solid var(--line);
    border-radius: var(--r);
    overflow: hidden;
    background: var(--surface-raised);
  }
  .card.on {
    border-color: var(--live);
  }
  .thumb {
    position: relative;
    display: block;
    width: 100%;
    aspect-ratio: 16 / 9;
    padding: 0;
    border: 0;
    background: #0e0f12;
    overflow: hidden;
  }
  .thumb img {
    position: absolute;
    inset: 0;
    z-index: 1;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .ph {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--text-meta);
  }
  .badge {
    position: absolute;
    z-index: 2;
    top: 6px;
    left: 6px;
    padding: 1px 6px;
    border-radius: 3px;
    background: var(--live);
    color: var(--live-ink);
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.05em;
  }
  .tport {
    position: absolute;
    z-index: 2;
    top: 6px;
    right: 6px;
    padding: 1px 5px;
    border-radius: 3px;
    background: var(--surface-overlay);
    border: 1px solid var(--line-strong);
    color: var(--text-label);
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.05em;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 6px;
  }
  .text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .name,
  .meta {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .name {
    font-size: 12.5px;
  }
  .text .meta {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--text-meta);
    font-size: 11px;
  }
  .sw {
    flex: none;
    width: 28px;
    height: 16px;
    padding: 0;
    border: 0;
    border-radius: 8px;
    background: var(--surface-overlay);
    position: relative;
  }
  .sw span {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #ddd;
    transition: left 0.12s;
  }
  .sw.on {
    background: var(--live);
  }
  .sw.on span {
    left: 14px;
    background: #fff;
  }
  .bottom {
    flex: none;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 8px;
    border-top: 1px solid var(--line);
  }
  .bottom .meta {
    margin-left: 6px;
    color: var(--text-meta);
    font-size: 11.5px;
  }
  .grow {
    flex: 1;
  }
  .filter {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--text-meta);
  }
  .filter input {
    width: 120px;
    padding: 3px 6px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-sunk);
    color: var(--text);
    font-size: 12px;
  }
</style>
