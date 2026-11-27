<script lang="ts">
  // Under the Show page's layer buttons: the Video Input layer's live source (G7) with how it is
  // received, its sound's mute (the layer's), and Clear; or, once cleared, Bring back. Hidden
  // while the layer has neither.
  import X from "lucide-svelte/icons/x";
  import Undo from "lucide-svelte/icons/undo-2";
  import Cctv from "lucide-svelte/icons/cctv";
  import Volume2 from "lucide-svelte/icons/volume-2";
  import VolumeX from "lucide-svelte/icons/volume-x";
  import type { LiveSourceStatus } from "$protocol";
  import { session } from "../../state/session.svelte";
  import { sourcesState } from "../../state/sources.svelte";
  import { useLive } from "../../live";
  import { sourceName, statusLine } from "./sources";

  const live = useLive();
  const layers = live.layers;
  const restorable = live.restorable;
  const audio = live.audio;

  let layer = $derived($layers?.videoInput ?? null);
  let back = $derived(!layer && $restorable.includes("videoInput"));
  let entry = $derived(layer ? sourcesState.find(layer.source) : null);
  let status = $derived<LiveSourceStatus>(entry?.status ?? "connecting");
  let muted = $derived($audio?.layers.videoInput?.muted ?? false);

  $effect(() => {
    if (layer) sourcesState.ensure();
  });
</script>

{#if layer}
  <div class="strip on" class:bad={status === "lost" || status === "unsupported"} data-testid="video-input-strip">
    <Cctv size={14} aria-hidden="true" />
    <span class="what" title={layer.source}><b>{layer.name || sourceName(layer.source ?? "")}</b> · {statusLine(entry ?? { status, listed: true })}</span>
    <button
      class="x"
      class:muted
      onclick={() => session.run({ type: "setMute", target: "layer", layer: "videoInput", muted: !muted })}
      aria-pressed={muted}
      aria-label={muted ? "Unmute the Video Input layer's sound" : "Mute the Video Input layer's sound"}
      title={muted ? "Sound muted: click to unmute" : "Mute the source's sound"}
      data-testid="video-input-mute">{#if muted}<VolumeX size={13} />{:else}<Volume2 size={13} />{/if}</button>
    <button class="x" onclick={() => session.run({ type: "clearLayer", layer: "videoInput" })} title="Clear the Video Input layer" aria-label="Clear Video Input"><X size={13} /></button>
  </div>
{:else if back}
  <button class="strip back" onclick={() => session.run({ type: "restoreLayer", layer: "videoInput" })} title="Bring back the source cleared off the Video Input layer" data-testid="video-input-strip">
    <Undo size={13} aria-hidden="true" /><span class="what">Video Input cleared · bring it back</span>
  </button>
{/if}

<style>
  .strip {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-width: 0;
    height: 26px;
    padding: 0 4px 0 8px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text-2);
    font-size: 11.5px;
    text-align: left;
  }
  .strip.on {
    border-color: var(--live);
    background: var(--live-soft);
    color: var(--text);
  }
  .strip.on.bad {
    border-color: var(--danger);
    background: var(--danger-soft);
  }
  .what {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .x {
    flex: none;
    display: flex;
    padding: 3px;
    border: 0;
    border-radius: var(--r);
    background: none;
    color: var(--text-2);
  }
  .x:hover {
    color: var(--text);
    background: var(--surface-overlay);
  }
  .x.muted {
    color: var(--danger);
  }
</style>
