<script lang="ts">
  // One section of the Show page's centre: a playlist header band, or one playlist item (or a
  // library item on its own) with a sticky header and its slides in the Grid, Text or List view.
  // The presentation is fetched, and its thumbnails prefetched, when the section first comes near
  // the viewport; until then a placeholder of the right height keeps the scroll bar honest.
  // Slides are shown in the play order `next` follows: with an arrangement, its sequence
  // (repeated groups again, each occurrence its own tile; LIVE / NEXT mark the occurrence the
  // server reports), then the groups it leaves out, collapsed and dimmed.
  import Play from "lucide-svelte/icons/play";
  import Info from "lucide-svelte/icons/info";
  import Ellipsis from "lucide-svelte/icons/ellipsis";
  import Loader from "lucide-svelte/icons/loader-circle";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import type { Connection } from "../../server";
  import type { SlidePosition } from "$protocol";
  import Chip from "../../ui/Chip.svelte";
  import { session } from "../../state/session.svelte";
  import { useLive } from "../../live";
  import { showView, type SlideView } from "../../state/show.svelte";
  import { mediaState } from "../../state/media.svelte";
  import { drag } from "../../dnd.svelte";
  import { aspectRatio, type GridSlide } from "../../model";
  import type { LazyObserver } from "../../lazy";
  import SlideThumb from "./SlideThumb.svelte";
  import { presCache } from "./presCache.svelte";
  import { KIND_DOT, arrangeSlides, inSection, occurrenceMarks, placeholderHeight, playOrder, presentationKind, sectionMeta, slideContextKey, thumbGap, type MediaCue, type Occurrence, type ShowSection } from "./sections";
  import { clickSlide, occurrenceKey, removeMediaCue, setArrangement, slideKey, slideMenu, toggleDisabled } from "./slideActions";

  let {
    conn,
    sec,
    view,
    thumbPx,
    thumbW,
    width,
    sections,
    tiles,
    live,
    announcement = null,
    cursor,
  }: {
    conn: Connection;
    sec: ShowSection;
    view: SlideView;
    thumbPx: number;
    /** Thumbnail width requested from the server (a cache bucket). */
    thumbW: number;
    /** Content width of the scroll area (placeholder heights). */
    width: number;
    /** Observes sections coming near the viewport (loads their documents). */
    sections: LazyObserver;
    /** Observes tiles coming near the viewport (loads their thumbnails). */
    tiles: LazyObserver;
    live: SlidePosition | null;
    /** What is on the announcements layer: LIVE too (it shows above the slide layer). */
    announcement?: SlidePosition | null;
    cursor: SlidePosition | null;
  } = $props();

  let near = $state(false);
  let pres = $derived(sec.type === "presentation" ? sec : null);
  let doc = $derived(pres ? presCache.docs.get(pres.presentationId) : undefined);
  let failed = $derived(pres ? presCache.failed.has(pres.presentationId) : false);
  let aspect = $derived(aspectRatio(doc?.presentation ?? null));
  let isLive = $derived(inSection(sec, live) || inSection(sec, announcement));
  const liveBuild = useLive().build;
  /** "step/steps" of the live slide's object builds (plan G18), when `id` is that slide. */
  function buildLabel(id: string): string | null {
    const b = $liveBuild;
    return pres && b && b.slide === id && b.presentation === pres.presentationId && b.steps > 0 ? `${b.step}/${b.steps}` : null;
  }
  let active = $derived(pres ? (pres.itemId ? showView.selectedItemId === pres.itemId : true) : false);
  let kind = $derived(pres ? pres.kind : presentationKind(doc?.presentation));
  let meta = $derived(doc ? sectionMeta(doc.presentation) : (pres?.meta ?? ""));
  let arrangements = $derived(doc?.presentation.arrangements ?? []);
  let gap = $derived(thumbGap(thumbPx));
  let order = $derived(doc && pres ? playOrder(doc.presentation, pres.arrangement) : null);
  let arranged = $derived(doc ? arrangeSlides(doc.slides, order) : { played: [], left: [], ids: [] });
  let count = $derived(doc ? arranged.played.length : (pres?.slideCount ?? 0));
  // The left-out groups: collapsed, unless the operator opens them or one of them is live.
  let openLeft = $state(false);
  let leftLive = $derived(arranged.left.some((o) => marks(o).live));
  let showLeft = $derived(openLeft || leftLive);

  function observe(node: HTMLElement) {
    return sections.observe(node, () => (near = true));
  }

  // Load the document, then warm the thumbnails at the size the grid asks for.
  $effect(() => {
    if (!near || !pres) return;
    void presCache.load(pres.presentationId);
  });
  $effect(() => {
    if (!near || !pres || view !== "grid") return;
    presCache.prefetch(pres.presentationId, thumbW);
  });

  function marks(o: Occurrence<GridSlide>) {
    return occurrenceMarks(sec, o.slide.id, o.slide.enabled, o.step, arranged.ids, live, cursor, announcement);
  }

  function cueOf(s: GridSlide): MediaCue | null {
    return doc?.cues.get(s.id)?.[0] ?? null;
  }

  function cueIsImage(c: MediaCue | null): boolean {
    if (!c) return false;
    return mediaState.mediaBin?.items.find((i) => i.asset === c.asset)?.kind === "image";
  }

  function thumbSrc(s: GridSlide): string {
    if (!pres) return "";
    const v = doc?.versions.get(s.id);
    return conn.rest.thumbnailUrl(pres.presentationId, s.id, thumbW) + (v ? `&v=${v}` : "");
  }

  function menu(o: Occurrence<GridSlide>, x: number, y: number) {
    if (pres) slideMenu(pres, o.slide, doc?.cues.get(o.slide.id) ?? [], x, y, o.step);
  }

  function rowContext(e: MouseEvent, o: Occurrence<GridSlide>) {
    e.preventDefault();
    if (!pres) return;
    if (e.button === 2) void toggleDisabled(pres, o.slide, o.step);
    else {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      menu(o, r.left + 12, r.top + 20);
    }
  }

  function rowKey(e: KeyboardEvent, o: Occurrence<GridSlide>) {
    if (!slideContextKey(e)) return;
    e.preventDefault();
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu(o, r.left + 12, r.top + 20);
  }

  function moreAt(e: MouseEvent, o: Occurrence<GridSlide>) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    menu(o, r.left, r.bottom + 2);
  }

  function goItem() {
    const pl = session.playlist;
    if (pl && sec.type === "item") void session.run({ type: "triggerPlaylistItem", playlist: pl.id, item: sec.itemId });
  }

  function onArrangement(e: Event) {
    if (!pres?.itemId) return;
    const v = (e.currentTarget as HTMLSelectElement).value;
    void setArrangement(pres.itemId, v || null);
  }

  const ARR_NOTE = "Next and the stage's next slide follow the chosen arrangement: repeated groups play again, groups left out are skipped.";
