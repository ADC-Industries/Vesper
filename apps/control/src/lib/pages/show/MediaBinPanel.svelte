<script lang="ts">
  import Plus from "lucide-svelte/icons/plus";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import LayoutGrid from "lucide-svelte/icons/layout-grid";
  import List from "lucide-svelte/icons/list";
  import ListFilter from "lucide-svelte/icons/list-filter";
  import Alert from "lucide-svelte/icons/triangle-alert";
  import Film from "lucide-svelte/icons/film";
  import Music from "lucide-svelte/icons/music";
  import ImageIcon from "lucide-svelte/icons/image";
  import type { MediaBinItemInfo } from "$protocol";
  import type { Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import { mediaState } from "../../state/media.svelte";
  import { showView } from "../../state/show.svelte";
  import { useLive } from "../../live";
  import { thumbWidthFor } from "../../model";
  import { formatTime } from "./transport";
  import { createLazyObserver, type LazyObserver } from "../../lazy";
  import { drag } from "../../dnd.svelte";
  import { mediaDrops } from "./slideActions";

  let { conn }: { conn: Connection } = $props();

  const live = useLive();
  const layers = live.layers;

  let lazy = $state.raw<LazyObserver | null>(null);
  let width = $state(0);
  let dpr = $state(typeof devicePixelRatio === "number" ? devicePixelRatio : 1);
  const GAP = 12;
  const PAD = 12;

  let bin = $derived(mediaState.mediaBin);
  let items = $derived.by(() => {
    const q = showView.mediaFilter.trim().toLowerCase();
    const all = bin?.items ?? [];
    return q ? all.filter((i) => i.name.toLowerCase().includes(q)) : all;
  });
  let cell = $derived(width > 0 ? (width - PAD * 2 - GAP * (showView.mediaColumns - 1)) / showView.mediaColumns : 200);
  let thumbW = $derived(thumbWidthFor(cell, dpr));
  let liveAssets = $derived([$layers?.media?.asset, $layers?.foreground?.asset].filter((a): a is string => !!a));

  function scroller(node: HTMLElement) {
    const l = createLazyObserver(node);
    lazy = l;
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) width = e.contentRect.width;
    });
    ro.observe(node);
    return () => {
      ro.disconnect();
      l.disconnect();
      lazy = null;
    };
  }

  /** Loads the thumbnail once the cell comes near the viewport. */
  function lazyImg(node: HTMLImageElement, src: string) {
    const l = lazy;
    if (!l) {
      node.src = src;
      return;
    }
    return l.observe(node, () => (node.src = src));
  }

  /** A bin item dragged onto a slide becomes a media cue. */
  function cueDrag(e: PointerEvent, it: MediaBinItemInfo) {
    if (it.kind === "audio" || it.availability === "missing") return;
    drag.press(e, { kind: "media", id: it.asset, name: it.name }, mediaDrops);
  }

  function trigger(it: MediaBinItemInfo) {
    // A drag released (or cancelled with Esc) over the tile it started on clicks it: not a pick.
    if (drag.consumeClick()) return;
    if (!it.enabled) return;
    if (it.availability === "missing") {
      conn.pushNotice("warning", `“${it.name}” is missing on the server.`, "mediaMissing");
      return;
    }
    if (it.kind === "audio") {
      showView.transportTarget = "audio";
      void session.run({ type: "triggerMedia", asset: it.asset, layer: "audio" });
      return;
    }
    if (it.kind === "video") showView.transportTarget = "video";
    void session.run({ type: "triggerMedia", asset: it.asset });
  }

  function isLive(it: MediaBinItemInfo): boolean {
    return liveAssets.includes(it.asset);
  }

  function kindIcon(it: MediaBinItemInfo) {
    return it.kind === "video" ? Film : it.kind === "audio" ? Music : ImageIcon;
  }
</script>

