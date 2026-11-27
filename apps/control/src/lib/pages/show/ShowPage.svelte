<script lang="ts">
  // The Show page (operator view): library and playlists on the left (full height), the slides
  // over the bottom bin in the centre, the output, layers, transport and compact tabs on the
  // right, all resizable; the status bar below.
  import PanelBottomOpen from "lucide-svelte/icons/panel-bottom-open";
  import type { Connection } from "../../server";
  import { DEFAULT_LAYOUT, showView } from "../../state/show.svelte";
  import Splitter from "../../ui/Splitter.svelte";
  import StatusBar from "../../shell/StatusBar.svelte";
  import LibraryPanel from "./LibraryPanel.svelte";
  import SlideArea from "./SlideArea.svelte";
  import BottomBin from "./BottomBin.svelte";
  import RightColumn from "./RightColumn.svelte";

  let { conn }: { conn: Connection } = $props();

  let bodyHeight = $state(0);

  let media = $derived(showView.mediaVisible);
  let split = $derived(showView.layout.split);
  let rows = $derived(media ? `minmax(120px, ${split}fr) 1px minmax(110px, ${1 - split}fr)` : "1fr 0 auto");
  const BIN_NAMES = { media: "Media", audio: "Audio", props: "Props", macros: "Macros", inputs: "Video Inputs" } as const;
</script>

<div class="show">
  <div
    class="body"
    style:grid-template-columns={`${showView.layout.left}px 1px minmax(360px, 1fr) 1px ${showView.layout.right}px`}
    style:grid-template-rows={rows}
    bind:clientHeight={bodyHeight}
  >
    <div class="cell lib"><LibraryPanel /></div>
    <div class="vsplit" style:grid-column="2" style:grid-row="1 / -1">
      <Splitter dir="col" label="Resize the left column" value={showView.layout.left} defaultValue={DEFAULT_LAYOUT.left} onchange={(v) => showView.setLayout({ left: v })} />
    </div>
    <div class="cell center" style:grid-column="3" style:grid-row="1">
      <SlideArea {conn} />
    </div>
    {#if media}
      <div class="hsplit" style:grid-column="3" style:grid-row="2">
        <Splitter
          dir="row"
          label="Resize the media bin"
          value={split}
          defaultValue={DEFAULT_LAYOUT.split}
          scale={bodyHeight > 0 ? 1 / bodyHeight : 0}
          onchange={(v) => showView.setLayout({ split: v })}
        />
      </div>
      <div class="cell" style:grid-column="3" style:grid-row="3">
        <BottomBin {conn} />
      </div>
    {:else}
      <div class="cell folded" style:grid-column="3" style:grid-row="3">
        <button class="unfold" onclick={() => showView.setMediaVisible(true)} title="Show the bin (Ctrl+M)" aria-label="Show Media Bin">
          <PanelBottomOpen size={15} />Media Bin
        </button>
        <span class="meta">Hidden · {BIN_NAMES[showView.binTab]}</span>
      </div>
    {/if}
    <div class="vsplit" style:grid-column="4" style:grid-row="1 / -1">
      <Splitter dir="col" label="Resize the right column" value={showView.layout.right} defaultValue={DEFAULT_LAYOUT.right} scale={-1} onchange={(v) => showView.setLayout({ right: v })} />
    </div>
    <div class="cell right" style:grid-column="5" style:grid-row="1 / -1"><RightColumn {conn} /></div>
  </div>
  <StatusBar />
</div>

<style>
  .show {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: grid;
  }
  .cell {
    position: relative;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .lib {
    grid-column: 1;
    grid-row: 1 / -1;
  }
  .folded {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 8px;
    background: var(--surface-panel);
    border-top: 1px solid var(--line);
  }
  .unfold {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--r);
    background: none;
    color: var(--text);
    font-size: 12px;
    font-weight: 600;
  }
  .unfold:hover {
    background: var(--surface-raised);
  }
  .meta {
    color: var(--text-meta);
    font-size: 11px;
  }
  .vsplit,
  .hsplit {
    display: flex;
    min-width: 0;
    min-height: 0;
  }
  .vsplit :global(.splitter) {
    height: 100%;
  }
  .hsplit :global(.splitter) {
    width: 100%;
  }
</style>