</script>

{#snippet tile(o: Occurrence<GridSlide>)}
  {#if pres}
    {@const s = o.slide}
    {@const m = marks(o)}
    {@const key = slideKey(sec, s.id)}
    {@const okey = occurrenceKey(sec, s.id, o.step)}
    {@const cue = cueOf(s)}
    <SlideThumb
      slide={s}
      number={o.n}
      dropKey={key}
      occKey={okey}
      src={thumbSrc(s)}
      live={m.live}
      next={m.next}
      selected={showView.selectedSlide === okey}
      dropping={drag.overSlide === key}
      {cue}
      cueIsImage={cueIsImage(cue)}
      lazy={tiles}
      {aspect}
      sectionName={sec.name}
      onclick={() => clickSlide(pres, s, o.step)}
      onRightClick={() => void toggleDisabled(pres, s, o.step)}
      onMenu={(x, y) => menu(o, x, y)}
      onRemoveCue={(c) => void removeMediaCue(pres.presentationId, s.id, c)}
      build={buildLabel(s.id)}
    />
  {/if}
{/snippet}

{#snippet row(o: Occurrence<GridSlide>)}
  {#if pres}
    {@const s = o.slide}
    {@const m = marks(o)}
    {@const key = slideKey(sec, s.id)}
    {@const okey = occurrenceKey(sec, s.id, o.step)}
    {@const cue = cueOf(s)}
    {@const num = o.n ?? "–"}
    <div class="rowwrap" role="listitem" class:dropping={drag.overSlide === key}>
      <button
        class="srow"
        class:live={m.live}
        class:next={m.next}
        class:off={!s.enabled}
        class:selected={showView.selectedSlide === okey}
        style:--gc={s.color ?? "var(--group-neutral)"}
        data-slide={s.id}
        data-key={okey}
        data-drop-slide={key}
        aria-current={m.live ? "true" : undefined}
        aria-label={`${sec.name}, ${o.n !== null ? `slide ${o.n}` : "not in arrangement"}, ${s.label || s.sectionName}${m.live ? ", live" : m.next ? ", next" : ""}${s.enabled ? "" : ", disabled"}`}
        title={s.enabled ? "Click to show · right-click to disable" : "Disabled: skipped when stepping. Right-click to enable"}
        onclick={() => clickSlide(pres, s, o.step)}
        oncontextmenu={(e) => rowContext(e, o)}
        onkeydown={(e) => rowKey(e, o)}
      >
        {#if view === "text"}
          <span class="stripe"></span>
          <span class="who">
            <b>{num} · {s.label || s.sectionName}</b>
            {#if m.live}<Chip kind="live" />{:else if m.next}<Chip kind="next" />{/if}
            {#if !s.enabled}<Chip kind="disabled" />{/if}
          </span>
          <span class="body">{s.text || " "}</span>
          <span class="cuename">{cue?.name ?? ""}</span>
        {:else}
          <span class="num">{num}</span>
          <span class="grp"><span class="sq"></span><span>{s.label || s.sectionName}</span></span>
          <span class="line">{s.text.split("\n").find((l) => l.trim()) ?? ""}</span>
          <span class="cuename">{cue?.name ?? ""}</span>
          <span class="state" class:live={m.live}>{m.live ? "LIVE" : m.next ? "NEXT" : !s.enabled ? "OFF" : ""}</span>
        {/if}
      </button>
      <button class="more" onclick={(e) => moreAt(e, o)} aria-label={`Slide ${num} actions`} title="Slide actions (Shift+F10)"><Ellipsis size={14} /></button>
    </div>
  {/if}
{/snippet}

{#if sec.type === "header"}
  <div id={`sec-${sec.itemId}`} class="hband" data-section={sec.key} role="heading" aria-level="2" style:--hc={sec.color ?? "var(--text-meta)"}>
    <span>{sec.name}</span>
  </div>
{:else}
  <section
    id={`sec-${sec.type === "presentation" && !sec.itemId ? sec.presentationId : sec.itemId}`}
    class="sec"
    aria-label={sec.name}
    data-section={sec.key}
    {@attach observe}
  >
    <div class="shead" class:active class:live={isLive}>
      <span class="dot" style:background={sec.type === "presentation" ? KIND_DOT[kind] : KIND_DOT.media}></span>
      <h2 title={sec.name}>{sec.name}</h2>
      {#if meta}<span class="meta">{meta}</span>{/if}
      {#if isLive}<Chip kind="live" />{/if}
      {#if pres && !pres.enabled}<Chip kind="disabled" />{/if}
      <span class="grow"></span>
      {#if pres?.itemId && arrangements.length}
        <label class="arr">
          Arrangement
          <select value={pres.arrangement ?? ""} onchange={onArrangement} aria-describedby={`arr-note-${pres.itemId}`}>
            <option value="">{doc?.presentation.selectedArrangement ? "Song's own" : "Slide order"}</option>
            {#each arrangements as a (a.id)}
              <option value={a.id}>{a.name || "Untitled"}</option>
            {/each}
          </select>
          <span class="note" id={`arr-note-${pres.itemId}`} title={ARR_NOTE} aria-label={ARR_NOTE}><Info size={13} /></span>
        </label>
      {/if}
      {#if pres}<span class="count">{count} {count === 1 ? "slide" : "slides"}</span>{/if}
    </div>

    {#if sec.type === "missing"}
      <p class="none">This presentation is not in the library any more.</p>
    {:else if sec.type === "item"}
      <div class="media-item">
        <button class="go" onclick={goItem} aria-label={`Show ${sec.name} on screen`} title="Show on screen"><Play size={30} strokeWidth={1.5} /></button>
        <span>{sec.kind === "videoInput" ? "Video input" : sec.kind === "media" ? "Media" : "Item"} · click to show</span>
      </div>
    {:else if pres && !doc}
      <div class="placeholder" style:height={`${placeholderHeight(view, pres.slideCount, width, thumbPx)}px`}>
        {#if failed}<p class="none">Could not load this presentation.</p>{:else if near}<Loader size={16} class="spin" />{/if}
      </div>
    {:else if pres && doc}
      {#if !doc.slides.length}
        <p class="none">This presentation has no slides.</p>
      {:else if view === "grid"}
        <div class="grid" style:grid-template-columns={`repeat(auto-fill, min(${thumbPx}px, 100%))`} style:gap={`${gap}px`}>
          {#each arranged.played as o (o.key)}
            {@render tile(o)}
          {/each}
        </div>
      {:else}
        <div class="rows" class:text={view === "text"} class:list={view === "list"} role="list">
          {#each arranged.played as o (o.key)}
            {@render row(o)}
          {/each}
        </div>
      {/if}
      {#if arranged.left.length}
        <div class="left" class:open={showLeft}>
          <button class="lefthead" aria-expanded={showLeft} disabled={leftLive} onclick={() => (openLeft = !openLeft)} title="Slides of groups the arrangement leaves out: Next skips them, a click still shows one">
            <ChevronRight size={14} class="chev" />
            Not in arrangement
            <span class="leftcount">{arranged.left.length} {arranged.left.length === 1 ? "slide" : "slides"} · {[...new Set(arranged.left.map((o) => o.slide.sectionName || "Untitled"))].join(", ")}</span>
          </button>
          {#if showLeft}
            {#if view === "grid"}
              <div class="grid" style:grid-template-columns={`repeat(auto-fill, min(${thumbPx}px, 100%))`} style:gap={`${gap}px`}>
                {#each arranged.left as o (o.key)}
                  {@render tile(o)}
                {/each}
              </div>
            {:else}
              <div class="rows" class:text={view === "text"} class:list={view === "list"} role="list">
                {#each arranged.left as o (o.key)}
                  {@render row(o)}
                {/each}
              </div>
            {/if}
          {/if}
        </div>
      {/if}
    {/if}
  </section>
{/if}

<style>
  .hband {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 16px -16px 0;
    padding: 10px 16px;
    background: #1f2127;
    border-left: 6px solid var(--hc);
  }
  .hband span {
    font-size: 13px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sec {
    padding-top: 12px;
  }
  .shead {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px 10px;
    margin: 0 -16px 12px;
    padding: 8px 16px;
    background: var(--surface-panel);
    border-top: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  .shead.active {
    background: var(--accent-sel-2);
  }
  .dot {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 2px;
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
    color: var(--text);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta,
  .count {
    color: var(--text-meta);
    font-size: 12px;
    white-space: nowrap;
  }
  .grow {
    flex: 1 1 10px;
  }
  .arr {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-label);
    font-size: 12px;
  }
  .arr select {
    height: 26px;
    padding: 0 6px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    font: inherit;
    font-size: 12px;
  }
  .note {
    display: grid;
    color: var(--text-meta);
    cursor: help;
  }
  .grid {
    display: grid;
    justify-content: start;
    align-items: start;
  }
  .left {
    margin-top: 14px;
  }
  .lefthead {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    margin-bottom: 8px;
    padding: 5px 8px;
    border: 1px dashed var(--line);
    border-radius: var(--r);
    background: transparent;
    color: var(--text-label);
    font-size: 12px;
    font-weight: 600;
    text-align: left;
  }
  .lefthead:hover:not(:disabled) {
    border-color: var(--line-hover);
    color: var(--text);
  }
  .lefthead:disabled {
    cursor: default;
  }
  .lefthead :global(.chev) {
    flex: none;
    transition: transform 0.12s;
  }
  .left.open .lefthead :global(.chev) {
    transform: rotate(90deg);
  }
  .leftcount {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-meta);
    font-weight: 400;
  }
  .left .grid,
  .left .rows {
    opacity: 0.55;
  }
  .left .grid:hover,
  .left .rows:hover,
  .left .grid:focus-within,
  .left .rows:focus-within {
    opacity: 0.85;
  }
  .placeholder {
    display: grid;
    place-items: center;
    border-radius: var(--r);
    background: repeating-linear-gradient(90deg, transparent 0 8px, rgba(255, 255, 255, 0.015) 8px 16px);
    color: var(--text-meta);
  }
  .none {
    margin: 0;
    padding: 8px 0;
    color: var(--text-meta);
  }
  .media-item {
    display: flex;
    align-items: center;
    gap: 12px;
    color: var(--text-label);
    font-size: 12px;
  }
  .go {
    width: 160px;
    aspect-ratio: 16 / 9;
    display: grid;
    place-items: center;
    border: 2px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-well);
    color: var(--text-2);
  }
  .go:hover {
    border-color: var(--live);
    color: var(--live);
  }
  .rows {
    display: flex;
    flex-direction: column;
  }
  .rows.text {
    gap: 6px;
  }
  .rows.list {
    border: 1px solid var(--line);
    border-radius: var(--r);
    overflow: hidden;
  }
  .rowwrap {
    position: relative;
    outline: 2px dashed transparent;
    outline-offset: -2px;
  }
  .rowwrap.dropping {
    outline-color: var(--ok);
  }
  .srow {
    width: 100%;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }
  .rows.text .srow {
    display: flex;
    align-items: stretch;
    padding: 0 30px 0 0;
    border: 2px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-panel);
    overflow: hidden;
  }
  .rows.list .srow {
    display: grid;
    grid-template-columns: 34px 110px minmax(0, 1fr) auto 52px;
    gap: 10px;
    align-items: center;
    padding: 6px 34px 6px 10px;
    border: 0;
    border-bottom: 1px solid var(--surface-raised);
    background: #17181c;
    font-size: 12.5px;
    box-shadow: inset 3px 0 0 transparent;
  }
  .rows.text .srow.next {
    border-color: var(--next-ring);
  }
  .rows.text .srow.selected {
    border-color: var(--accent-ring);
  }
  .rows.text .srow.live {
    border-color: var(--live);
  }
  .rows.list .srow.next {
    box-shadow: inset 3px 0 0 var(--next-ring);
  }
  .rows.list .srow.selected {
    background: var(--accent-sel-2);
    box-shadow: inset 3px 0 0 var(--accent-ring);
  }
  .rows.list .srow.live {
    background: #2b2416;
    box-shadow: inset 3px 0 0 var(--live);
  }
  .srow.off > :not(.who):not(.state) {
    opacity: 0.38;
  }
  .srow:focus-visible {
    outline: 2px solid var(--accent-ring);
    outline-offset: -2px;
  }
  .stripe {
    flex: none;
    width: 6px;
    background: var(--gc);
  }
  .who {
    flex: none;
    width: 110px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 3px;
    padding: 8px 10px;
    font-size: 11.5px;
    color: var(--text-label);
  }
  .who b {
    color: var(--text);
    font-weight: 700;
  }
  .body {
    flex: 1;
    padding: 8px 12px;
    font-size: 15px;
    line-height: 1.45;
    white-space: pre-line;
  }
  .cuename {
    flex: none;
    padding: 8px 12px;
    font-size: 11.5px;
    color: var(--ok-text);
    white-space: nowrap;
  }
  .rows.list .cuename {
    padding: 0;
  }
  .num {
    color: var(--text-label);
    font-variant-numeric: tabular-nums;
  }
  .grp {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .grp span:last-child,
  .line {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sq {
    flex: none;
    width: 10px;
    height: 10px;
    border-radius: 2px;
    background: var(--gc);
  }
  .state {
    font-size: 10px;
    font-weight: 700;
    color: var(--text-meta);
  }
  .srow.next .state {
    color: var(--text-2);
  }
  .state.live {
    color: var(--live);
  }
  .rowwrap .more {
    position: absolute;
    right: 6px;
    top: 6px;
    width: 22px;
    height: 20px;
    display: grid;
    place-items: center;
    padding: 0;
    border: 0;
    border-radius: var(--r-xs);
    background: transparent;
    color: var(--text-label);
    opacity: 0;
  }
  .rowwrap:hover .more,
  .rowwrap:focus-within .more {
    opacity: 1;
  }
  .rowwrap .more:hover {
    background: var(--surface-overlay);
    color: var(--text);
  }
</style>