<section class="bin" aria-label={bin ? `Media: ${bin.name}` : "Media"}>
  <div class="scroll" {@attach scroller}>
    {#if !bin || items.length === 0}
      <div class="tile-empty">
        <button class="tile" disabled title="Uploading media comes later" aria-label="Import Media"><ArrowDown size={44} strokeWidth={1.3} /></button>
        <span>{bin && showView.mediaFilter ? "No matches" : "Import Media"}</span>
      </div>
    {:else if lazy}
      {#if showView.mediaView === "grid"}
        <div class="grid" style:grid-template-columns={`repeat(${showView.mediaColumns}, minmax(0, 1fr))`} style:gap={`${GAP}px`} style:padding={`${PAD}px`}>
          {#each items as it (it.id)}
            {@const Icon = kindIcon(it)}
            <button
              class="cell"
              class:missing={it.availability === "missing"}
              class:live={isLive(it)}
              class:disabled={!it.enabled}
              onclick={() => trigger(it)}
              onpointerdown={(e) => cueDrag(e, it)}
              title={`${it.name}${it.availability === "missing" ? " (missing on the server)" : ""} · drag onto a slide to add a media cue`}
              aria-current={isLive(it) ? "true" : undefined}
            >
              <span class="thumb">
                {#if it.availability === "online"}
                  {#key thumbW}
                    <img alt="" draggable="false" decoding="async" {@attach (n: HTMLImageElement) => lazyImg(n, conn.rest.mediaThumbnailUrl(it.asset, thumbW))} />
                  {/key}
                {:else}
                  <span class="ph"><Icon size={26} strokeWidth={1.4} /></span>
                {/if}
                {#if it.availability === "missing"}<span class="warn" title="Missing"><Alert size={15} /></span>{/if}
                {#if it.kind !== "image"}<span class="kind"><Icon size={12} /></span>{/if}
                {#if it.duration != null}<span class="dur" data-testid="duration">{formatTime(it.duration)}</span>{/if}
              </span>
              <span class="name">{it.name}</span>
            </button>
          {/each}
        </div>
      {:else}
        <div class="list">
          {#each items as it (it.id)}
            {@const Icon = kindIcon(it)}
            <button
              class="lrow"
              class:missing={it.availability === "missing"}
              class:live={isLive(it)}
              onclick={() => trigger(it)}
              onpointerdown={(e) => cueDrag(e, it)}
            >
              <Icon size={14} />
              <span class="name">{it.name}</span>
              <span class="k">{it.kind}</span>
              {#if it.duration != null}<span class="k">{formatTime(it.duration)}</span>{/if}
              {#if it.availability === "missing"}<span class="warn"><Alert size={14} /></span>{/if}
            </button>
          {/each}
        </div>
      {/if}
    {/if}
  </div>

  <div class="bottom">
    <button class="ibtn" disabled title="Uploading media comes later" aria-label="Add media"><Plus size={16} /></button>
    <span class="bsep"></span>
    <span class="meta">{bin ? `${bin.name} · ${bin.items.length} item${bin.items.length === 1 ? "" : "s"}` : ""}</span>
    <span class="grow"></span>
    <label class="filter mf">
      <ListFilter size={14} />
      <input type="search" placeholder="Filter" bind:value={showView.mediaFilter} spellcheck="false" aria-label="Filter media" />
    </label>
    <span class="grow"></span>
    <button class="ibtn" class:on={showView.mediaView === "grid"} onclick={() => showView.setMediaView("grid")} aria-label="Grid" title="Grid"><LayoutGrid size={16} /></button>
    <button class="ibtn" class:on={showView.mediaView === "list"} onclick={() => showView.setMediaView("list")} aria-label="List" title="List"><List size={16} /></button>
    <label class="sr-only" for="media-size">Media size</label>
    <input
      id="media-size"
      class="slim"
      type="range"
      min="2"
      max="12"
      step="1"
      disabled={showView.mediaView !== "grid"}
      value={14 - showView.mediaColumns}
      oninput={(e) => showView.setMediaColumns(14 - Number((e.currentTarget as HTMLInputElement).value))}
    />
  </div>
</section>

<style>
  .bin {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .grid {
    display: grid;
    align-content: start;
  }
  .cell {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: center;
  }
  .thumb {
    position: relative;
    display: block;
    aspect-ratio: 16 / 9;
    background: var(--surface-app);
    border: 1px solid var(--line);
    overflow: hidden;
  }
  .thumb img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .ph {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-disabled);
  }
  .cell:hover .thumb {
    border-color: var(--line-strong);
  }
  .cell.live .thumb {
    outline: 3px solid var(--live);
    outline-offset: 1px;
  }
  .cell.missing .thumb,
  .cell.missing .name,
  .lrow.missing {
    opacity: 0.55;
  }
  .cell.disabled {
    opacity: 0.35;
  }
  .warn {
    position: absolute;
    top: 4px;
    right: 4px;
    color: var(--warn);
    display: grid;
  }
  .dur {
    position: absolute;
    right: 4px;
    bottom: 4px;
    padding: 1px 4px;
    border-radius: 2px;
    background: rgba(0, 0, 0, 0.7);
    color: var(--text);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    line-height: 1.3;
  }
  .kind {
    position: absolute;
    left: 4px;
    bottom: 4px;
    padding: 2px;
    border-radius: 2px;
    background: rgba(0, 0, 0, 0.6);
    display: grid;
  }
  .name {
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list {
    display: flex;
    flex-direction: column;
    padding: 4px 6px;
  }
  .lrow {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 25px;
    padding: 0 8px;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
  }
  .lrow:hover {
    background: rgba(255, 255, 255, 0.06);
  }
  .lrow.live {
    box-shadow: inset 3px 0 0 var(--live);
  }
  .lrow .name {
    flex: 1;
    text-align: left;
  }
  .lrow .k {
    color: var(--text-meta);
    font-size: 12px;
  }
  .lrow .warn {
    position: static;
  }
  .bottom {
    flex: none;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 10px 0 6px;
    border-top: 1px solid var(--line);
  }
  .bsep {
    width: 1px;
    height: 14px;
    background: var(--surface-overlay);
    margin: 0 4px;
  }
  .meta {
    color: var(--text-meta);
    font-size: 12px;
  }
  .grow {
    flex: 1;
  }
  .mf {
    width: 230px;
  }
  .slim {
    margin-left: 10px;
  }
</style>
