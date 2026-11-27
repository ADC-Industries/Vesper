<script lang="ts">
  // The Show page's right column, as the canvas draws it: Audience / Stage output buttons, the
  // screen picker, the live preview (LIVE / STAND-BY, the stereo meter overlaid), Clear All and
  // the five layer buttons (a lit one clears its layer; an empty one brings back what was cleared
  // off it while the server keeps it), the transport, and the compact tabs Timers / Messages /
  // Stage.
  import Volume2 from "lucide-svelte/icons/volume-2";
  import VolumeX from "lucide-svelte/icons/volume-x";
  import Undo from "lucide-svelte/icons/undo-2";
  import type { ScreenRole } from "$protocol";
  import type { Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import { screensState } from "../../state/screens.svelte";
  import { showView, type RightTab } from "../../state/show.svelte";
  import { useLive } from "../../live";
  import Tabs from "../../ui/Tabs.svelte";
  import { formatDb } from "./audio";
  import { rateLabel } from "./transport";
  import { layerAction, type LayerButtonId } from "./layers";
  import LivePreview from "./LivePreview.svelte";
  import MasterMeters from "./MasterMeters.svelte";
  import Transport from "./Transport.svelte";
  import VideoInputStrip from "./VideoInputStrip.svelte";
  import CaptionsStrip from "./CaptionsStrip.svelte";
  import RecordStreamIndicator from "./RecordStreamIndicator.svelte";
  import { streaming } from "../../state/streaming.svelte";
  import CompactTimers from "./CompactTimers.svelte";
  import CompactMessages from "./CompactMessages.svelte";
  import CompactStage from "./CompactStage.svelte";

  let { conn }: { conn: Connection } = $props();

  const live = useLive();
  const activity = live.activity;
  const restorable = live.restorable;
  const screens = live.screens;
  const screensEnabled = live.screensEnabled;
  const audio = live.audio;

  // ---- outputs and preview ----------------------------------------------------------------------
  const OUTPUTS: { role: ScreenRole; label: string; key: string }[] = [
    { role: "audience", label: "Audience", key: "Ctrl+1" },
    { role: "stage", label: "Stage", key: "Ctrl+2" },
  ];
  let screenId = $derived(screensState.previewScreen ?? $screens[0]?.id ?? null);
  let screen = $derived($screens.find((s) => s.id === screenId) ?? null);
  let onAir = $derived(!!screen && screen.enabled && (screen.role === "stage" ? $screensEnabled.stage : $screensEnabled.audience));

  let masterMuted = $derived($audio?.muted ?? false);
  let outputState = $derived($audio?.outputState ?? "null");
  let masterTitle = $derived(
    `Master ${formatDb($audio?.masterDb ?? 0)} dB` +
      (outputState === "error" ? " · audio device problem (Settings ▸ Audio)" : outputState === "null" ? " · no local audio device" : ""),
  );

  // ---- layers -------------------------------------------------------------------------------------
  type LayerButton = { id: LayerButtonId; label: string; name: string; key: string };
  const LAYERS: LayerButton[] = [
    { id: "media", label: "Media", name: "Media", key: "F3" },
    { id: "slide", label: "Slide", name: "Slide", key: "F2" },
    { id: "props", label: "Props", name: "Props", key: "F5" },
    { id: "messages", label: "Msgs", name: "Messages", key: "F6" },
    { id: "audio", label: "Audio", name: "Audio", key: "F4" },
  ];
  function press(id: LayerButtonId) {
    for (const c of layerAction(id, $activity, $restorable).commands) void session.run(c);
  }
  function hint(l: LayerButton, kind: "clear" | "restore" | "none"): string {
    if (kind === "clear") return `Clear the ${l.name} layer (${l.key})${l.id === "slide" ? "; announcements too" : ""}`;
    if (kind === "restore") return `Bring back what was cleared off the ${l.name} layer${l.id === "audio" || l.id === "media" ? " (it plays from the start)" : ""}`;
    return `The ${l.name} layer is clear`;
  }
  let anyActive = $derived(Object.values($activity).some(Boolean));

  // ---- compact tabs -------------------------------------------------------------------------------
  const TABS: { value: RightTab; label: string }[] = [
    { value: "timers", label: "Timers" },
    { value: "messages", label: "Messages" },
    { value: "stage", label: "Stage" },
  ];
  let tabsEl = $state<HTMLElement | null>(null);
  $effect(() => {
    if (showView.focusRequest === 0 || !tabsEl) return;
    tabsEl.querySelector<HTMLButtonElement>(`#side-tab-${showView.rightTab}`)?.focus({ preventScroll: true });
  });
</script>

<aside class="right" aria-label="Output and layers">
  <div class="top">
    <div class="outs">
      {#each OUTPUTS as o (o.role)}
        {@const on = $screensEnabled[o.role]}
        <button
          class="out"
          class:on
          aria-pressed={on}
          onclick={() => session.run({ type: "setScreensEnabled", role: o.role, enabled: !on })}
          title={`${o.label} screens ${on ? "on: click to send black" : "off: click to switch on"} (${o.key})`}
          aria-label={`${o.label} output ${on ? "on" : "off"}`}
          data-testid={`output-${o.role}`}
        >
          <span class="ring" aria-hidden="true"></span>
          <span>{o.label}</span>
        </button>
      {/each}
      {#if streaming.available}<RecordStreamIndicator />{/if}
    </div>

    <div class="screen-row">
      <label class="screen-sel">
        <span>Screen</span>
        <select value={screenId ?? ""} onchange={(e) => (screensState.previewScreen = e.currentTarget.value)} aria-label="Preview screen">
          {#each $screens as s (s.id)}
            <option value={s.id}>{s.name}</option>
          {/each}
        </select>
      </label>
      <span class="grow"></span>
      {#if screen}<span class="meta">{screen.width} × {screen.height} · {rateLabel(screen.frameRate.num, screen.frameRate.den)}</span>{/if}
      <button
        class="mute"
        class:on={masterMuted}
        class:warn={outputState === "error"}
        onclick={() => session.run({ type: "setMute", target: "master", muted: !masterMuted })}
        title={masterMuted ? "Master muted: click to unmute" : `Mute the master (${masterTitle})`}
        aria-label={masterMuted ? "Unmute master audio" : "Mute master audio"}
        aria-pressed={masterMuted}
        data-testid="master-mute"
      >
        {#if masterMuted}<VolumeX size={14} />{:else}<Volume2 size={14} />{/if}
      </button>
    </div>

    <LivePreview {conn} live={onAir}>
      <MasterMeters {conn} />
    </LivePreview>
  </div>

  <div class="controls">
    <button class="clear-all" class:lit={anyActive} onclick={() => session.run({ type: "clearAll" })} title="Clear all layers (F1)" data-testid="clear-all">Clear All</button>
    <div class="layers" role="group" aria-label="Layers">
      {#each LAYERS as l (l.id)}
        {@const kind = layerAction(l.id, $activity, $restorable).kind}
        <button
          class="lyr"
          class:on={kind === "clear"}
          class:back={kind === "restore"}
          style:--c={`var(--layer-${l.id})`}
          aria-disabled={kind === "none"}
          onclick={() => press(l.id)}
          title={hint(l, kind)}
          aria-label={kind === "clear" ? `Clear ${l.name}` : kind === "restore" ? `Restore ${l.name}` : `${l.name} (clear)`}
          data-testid={`layer-${l.id}`}
        >
          {#if kind === "restore"}<Undo size={11} strokeWidth={2.4} aria-hidden="true" />{:else}<span class="dot" aria-hidden="true"></span>{/if}{l.label}
        </button>
      {/each}
    </div>
    <VideoInputStrip />
    <CaptionsStrip />
    <Transport />
  </div>

  <div class="side" bind:this={tabsEl}>
    <Tabs tabs={TABS} value={showView.rightTab} label="Controls" idPrefix="side" fill onchange={(v) => showView.setRightTab(v)} />
    <div class="panel" role="tabpanel" id="side-panel-{showView.rightTab}" aria-labelledby="side-tab-{showView.rightTab}">
      {#if showView.rightTab === "timers"}
        <CompactTimers {conn} />
      {:else if showView.rightTab === "messages"}
        <CompactMessages {conn} />
      {:else}
        <CompactStage />
      {/if}
    </div>
  </div>
</aside>

<style>
  .right {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow-y: auto;
    background: var(--surface-panel);
    container-type: inline-size;
  }
  .top {
    flex: none;
    padding: 12px 12px 10px;
  }
  .outs {
    display: flex;
    gap: 8px;
    margin-bottom: 10px;
  }
  .out {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 6px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r-card);
    background: var(--surface-raised);
    color: var(--text-2);
    font-size: 12px;
    font-weight: 600;
  }
  .out .ring {
    width: 14px;
    height: 14px;
    box-sizing: border-box;
    border-radius: 50%;
    border: 2.5px solid var(--text-meta);
  }
  .out.on {
    border-color: color-mix(in srgb, var(--ok) 45%, var(--line));
    background: var(--ok-bg);
    color: var(--text);
  }
  .out.on .ring {
    border-color: var(--ok);
    background: var(--ok);
  }
  .out:hover {
    filter: brightness(1.12);
  }
  .screen-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 10px;
  }
  .screen-sel {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    color: var(--text-2);
    font-size: 12px;
  }
  .screen-sel select {
    min-width: 0;
    max-width: 180px;
    font-size: 12px;
  }
  .grow {
    flex: 1 1 0;
  }
  .meta {
    color: var(--text-meta);
    font-size: 11px;
    white-space: nowrap;
  }
  @container (max-width: 340px) {
    .meta {
      display: none;
    }
  }
  .mute {
    flex: none;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
  }
  .mute:hover {
    color: var(--text-strong);
    background: var(--surface-raised);
  }
  .mute.warn {
    color: var(--warn);
  }
  .mute.on {
    color: var(--text-strong);
    background: var(--danger-bg);
  }
  .controls {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 0 12px 12px;
  }
  .clear-all {
    padding: 8px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, var(--line));
    border-radius: var(--r);
    background: var(--danger-soft);
    color: var(--danger-text);
    font-weight: 700;
    font-size: 13px;
  }
  .clear-all.lit {
    color: #ffd9d9;
  }
  .clear-all:hover {
    background: color-mix(in srgb, var(--danger) 26%, transparent);
  }
  .layers {
    display: grid;
    grid-template-columns: repeat(5, minmax(0, 1fr));
    gap: 4px;
  }
  .lyr {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    min-width: 0;
    padding: 7px 2px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-panel);
    color: var(--text-meta);
    font-size: 11px;
    font-weight: 600;
  }
  .lyr .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--surface-overlay);
  }
  .lyr.on {
    border-color: var(--c);
    background: var(--surface-raised);
    color: var(--text);
  }
  .lyr.on .dot {
    background: var(--c);
  }
  .lyr.on:hover {
    background: var(--surface-overlay);
  }
  .lyr.back {
    border-style: dashed;
    border-color: var(--c);
    color: var(--text-2);
  }
  .lyr.back :global(svg) {
    margin: -3px 0 -2px;
    color: var(--c);
  }
  .lyr.back:hover {
    background: var(--surface-raised);
    color: var(--text);
  }
  .lyr[aria-disabled="true"] {
    cursor: default;
  }
  .side {
    flex: 1 0 auto;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--line);
  }
  .side :global(.ui-tabs) {
    padding: 0 8px;
  }
  .panel {
    flex: 1;
  }
</style>
