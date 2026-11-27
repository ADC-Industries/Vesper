<script lang="ts">
  import { onDestroy } from "svelte";
  import type { Connection, PreviewImage } from "../server";
  import { screensState } from "../state/screens.svelte";
  import { useLive } from "../live";

  let { conn }: { conn: Connection } = $props();

  const live = useLive();
  const layers = live.layers;
  const screens = live.screens;
  const cursor = live.cursor;

  // The right column already subscribes to the selected screen; show its frames bigger.
  // svelte-ignore state_referenced_locally
  let img = $state.raw<PreviewImage | null>(screensState.previewScreen ? (conn.previews.get(screensState.previewScreen) ?? null) : null);
  // svelte-ignore state_referenced_locally
  const off = conn.onPreview((p) => {
    if (p.screen === screensState.previewScreen) img = p;
  });
  onDestroy(off);

  let screen = $derived($screens.find((s) => s.id === screensState.previewScreen) ?? null);
</script>

<div class="live">
  <div class="frame" style:aspect-ratio={screen ? screen.width / screen.height : 16 / 9}>
    {#if img}<img src={img.url} alt={`Live picture of ${screen?.name ?? "the screen"}`} />{/if}
    {#if screen && !screen.enabled}<span class="off">{screen.name} is switched off</span>{/if}
  </div>
  <dl>
    <dt>Screen</dt>
    <dd>{screen?.name ?? "–"}{screen ? ` · ${screen.width}×${screen.height}` : ""}</dd>
    <dt>Slide</dt>
    <dd>
      {#if $layers?.slide}{$layers.slide.presentationName} · {$layers.slide.sectionName || "Slide"} {$layers.slide.slideIndex + 1}{:else}<span class="dim">–</span>{/if}
    </dd>
    <dt>Next</dt>
    <dd>{#if $cursor}{$cursor.presentationName} · slide {$cursor.slideIndex + 1}{:else}<span class="dim">–</span>{/if}</dd>
    <dt>Media</dt>
    <dd>{#if $layers?.media}{$layers.media.name}{#if !$layers.media.online}<span class="warn"> (missing)</span>{/if}{:else}<span class="dim">–</span>{/if}</dd>
    <dt>Announcements</dt>
    <dd>{#if $layers?.announcements}{$layers.announcements.presentationName}{:else}<span class="dim">–</span>{/if}</dd>
  </dl>
</div>

<style>
  .live {
    padding: 10px;
  }
  .frame {
    position: relative;
    width: 100%;
    background: #000;
    border: 1px solid var(--line);
  }
  img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .off {
    position: absolute;
    left: 8px;
    bottom: 8px;
    padding: 2px 8px;
    background: rgba(0, 0, 0, 0.7);
    color: var(--text);
    font-size: 12px;
  }
  dl {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 12px;
    margin: 10px 0 0;
    font-size: 12.5px;
  }
  dt {
    color: var(--text-label);
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    color: var(--text-disabled);
  }
  .warn {
    color: var(--warn);
  }
</style>
