<script lang="ts">
  // The Show page's centre: the whole playlist as one scrolling list of sections (sticky
  // headers, header bands), or a library item picked outside the playlist as a single section
  // with "Add to <playlist>". The toolbar holds Grid / Text / List and the size slider.
  import LayoutGrid from "lucide-svelte/icons/layout-grid";
  import Type from "lucide-svelte/icons/type";
  import List from "lucide-svelte/icons/list";
  import ImageSmall from "lucide-svelte/icons/image-minus";
  import ImageLarge from "lucide-svelte/icons/image-plus";
  import type { Connection } from "../../server";
  import type { PlaylistItemDetail } from "$protocol";
  import Segmented from "../../ui/Segmented.svelte";
  import Button from "../../ui/Button.svelte";
  import { session } from "../../state/session.svelte";
  import { libraryState } from "../../state/library.svelte";
  import { showView, type SlideView } from "../../state/show.svelte";
  import { useLive } from "../../live";
  import { cssColor, thumbWidthFor } from "../../model";
  import { createLazyObserver, type LazyObserver } from "../../lazy";
  import SectionView from "./SectionView.svelte";
  import { presCache } from "./presCache.svelte";
  import { librarySection, liveSectionKey, playlistCounts, playlistSections, THUMB_MAX, THUMB_MIN, THUMB_STEP, type ShowSection } from "./sections";
  import { slideKey } from "./slideActions";
  import type { OpenDocument } from "$protocol";
  import PresenceBadge from "../../components/PresenceBadge.svelte";
  import { presence } from "../../state/presence.svelte";

  let { conn }: { conn: Connection } = $props();

  const live = useLive();
  const liveSlide = live.slide;
  const liveAnnouncement = live.announcement;
  /** The slide or announcement triggered last: the grid scrolls to it. */
  const latest = live.latest;
  const cursor = live.cursor;

  // The cache follows the connection (the session keeps it current, also while this page is
  // not shown).
  $effect(() => presCache.use(conn));
  // The open presentation (fetched by the session) feeds the cache too.
  $effect(() => {
    const p = session.open?.presentation;
    if (p && !session.open?.prop) presCache.put(p);
  });

  let width = $state(0);
  let dpr = $state(typeof devicePixelRatio === "number" ? devicePixelRatio : 1);
  let sectionsLazy = $state.raw<LazyObserver | null>(null);
  let tilesLazy = $state.raw<LazyObserver | null>(null);
  let body = $state<HTMLElement | null>(null);

  let thumbW = $derived(thumbWidthFor(showView.thumbPx, dpr));

  const headerColor = (it: PlaylistItemDetail) => (it.kind.type === "header" ? cssColor(it.kind.color, 1) : null);

  /** A library item picked in the left column (outside the playlist). */
  let libPick = $derived.by(() => {
    if (showView.listSource !== "library" || !showView.selectedPresentationId) return null;
    const id = showView.selectedPresentationId;
    const sum = libraryState.libraryItems.find((p) => p.id === id);
    if (sum) return sum;
    const open = session.open?.presentation;
    return open && open.id === id ? { id, name: open.name ?? "Untitled", slideCount: session.open?.slides.length ?? 0 } : null;
  });

  let sections = $derived.by((): ShowSection[] => {
    if (libPick) return [librarySection(libPick)];
    if (showView.listSource === "playlist" && session.playlist) return playlistSections(session.playlist.items, headerColor);
    return [];
  });

  let counts = $derived(playlistCounts(sections));
  let pickInPlaylist = $derived(!!libPick && !!session.playlist?.items.some((i) => i.kind.type === "presentation" && i.kind.presentation.id === libPick.id));
  let libName = $derived(libraryState.libraries.find((l) => l.id === libraryState.libraryId)?.name ?? "Library");

  // Presence (G8, browsing): the playlist shown here, the library item picked, and the
  // presentation selected are open on the Show page (others see "viewing on the Show page";
  // browsing never takes an edit lock).
  let showOpen = $derived.by((): OpenDocument[] => {
    const out: OpenDocument[] = [];
    if (showView.listSource === "playlist" && session.playlist) out.push({ kind: "playlistNode", id: session.playlist.id, place: "show" });
    if (libPick) out.push({ kind: "presentation", id: libPick.id, place: "show" });
    const cur = session.open?.presentation.id;
    if (cur && !session.open?.prop && !session.open?.theme && !session.open?.stageLayout) out.push({ kind: "presentation", id: cur, place: "show" });
    return out;
  });
  $effect(() => {
    presence.setOpen("show", showOpen);
  });
  $effect(() => () => presence.setOpen("show", null));
  let badge = $derived<{ kind: "playlistNode" | "presentation"; id: string } | null>(
    libPick ? { kind: "presentation", id: libPick.id } : showView.listSource === "playlist" && session.playlist ? { kind: "playlistNode", id: session.playlist.id } : null,
  );

  let title = $derived(libPick ? `${libName} · ${libPick.name}` : showView.listSource === "playlist" ? (session.playlist?.name ?? "") : libName);
  let meta = $derived(
    libPick
      ? `${libPick.slideCount} ${libPick.slideCount === 1 ? "slide" : "slides"}`
      : showView.listSource === "playlist" && session.playlist
        ? `${counts.items} ${counts.items === 1 ? "item" : "items"} · ${counts.slides} ${counts.slides === 1 ? "slide" : "slides"}`
        : "Nothing selected",
  );
  let empty = $derived(
    sections.length
      ? ""
      : showView.listSource === "library"
        ? "Pick a song from the list on the left to show its slides here. Double-click it, or drag it onto a playlist, to add it."
        : session.playlist
          ? "This playlist is empty. Drag songs onto it from the library, or pick a library and double-click a song."
          : "Pick a playlist on the left.",
  );

  function scroller(node: HTMLElement) {
    const s = createLazyObserver(node, "1200px 0px");
    const t = createLazyObserver(node, "600px 0px");
    sectionsLazy = s;
    tilesLazy = t;
    const ro = new ResizeObserver((entries) => {
      for (const e of entries) width = e.contentRect.width;
    });
    ro.observe(node);
    const mq = typeof matchMedia === "function" ? matchMedia(`(resolution: ${dpr}dppx)`) : null;
    const onDpr = () => (dpr = devicePixelRatio);
    mq?.addEventListener("change", onDpr);
    return () => {
      ro.disconnect();
      s.disconnect();
      t.disconnect();
      mq?.removeEventListener("change", onDpr);
      sectionsLazy = null;
      tilesLazy = null;
    };
  }

  // Scroll the live slide into view when it changes (Space, a click, another operator), also
  // across section boundaries: first its section (which loads it), then the slide.
  let scrolledFor = "";
  $effect(() => {
    const pos = $latest;
    void showView.slideView;
    if (!pos || !body) return;
    const key = liveSectionKey(sections, pos);
    const sec = key ? sections.find((s) => s.key === key) : null;
    if (!sec) return;
    const want = `${showView.slideView}:${slideKey(sec, pos.slide)}#${pos.step ?? "-"}`;
    const loaded = sec.type === "presentation" && presCache.docs.has(sec.presentationId);
    if (want === scrolledFor) return;
    // After the DOM caught up with a section that just loaded.
    queueMicrotask(() => {
      // The live occurrence (a repeated slide has a tile per occurrence), else any tile of it.
      const el =
        body?.querySelector(`[data-section="${CSS.escape(sec.key)}"] [data-key][aria-current="true"]`) ??
        body?.querySelector(`[data-key^="${CSS.escape(slideKey(sec, pos.slide) + "#")}"]`);
      if (el) {
        scrolledFor = want;
        el.scrollIntoView({ block: "nearest", behavior: "smooth" });
      } else if (!loaded) {
        body?.querySelector(`[data-section="${CSS.escape(sec.key)}"]`)?.scrollIntoView({ block: "start" });
      }
    });
  });

  function onWheel(e: WheelEvent) {
    // Ctrl + wheel resizes the tiles, like zooming.
    if (!e.ctrlKey || showView.slideView !== "grid") return;
    e.preventDefault();
    if (e.deltaY < 0) showView.thumbLarger();
    else showView.thumbSmaller();
  }

  function addPick() {
    const pl = session.playlist;
    if (!pl || !libPick) return;
    if (pickInPlaylist) {
      const it = pl.items.find((i) => i.kind.type === "presentation" && i.kind.presentation.id === libPick.id);
      showView.listSource = "playlist";
      if (it) {
        showView.selectedItemId = it.id;
        setTimeout(() => body?.querySelector(`[data-section="item:${CSS.escape(it.id)}"]`)?.scrollIntoView({ block: "start" }));
      }
    } else void session.addToPlaylist(pl.id, { id: libPick.id, name: libPick.name }).then(() => session.selectPlaylist(pl.id, true));
  }

  const VIEWS: { value: SlideView; label: string; icon: typeof LayoutGrid; iconOnly: true }[] = [
    { value: "grid", label: "Grid view", icon: LayoutGrid, iconOnly: true },
    { value: "text", label: "Text view", icon: Type, iconOnly: true },
    { value: "list", label: "List view", icon: List, iconOnly: true },
  ];
