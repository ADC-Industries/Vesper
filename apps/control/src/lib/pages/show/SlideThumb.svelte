<script lang="ts">
  // One slide tile of the Show page's grid (one occurrence of the slide in the play order): the
  // thumbnail (requested once it scrolls near the viewport), the group band with number and label, LIVE / NEXT / DISABLED chips, the media
  // cue badge with its ✕, and the "…" menu button. Disabled slides stay clickable (to select);
  // a right-click toggles disabled; a media bin item can be dropped on it.
  import Film from "lucide-svelte/icons/film";
  import ImageIcon from "lucide-svelte/icons/image";
  import X from "lucide-svelte/icons/x";
  import Ellipsis from "lucide-svelte/icons/ellipsis";
  import Chip from "../../ui/Chip.svelte";
  import type { GridSlide } from "../../model";
  import type { LazyObserver } from "../../lazy";
  import { slideContextKey, type MediaCue } from "./sections";

  let {
    slide,
    number,
    dropKey,
    occKey,
    src,
    live,
    next,
    selected,
    dropping,
    cue,
    cueIsImage,
    lazy,
    aspect,
    sectionName,
    onclick,
    onRightClick,
    onMenu,
    onRemoveCue,
    build = null,
  }: {
    slide: GridSlide;
    /** The number on the tile: its place in the play order (`null`: left out of it). */
    number: number | null;
    /** `data-drop-slide` value (the slide). */
    dropKey: string;
    /** `data-key` value (this occurrence of the slide). */
    occKey: string;
    /** Thumbnail URL (only requested once the tile scrolls near the viewport). */
    src: string;
    live: boolean;
    next: boolean;
    selected: boolean;
    /** A media item is dragged over it. */
    dropping: boolean;
    cue: MediaCue | null;
    cueIsImage: boolean;
    lazy: LazyObserver;
    aspect: number;
    sectionName: string;
    onclick: () => void;
    onRightClick: () => void;
    onMenu: (x: number, y: number) => void;
    onRemoveCue: (c: MediaCue) => void;
    /** The live slide's object build progress ("1/3"; plan G18), or null. */
    build?: string | null;
  } = $props();

  let visible = $state(false);
  let loaded = $state(false);
  let failed = $state(false);
  let shown = $state("");
  let el = $state<HTMLElement | null>(null);

  // Keep showing the previous picture while a new size loads, so resizing doesn't flash.
  $effect(() => {
    if (visible && src !== shown) {
      failed = false;
      shown = src;
    }
  });

  function observe(node: HTMLElement) {
    return lazy.observe(node, () => (visible = true));
  }

  let label = $derived(slide.label || slide.sectionName);
  let aria = $derived(
    [
      number !== null ? `${sectionName}, slide ${number}` : `${sectionName}, not in arrangement`,
      label,
      cue ? `media cue ${cue.name}` : "",
      live ? "live" : "",
      next ? "next" : "",
      slide.enabled ? "" : "disabled",
    ]
      .filter(Boolean)
      .join(", "),
  );
  let tip = $derived(slide.enabled ? "Click to show · right-click to disable" : "Disabled: skipped when stepping. Click to select · right-click to enable");

  function menuAtTile() {
    const r = (el ?? document.body).getBoundingClientRect();
    onMenu(r.left + 12, r.top + 24);
  }

  function key(e: KeyboardEvent) {
    if (slideContextKey(e)) {
      e.preventDefault();
      e.stopPropagation();
      menuAtTile();
    }
  }

  function context(e: MouseEvent) {
    e.preventDefault();
    // The menu key and Shift+F10 also fire `contextmenu` (button 0): they open the menu, a
    // right-click (button 2) toggles disabled.
    if (e.button === 2) onRightClick();
    else menuAtTile();
  }
</script>

