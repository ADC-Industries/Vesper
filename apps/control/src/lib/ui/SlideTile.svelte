<script lang="ts">
  // A slide tile: thumbnail (16:9 well), number and group label on a band in the group colour,
  // LIVE / NEXT / DISABLED chips, amber ring when live, grey ring when next, violet when selected.
  import type { Snippet } from "svelte";
  import Chip from "./Chip.svelte";

  let {
    src,
    number,
    label = "",
    groupColor,
    live = false,
    next = false,
    disabled = false,
    selected = false,
    aspect = 16 / 9,
    ariaLabel,
    onclick,
    oncontextmenu,
    children,
  }: {
    src?: string | null;
    number: number;
    label?: string;
    /** The band's colour (`null`: neutral). */
    groupColor?: string | null;
    live?: boolean;
    next?: boolean;
    disabled?: boolean;
    selected?: boolean;
    aspect?: number;
    ariaLabel?: string;
    onclick?: (e: MouseEvent) => void;
    oncontextmenu?: (e: MouseEvent) => void;
    /** Drawn over the thumbnail when there is no `src` (e.g. the slide's text). */
    children?: Snippet;
  } = $props();
</script>

<button
  type="button"
  class="ui-slide"
  class:live
  class:next={next && !live}
  class:selected
  class:disabled
  aria-label={ariaLabel ?? `Slide ${number}${label ? `, ${label}` : ""}${live ? ", live" : ""}`}
  aria-pressed={selected}
  {onclick}
  {oncontextmenu}
>
  <span class="well" style:aspect-ratio={aspect}>
    {#if src}<img {src} alt="" draggable="false" />{:else if children}{@render children()}{/if}
    <span class="chips">
      {#if live}<Chip kind="live" />{:else if next}<Chip kind="next" />{/if}
      {#if disabled}<Chip kind="disabled" />{/if}
    </span>
  </span>
  <span class="band" style:background={groupColor ?? "var(--group-neutral)"}>
    <span class="num">{number}</span>
    <span class="lbl">{label}</span>
  </span>
</button>

<style>
  .ui-slide {
    display: flex;
    flex-direction: column;
    min-width: 0;
    padding: 0;
    border: 0;
    border-radius: var(--r-card);
    background: var(--surface-well);
    color: var(--text-strong);
    overflow: hidden;
    box-shadow: 0 0 0 1px var(--line);
    text-align: left;
  }
  .ui-slide:hover {
    box-shadow: 0 0 0 1px var(--line-hover);
  }
  .ui-slide.selected {
    box-shadow: 0 0 0 2px var(--accent);
  }
  .ui-slide.next {
    box-shadow: 0 0 0 2px var(--next-ring);
  }
  .ui-slide.live {
    box-shadow: 0 0 0 3px var(--live);
  }
  .ui-slide.disabled .well,
  .ui-slide.disabled .band {
    opacity: 0.45;
  }
  .well {
    position: relative;
    display: block;
    width: 100%;
    background: #000;
  }
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .chips {
    position: absolute;
    top: 6px;
    left: 6px;
    display: flex;
    gap: 4px;
  }
  .band {
    display: flex;
    align-items: center;
    gap: var(--sp-3);
    min-height: 22px;
    padding: 2px 8px;
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  .lbl {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
