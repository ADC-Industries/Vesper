<script lang="ts">
  // The Show page's bottom bin (under the slides): tabs Media / Audio / Props / Macros / Video
  // Inputs (live network sources, G7), the bin playlists on the left and the picked one's items
  // on the right. Hide folds it to a one-line bar (View ▸ Media Bin,
  // Ctrl+M).
  import Plus from "lucide-svelte/icons/plus";
  import Layers from "lucide-svelte/icons/layers";
  import SquareM from "lucide-svelte/icons/square-m";
  import PanelBottomClose from "lucide-svelte/icons/panel-bottom-close";
  import type { Connection } from "../../server";
  import { mediaState } from "../../state/media.svelte";
  import { showDocsState } from "../../state/showDocs.svelte";
  import { showView, type BinTab } from "../../state/show.svelte";
  import Tabs from "../../ui/Tabs.svelte";
  import EmptyState from "../../ui/EmptyState.svelte";
  import MediaTree from "./MediaTree.svelte";
  import MediaBinPanel from "./MediaBinPanel.svelte";
  import AudioSources from "./AudioSources.svelte";
  import AudioPanel from "./AudioPanel.svelte";
  import PropsPanel from "./PropsPanel.svelte";
  import VideoInputsPanel from "./VideoInputsPanel.svelte";

  let { conn }: { conn: Connection } = $props();

  const TABS: { value: BinTab; label: string; disabled?: boolean; title?: string }[] = [
    { value: "media", label: "Media" },
    { value: "audio", label: "Audio" },
    { value: "props", label: "Props" },
    { value: "macros", label: "Macros" },
    { value: "inputs", label: "Video Inputs", title: "Cameras and other network video sources (IDN Open Protocol, compatible with NDI®)" },
  ];

  const HINTS: Record<BinTab, string> = {
    media: "Click to send to the Media layer · drag onto a slide to add a media cue",
    audio: "Click a track to play it on the Audio layer",
    props: "Click a prop to toggle it on the Props layer",
    macros: "",
    inputs: "Click a source to put it on the Video Input layer",
  };

  let tab = $derived(showView.binTab);
</script>

<section class="bin" aria-label="Media Bin" data-testid="bottom-bin">
  <div class="bar">
    <Tabs tabs={TABS} value={tab} label="Bin" idPrefix="bin" onchange={(v) => showView.setBinTab(v)} />
    <span class="grow"></span>
    {#if HINTS[tab]}<span class="hint">{HINTS[tab]}</span>{/if}
    <button class="hide" onclick={() => showView.setMediaVisible(false)} title="Hide the bin (Ctrl+M)" aria-label="Hide Media Bin">
      <PanelBottomClose size={15} />Hide
    </button>
  </div>
  <div class="body" role="tabpanel" id="bin-panel-{tab}" aria-labelledby="bin-tab-{tab}">
    {#if tab !== "macros" && tab !== "inputs"}
      <nav class="lists" aria-label={tab === "media" ? "Media bins" : tab === "audio" ? "Audio sources" : "Prop lists"}>
        <div class="lhead">
          <span class="section-title">Playlists</span>
          {#if tab === "audio"}
            <button class="ibtn" onclick={() => mediaState.newAudioPlaylist()} title="New audio playlist" aria-label="New audio playlist"><Plus size={15} /></button>
          {:else}
            <button class="ibtn" disabled title="Making {tab} playlists needs a server update" aria-label="New {tab} playlist"><Plus size={15} /></button>
          {/if}
        </div>
        {#if tab === "media"}
          <MediaTree />
        {:else if tab === "audio"}
          <AudioSources />
        {:else}
          <ul class="one">
            <li>
              <button class="row sel" aria-current="true">
                <Layers size={15} /><span class="name">All Props</span><span class="count">{showDocsState.propDocs.length}</span>
              </button>
            </li>
          </ul>
        {/if}
      </nav>
    {/if}
    <div class="content">
      {#if tab === "media"}
        <MediaBinPanel {conn} />
      {:else if tab === "audio"}
        <AudioPanel />
      {:else if tab === "props"}
        <PropsPanel {conn} />
      {:else if tab === "inputs"}
        <VideoInputsPanel {conn} />
      {:else}
        <EmptyState icon={SquareM} title="Macros">
          Macros (one button for a look, props, media and clears together) arrive in a later version.
        </EmptyState>
      {/if}
    </div>
  </div>
</section>

<style>
  .bin {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface-panel);
  }
  .bar {
    flex: none;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 0 8px;
    border-bottom: 1px solid var(--line);
  }
  .bar :global(.ui-tabs) {
    border-bottom: 0;
    flex: none;
  }
  .grow {
    flex: 1 1 10px;
  }
  .hint {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    padding: 0 8px;
    color: var(--text-meta);
    font-size: 11px;
  }
  .hide {
    flex: none;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--r);
    background: none;
    color: var(--text-2);
    font-size: 12px;
  }
  .hide:hover {
    color: var(--text);
    background: var(--surface-raised);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .lists {
    flex: 0 0 190px;
    min-width: 0;
    overflow-y: auto;
    padding: 4px 0 6px;
    border-right: 1px solid var(--line);
  }
  .lhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 2px 6px 4px 10px;
  }
  .content {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .content > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .one {
    list-style: none;
    margin: 0;
    padding: 0 6px;
  }
  .row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
    font-size: 12px;
    text-align: left;
  }
  .row.sel {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .name {
    flex: 1;
  }
  .count {
    color: var(--text-meta);
    font-size: 11px;
  }
</style>