</script>

<section class="slides" aria-label="Slides">
  <div class="tools">
    <h1 title={title}>{title}</h1>
    <span class="meta">{meta}</span>
    {#if badge}<PresenceBadge kind={badge.kind} id={badge.id} />{/if}
    <span class="grow"></span>
    {#if libPick}
      <div class="pick">
        {#if session.playlist}
          <Button variant="secondary" size="sm" onclick={() => (showView.listSource = "playlist")}>Back to playlist</Button>
          <Button variant="primary" size="sm" onclick={addPick}>{pickInPlaylist ? `Show in ${session.playlist.name}` : `Add to ${session.playlist.name}`}</Button>
        {:else}
          <Button variant="primary" size="sm" disabled title="Pick a playlist first">Add to playlist</Button>
        {/if}
      </div>
    {/if}
    <Segmented options={VIEWS} value={showView.slideView} label="Slide view" size="sm" onchange={(v) => showView.setSlideView(v)} />
    {#if showView.slideView === "grid"}
      <label class="size" title="Slide size (Ctrl+wheel)">
        <ImageSmall size={14} aria-hidden="true" />
        <input
          type="range"
          min={THUMB_MIN}
          max={THUMB_MAX}
          step={THUMB_STEP}
          value={showView.thumbPx}
          oninput={(e) => showView.setThumbPx(Number((e.currentTarget as HTMLInputElement).value))}
          aria-label="Slide size"
          aria-valuetext={`${showView.thumbPx} pixels wide`}
        />
        <ImageLarge size={14} aria-hidden="true" />
      </label>
    {/if}
  </div>

  <div class="scroll" bind:this={body} {@attach scroller} onwheel={onWheel}>
    {#if sectionsLazy && tilesLazy}
      {#each sections as sec (sec.key)}
        <SectionView
          {conn}
          {sec}
          view={showView.slideView}
          thumbPx={showView.thumbPx}
          {thumbW}
          {width}
          sections={sectionsLazy}
          tiles={tilesLazy}
          live={$liveSlide}
          announcement={$liveAnnouncement}
          cursor={$cursor}
        />
      {/each}
    {/if}
    {#if empty}<p class="empty">{empty}</p>{/if}
  </div>
</section>

<style>
  .slides {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    background: var(--surface-sunk);
  }
  .tools {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 10px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--line);
    background: var(--surface-panel);
  }
  h1 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
    color: var(--text);
    min-width: 0;
    max-width: 40%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--text-meta);
    white-space: nowrap;
  }
  .grow {
    flex: 1 1 10px;
  }
  .pick {
    display: flex;
    gap: 6px;
  }
  .size {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-label);
  }
  .size input {
    width: 160px;
    accent-color: var(--accent-ring);
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 0 16px 16px;
  }
  .empty {
    margin: 40px auto;
    max-width: 360px;
    color: var(--text-label);
    text-align: center;
  }
</style>
