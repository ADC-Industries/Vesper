<script lang="ts">
  // The right column's transport: Video (the video on the Media layer, or a foreground video
  // on the Slide layer) or Audio (the Audio layer). Name and state, a seek bar (arrow keys skip
  // 5 s), previous / play-pause / stop / next, loop on / off for what is playing and the layer's
  // volume (dB; the slider maps 0–100). Positions arrive at most 4 times a second; between
  // them the bar runs on the local clock.
  import Play from "lucide-svelte/icons/play";
  import Pause from "lucide-svelte/icons/pause";
  import Square from "lucide-svelte/icons/square";
  import SkipBack from "lucide-svelte/icons/skip-back";
  import SkipForward from "lucide-svelte/icons/skip-forward";
  import Repeat from "lucide-svelte/icons/repeat";
  import Volume2 from "lucide-svelte/icons/volume-2";
  import VolumeX from "lucide-svelte/icons/volume-x";
  import type { Layer, MediaTransport } from "$protocol";
  import { session } from "../../state/session.svelte";
  import { mediaState } from "../../state/media.svelte";
  import { showView, type TransportTarget } from "../../state/show.svelte";
  import { useLive } from "../../live";
  import { RESTART_AFTER, formatTime, fraction, positionAt, seekTarget, stepAction } from "./transport";
  import { audioTransport, binRows, dbToFader, faderToDb, formatDb, playlistRows, roundDb, throttle } from "./audio";

  const live = useLive();
  const transport = live.transport;
  const audioLayer = live.audioLayer;
  const audio = live.audio;

  const TARGETS: { id: TransportTarget; label: string }[] = [
    { id: "video", label: "Video" },
    { id: "audio", label: "Audio" },
  ];

  let isVideo = $derived(showView.transportTarget === "video");
  let target = $derived.by((): MediaTransport | null => {
    if (!isVideo) return audioTransport($audioLayer);
    const t = $transport;
    return t && t.layer !== "audio" ? t : null;
  });
  /** The layer the buttons and the volume act on. */
  let layer = $derived<Layer>(isVideo ? (target?.layer ?? "media") : "audio");
  let layerName = $derived(layer === "slide" ? "Slide" : layer === "audio" ? "Audio" : "Media");

  // ---- position between state updates ---------------------------------------------------------
  let received = $state(performance.now());
  let now = $state(performance.now());
  $effect(() => {
    void target;
    received = performance.now();
  });
  $effect(() => {
    if (target?.state !== "playing") return;
    let raf = 0;
    const tick = () => {
      now = performance.now();
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  });
  let scrubbing = $state<number | null>(null);
  let shownPos = $derived(target ? (scrubbing ?? positionAt(target, Math.max(0, now - received))) : 0);
  let playing = $derived(target?.state === "playing");
  let timed = $derived(!!target && target.duration != null && target.duration > 0);
  let trackEl = $state<HTMLElement | null>(null);

  let status = $derived(
    !target ? "Idle" : target.state === "playing" ? "Playing" : target.state === "paused" ? "Paused" : "Stopped",
  );
  let note = $derived(
    isVideo
      ? "Nothing is playing on the Media layer. Click a video in the bin; stills have no transport."
      : "The Audio layer is clear. Click a track in the bin's Audio tab.",
  );

  // ---- buttons ----------------------------------------------------------------------------------
  function run(type: "mediaToggle" | "mediaStop" | "mediaRestart") {
    if (!target) return;
    const layer = target.layer;
    // One literal per type: TypeScript checks a union discriminant against `Command` only up to
    // 25 combinations, and three types × every layer is more.
    void session.run(type === "mediaToggle" ? { type, layer } : type === "mediaStop" ? { type, layer } : { type, layer });
  }
  function skip(seconds: number) {
    if (target) void session.run({ type: "mediaSkip", seconds, layer: target.layer });
  }

  /** Videos of the bin shown in the Media tab (what Previous / Next step through). */
  let binVideos = $derived((mediaState.mediaBin?.items ?? []).filter((i) => i.kind === "video" && i.enabled && i.availability !== "missing").map((i) => i.asset));
  /** Tracks of the Audio tab's bin (a playlist steps on the server). */
  let binTracks = $derived.by(() => {
    if (mediaState.audioSource?.kind === "bin" && mediaState.audioBin) return binRows(mediaState.audioBin).flatMap((r) => (r.asset && r.online ? [r.asset] : []));
    if (mediaState.audioPlaylist) return playlistRows(mediaState.audioPlaylist, mediaState.audioDurations).flatMap((r) => (r.asset ? [r.asset] : []));
    return [];
  });
  let ctx = $derived($audio?.playlist ?? null);

  function step(d: 1 | -1) {
    const a = stepAction({ d, target, position: shownPos, list: isVideo ? binVideos : binTracks, serverSteps: !isVideo && !!ctx });
    if (!a) return;
    if (a.kind === "restart") return run("mediaRestart");
    if (a.kind === "server") return void session.run({ type: a.next ? "audioNext" : "audioPrev" });
    void session.run(isVideo ? { type: "triggerMedia", asset: a.asset } : { type: "triggerMedia", asset: a.asset, layer: "audio" });
  }
  let canStep = $derived(isVideo ? layer === "media" && binVideos.length > 0 : !!ctx || binTracks.length > 0);
  let noun = $derived(isVideo ? "video" : "track");

  // ---- seeking ------------------------------------------------------------------------------------
  function scrubAt(e: PointerEvent): number {
    const r = trackEl!.getBoundingClientRect();
    return seekTarget(e.clientX, r.left, r.width, target?.duration);
  }
  function scrubStart(e: PointerEvent) {
    if (!target || !trackEl || !timed) return;
    trackEl.setPointerCapture(e.pointerId);
    scrubbing = scrubAt(e);
  }
  function scrubMove(e: PointerEvent) {
    if (scrubbing !== null) scrubbing = scrubAt(e);
  }
  function scrubEnd(e: PointerEvent) {
    if (scrubbing === null || !target) return;
    const seconds = scrubAt(e);
    scrubbing = null;
    void session.run({ type: "mediaSeek", seconds, layer: target.layer });
  }
  function scrubKey(e: KeyboardEvent) {
    if (!target) return;
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") {
      e.preventDefault();
      skip(e.key === "ArrowLeft" ? -5 : 5);
    }
  }

  // ---- volume -------------------------------------------------------------------------------------
  let volLayer = $derived<"audio" | "media" | "slide">(layer === "slide" ? "slide" : layer === "audio" ? "audio" : "media");
  let layerDb = $derived($audio?.layers[volLayer].db ?? 0);
  let muted = $derived($audio?.layers[volLayer].muted ?? false);
  let dragDb = $state<number | null>(null);
  let release: ReturnType<typeof setTimeout> | null = null;
  let shownDb = $derived(dragDb ?? layerDb);
  const send = throttle((v: { layer: "audio" | "media" | "slide"; db: number }) => void session.run({ type: "setVolume", target: "layer", layer: v.layer, db: v.db }), 60);

  function onVolume(e: Event) {
    const db = roundDb(faderToDb(Number((e.currentTarget as HTMLInputElement).value) / 100));
    dragDb = db;
    if (release) clearTimeout(release);
    send({ layer: volLayer, db });
  }
  function onVolumeDone() {
    send.flush();
    if (release) clearTimeout(release);
    release = setTimeout(() => (dragDb = null), 500);
  }