<div class="tile" class:selected class:dropping class:off={!slide.enabled} bind:this={el} {@attach observe}>
  <button
    class="slide"
    class:live
    class:next
    style:--gc={slide.color ?? "var(--group-neutral)"}
    {onclick}
    oncontextmenu={context}
    onkeydown={key}
    aria-label={aria}
    aria-current={live ? "true" : undefined}
    aria-pressed={selected}
    title={tip}
    data-slide={slide.id}
    data-key={occKey}
    data-drop-slide={dropKey}
  >
    <span class="frame" style:aspect-ratio={aspect}>
      {#if shown && !failed}
        <img src={shown} alt="" decoding="async" draggable="false" class:ready={loaded} onload={() => (loaded = true)} onerror={() => (failed = true)} />
      {/if}
      {#if !loaded || failed}
        <span class="ph">{failed ? "No preview" : ""}</span>
      {/if}
      {#if cue}
        <span class="cue" title={`Media cue: ${cue.name}`}>
          {#if cueIsImage}<ImageIcon size={11} />{:else}<Film size={11} />{/if}
          <span>{cue.name}</span>
        </span>
      {/if}
      {#if !slide.enabled}<span class="dis"><Chip kind="disabled" /></span>{/if}
    </span>
    <span class="bar">
      <span class="num">{number ?? "–"}</span>
      <span class="group">{label}</span>
      {#if live}<Chip kind="live" />{:else if next}<Chip kind="next" />{/if}
      {#if live && build}<span class="bld" title="Object builds: steps taken / steps (Next takes the next one)" data-build-progress>Build {build}</span>{/if}
      <span class="menu-space"></span>
    </span>
  </button>
  {#if cue}
    <button class="x" onclick={() => onRemoveCue(cue)} aria-label={`Remove media cue ${cue.name} from slide ${number ?? slide.index + 1}`} title="Remove media cue">
      <X size={12} />
    </button>
  {/if}
  <button
    class="more"
    onclick={(e) => {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      onMenu(r.left, r.bottom + 2);
    }}
    aria-label={`Slide ${number ?? slide.index + 1} actions`}
    title="Slide actions (Shift+F10)"
  >
    <Ellipsis size={14} />
  </button>
</div>

<style>
  .tile {
    position: relative;
    min-width: 0;
    border-radius: var(--r-card);
    outline: 2px dashed transparent;
    outline-offset: 2px;
  }
  .tile.dropping {
    outline-color: var(--ok);
  }
  .slide {
    width: 100%;
    display: flex;
    flex-direction: column;
    padding: 0;
    border: 2px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-well);
    color: var(--text);
    overflow: hidden;
    text-align: left;
  }
  .slide:hover {
    border-color: var(--line-hover);
  }
  /* Live beats selected beats next, as drawn. */
  .tile .slide.next {
    border-color: var(--next-ring);
  }
  .tile.selected .slide {
    border-color: var(--accent-ring);
  }
  .bld {
    flex: none;
    padding: 0 5px;
    border-radius: 4px;
    background: var(--accent-sel);
    color: var(--accent-link);
    font-size: 10.5px;
    font-weight: 700;
    line-height: 16px;
  }
  .tile .slide.live {
    border-color: var(--live);
    box-shadow: 0 0 12px rgba(245, 165, 36, 0.3);
  }
  .slide:focus-visible {
    outline: 2px solid var(--accent-ring);
    outline-offset: 2px;
  }
  .tile.off .frame,
  .tile.off .bar {
    opacity: 0.38;
  }
  .frame {
    position: relative;
    display: block;
    width: 100%;
    background: #000;
    overflow: hidden;
  }
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    opacity: 0;
    transition: opacity 120ms;
  }
  img.ready {
    opacity: 1;
  }
  .ph {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-disabled);
    font-size: 11px;
  }
  .cue {
    position: absolute;
    left: 6px;
    top: 6px;
    max-width: calc(100% - 40px);
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 2px 5px;
    border-radius: var(--r-xs);
    background: #000;
    color: #fff;
    font-size: 9.5px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
  }
  .cue span {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dis {
    position: absolute;
    right: 6px;
    bottom: 6px;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 22px;
    padding: 3px 6px;
    background: var(--gc);
    color: #fff;
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
  }
  .num {
    flex: none;
    font-variant-numeric: tabular-nums;
  }
  .group {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .menu-space {
    flex: none;
    width: 16px;
  }
  .x {
    position: absolute;
    right: 6px;
    top: 6px;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    padding: 0;
    border-radius: 50%;
    border: 1px solid var(--line-strong);
    background: #000;
    color: #fff;
  }
  .x:hover {
    background: var(--danger);
  }
  .more {
    position: absolute;
    right: 4px;
    bottom: 3px;
    width: 20px;
    height: 18px;
    display: grid;
    place-items: center;
    padding: 0;
    border: 0;
    border-radius: var(--r-xs);
    background: rgba(0, 0, 0, 0.35);
    color: #fff;
    opacity: 0;
  }
  .tile:hover .more,
  .tile:focus-within .more,
  .tile.selected .more {
    opacity: 1;
  }
  .more:hover {
    background: rgba(0, 0, 0, 0.6);
  }
</style>