</script>

<div class="transport" data-testid="transport">
  <div class="head">
    <span class="section-title">Transport</span>
    <div class="targets" role="group" aria-label="Transport target">
      {#each TARGETS as t (t.id)}
        <button class:on={showView.transportTarget === t.id} aria-pressed={showView.transportTarget === t.id} onclick={() => (showView.transportTarget = t.id)}>{t.label}</button>
      {/each}
    </div>
  </div>

  <div class="card" class:idle={!target}>
    <div class="what">
      <span class="tname" title={target?.name ?? ""}>{target ? target.name : isVideo ? "No video playing" : "No track playing"}</span>
      <span class="tstate" data-state={target?.state ?? "idle"}>{status}{target ? ` · ${layerName} layer` : ""}</span>
    </div>

    {#if target && timed}
      <div class="seek">
        <span class="t" data-testid="transport-time">{formatTime(shownPos)}</span>
        <div
          class="track"
          class:scrub={scrubbing !== null}
          bind:this={trackEl}
          role="slider"
          tabindex="0"
          aria-label="Position"
          aria-valuemin={0}
          aria-valuemax={target.duration ?? 0}
          aria-valuenow={Math.round(shownPos * 10) / 10}
          aria-valuetext={`${formatTime(shownPos)} of ${formatTime(target.duration)}`}
          title="Drag to seek · arrow keys skip 5 s"
          onpointerdown={scrubStart}
          onpointermove={scrubMove}
          onpointerup={scrubEnd}
          onpointercancel={() => (scrubbing = null)}
          onkeydown={scrubKey}
        >
          <span class="fill" style:width={`${fraction(shownPos, target.duration) * 100}%`}></span>
          <span class="knob" style:left={`${fraction(shownPos, target.duration) * 100}%`}></span>
        </div>
        <span class="t">-{formatTime(Math.max(0, (target.duration ?? 0) - shownPos))}</span>
      </div>
    {:else}
      <span class="note">{target ? "A still or live picture: nothing to seek." : note}</span>
    {/if}

    <div class="buttons">
      <button class="tb" disabled={!target && !canStep} onclick={() => step(-1)} title={`Previous ${noun} (restarts after ${RESTART_AFTER} s)`} aria-label={`Previous ${noun}`}><SkipBack size={15} /></button>
      <button class="tb pp" disabled={!target} onclick={() => run("mediaToggle")} aria-label={playing ? "Pause" : "Play"} title={playing ? "Pause" : "Play"} data-testid="transport-play">
        {#if playing}<Pause size={15} fill="currentColor" />{:else}<Play size={15} fill="currentColor" />{/if}
      </button>
      <button class="tb" disabled={!target} onclick={() => run("mediaStop")} title={`Stop the ${layerName} layer`} aria-label={`Stop the ${layerName} layer`}><Square size={13} fill="currentColor" /></button>
      <button class="tb" disabled={!canStep} onclick={() => step(1)} title={`Next ${noun}`} aria-label={`Next ${noun}`}><SkipForward size={15} /></button>
      <button
        class="tb"
        class:on={!!target?.loop}
        disabled={!target}
        onclick={() => target && session.run({ type: "mediaSetLoop", layer: target.layer, loop: !target.loop })}
        aria-pressed={!!target?.loop}
        title={target ? (target.loop ? "Loop: on (click to play to the end and stop)" : "Loop: off (click to play it again and again)") : "Loop"}
        aria-label="Loop"
        data-testid="transport-loop"
      >
        <Repeat size={15} />
      </button>
      <label class="vol" title={`${layerName} layer: ${formatDb(shownDb)} dB`}>
        <button
          class="mute"
          class:muted
          onclick={() => session.run({ type: "setMute", target: "layer", layer: volLayer, muted: !muted })}
          aria-label={muted ? `Unmute the ${layerName} layer` : `Mute the ${layerName} layer`}
          aria-pressed={muted}
          title={muted ? "Muted: click to unmute" : "Mute"}
        >
          {#if muted}<VolumeX size={15} />{:else}<Volume2 size={15} />{/if}
        </button>
        <input
          type="range"
          min="0"
          max="100"
          step="0.5"
          value={dbToFader(shownDb) * 100}
          oninput={onVolume}
          onchange={onVolumeDone}
          ondblclick={() => {
            dragDb = null;
            void session.run({ type: "setVolume", target: "layer", layer: volLayer, db: 0 });
          }}
          aria-label={`${layerName} layer volume`}
          aria-valuetext={`${formatDb(shownDb)} dB`}
        />
        <span class="db" class:muted>{formatDb(shownDb)}</span>
      </label>
    </div>
  </div>
</div>

<style>
  .transport {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-top: 4px;
  }
  .targets {
    display: flex;
    gap: 2px;
  }
  .targets button {
    padding: 3px 10px;
    border: 1px solid var(--line);
    border-radius: 5px;
    background: var(--surface-raised);
    color: var(--text-2);
    font-size: 11px;
    font-weight: 600;
  }
  .targets button.on {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px;
    border: 1px solid var(--line);
    border-radius: var(--r-card);
    background: var(--surface-raised);
  }
  .what {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .tname {
    font-weight: 600;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card.idle .tname {
    color: var(--text-2);
  }
  .tstate {
    font-size: 11px;
    font-weight: 600;
    color: var(--ok-text);
  }
  .tstate[data-state="paused"] {
    color: var(--info-text);
  }
  .tstate[data-state="stopped"],
  .tstate[data-state="idle"] {
    color: var(--text-meta);
  }
  .note {
    font-size: 11px;
    color: var(--text-2);
  }
  .seek {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 11px;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .track {
    position: relative;
    flex: 1;
    min-width: 0;
    height: 5px;
    border-radius: 3px;
    background: var(--surface-overlay);
    cursor: pointer;
    touch-action: none;
  }
  .track:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 3px;
  }
  .track .fill {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    border-radius: 3px;
    background: var(--accent-ring);
  }
  .track .knob {
    position: absolute;
    top: 50%;
    width: 11px;
    height: 11px;
    margin-left: -5.5px;
    border-radius: 50%;
    background: var(--text-strong);
    transform: translateY(-50%);
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.4);
  }
  .track.scrub .knob {
    width: 13px;
    height: 13px;
    margin-left: -6.5px;
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
  }
  .tb {
    flex: none;
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-panel);
    color: var(--text);
  }
  .tb:hover:not(:disabled) {
    background: var(--surface-overlay);
  }
  .tb:disabled {
    color: var(--text-disabled);
  }
  .tb.pp {
    width: 38px;
    border: 0;
    background: var(--accent);
    color: #fff;
  }
  .tb.pp:disabled {
    background: var(--surface-overlay);
    color: var(--text-disabled);
  }
  .tb.on {
    border-color: var(--accent-ring);
    background: var(--accent-sel);
  }
  .vol {
    flex: 1 1 130px;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    padding-left: 2px;
    color: var(--text-2);
  }
  .vol input {
    flex: 1;
    min-width: 0;
    accent-color: var(--accent-ring);
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
    background: none;
    color: var(--text-2);
  }
  .mute:hover {
    background: var(--surface-overlay);
  }
  .mute.muted {
    color: var(--danger);
  }
  .db {
    flex: none;
    min-width: 30px;
    text-align: right;
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .db.muted {
    color: var(--text-meta);
    text-decoration: line-through;
  }
</style>
