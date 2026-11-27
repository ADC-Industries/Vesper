<script lang="ts">
  // Settings ▸ Media (docs/MEDIA-PREP-PLAN.md): media preparation on the Codex box: automatic on
  // upload or only when asked (analysis and conversion both), the priority conversions run at
  // and what they do while something is live, the quality (a preset or an explicit bit rate, the
  // codec, a size limit), the encoder (with what this box has), the loudness target and jobs at
  // once; the converter found, and the jobs running now. On a show box: where it runs.
  import { onMount } from "svelte";
  import type { MediaPrepSettings } from "$protocol";
  import { Button, Select, Switch } from "../../ui";
  import { mediaPrep } from "../../state/mediaPrep.svelte";
  import {
    CODEC_CHOICES,
    LIVE_CHOICES,
    PRIORITY_CHOICES,
    SIZE_CHOICES,
    encoderChoices,
    encoderProblem,
    encodersFor,
    encodersLine,
    formatMbps,
    parseMbps,
    qualityChoices,
    sameSettings,
  } from "./mediaPrep";

  onMount(() => mediaPrep.watch());

  let st = $derived(mediaPrep.status);
  let edit = $state<MediaPrepSettings | null>(null);
  let rateText = $state("");
  $effect(() => {
    if (st && !edit) {
      edit = { ...st.settings };
      rateText = formatMbps(st.settings.videoKbps);
    }
  });
  let rate = $derived(parseMbps(rateText));
  /** The settings as they would be saved. */
  let wanted = $derived.by((): MediaPrepSettings | null => {
    if (!edit) return null;
    const w: MediaPrepSettings = { ...edit };
    delete w.videoKbps;
    if (typeof rate === "number") w.videoKbps = rate;
    return w;
  });
  let dirty = $derived(!!wanted && !!st && !sameSettings(wanted, st.settings));
  const WHEN = [
    { value: "auto", label: "Automatically, when media are uploaded" },
    { value: "manual", label: "Only when someone asks (Prepare on a media item)" },
  ] as const;
  let when = $derived(edit?.enabled ? "auto" : "manual");
  /** This box's encoders for the codec being edited (saved or not). */
  let boxEncoders = $derived(st && edit ? encodersFor(st, edit.video) : null);
  let encoders = $derived(boxEncoders ? encoderChoices(boxEncoders) : []);
  let problem = $derived(boxEncoders && edit ? encoderProblem(edit.encoder, boxEncoders) : null);
  let running = $derived((st?.jobs ?? []).filter((j) => j.step !== "done"));
  async function save() {
    if (wanted && rate !== null && (await mediaPrep.saveSettings(wanted))) edit = null;
  }
</script>

{#if mediaPrep.elsewhere}
  <section class="card"><span class="meta">Media preparation runs on {mediaPrep.elsewhere}, where the media library is. Change its settings there.</span></section>
{:else if !st || !edit}
  <section class="card"><span class="meta">Loading…</span></section>
{:else}
  <section class="card" data-testid="media-prep-settings">
    <div class="head"><h2>Media preparation</h2></div>
    <p class="meta">Videos and sounds are analysed (their loudness, evened out at playback without changing the file; a poster frame that isn't black; whether they're hard to play live) and those hard to play live are converted to the house format, the original kept.</p>
    <div class="grid">
      <label for="prep-when">Prepare</label>
      <Select id="prep-when" options={[...WHEN]} value={when} onchange={(v) => edit && (edit.enabled = v === "auto")} />
      <span class="lab">Convert</span>
      <Switch label="Convert videos that are hard to play live" showLabel bind:checked={edit.transcode} />
    </div>

    <h3>Priority</h3>
    <div class="grid">
      <label for="prep-priority">Conversions</label>
      <span class="pair"><Select id="prep-priority" options={PRIORITY_CHOICES} bind:value={edit.priority} /><span class="meta">{edit.priority === "low" ? "below the show and everything else" : "as other programs: faster when the box is idle"}</span></span>
      <label for="prep-live">While live</label>
      <Select id="prep-live" options={LIVE_CHOICES} bind:value={edit.whileLive} />
    </div>
    <p class="meta">{edit.whileLive === "lowPriority" ? "Media added during a service are still converted, at the lowest priority, so the outputs come first." : "Conversions stop while something is live and start again once it's cleared; analysis still runs."}</p>

    <h3>Quality</h3>
    <div class="grid">
      <label for="prep-video">Codec</label>
      <Select id="prep-video" options={CODEC_CHOICES} bind:value={edit.video} />
      <label for="prep-quality">Quality</label>
      <Select id="prep-quality" options={qualityChoices(edit.video)} bind:value={edit.quality} disabled={typeof rate === "number"} />
      <label for="prep-rate">Bit rate</label>
      <span class="pair"><input id="prep-rate" type="text" inputmode="decimal" placeholder="by quality" bind:value={rateText} aria-invalid={rate === null} /> Mb/s <span class="meta">{rate === null ? "a number, 1–60" : "optional: the same for every video"}</span></span>
      <label for="prep-size">Size</label>
      <Select id="prep-size" options={SIZE_CHOICES} bind:value={edit.maxSize} />
    </div>

    <h3>Encoder</h3>
    <div class="grid">
      <label for="prep-encoder">Encoder</label>
      <Select id="prep-encoder" options={encoders} bind:value={edit.encoder} />
    </div>
    <span class="meta" data-testid="media-prep-encoders">{encodersLine(boxEncoders ?? {})}{edit.encoder === "auto" ? " · Auto takes the CPU's while something is live" : ""}</span>
    {#if problem}<span class="err">{problem}</span>{/if}

    <h3>Sound and jobs</h3>
    <div class="grid">
      <label for="prep-lufs">Loudness target</label>
      <span class="pair"><input id="prep-lufs" type="number" min="-30" max="-5" step="1" bind:value={edit.loudnessTarget} /> LUFS <span class="meta">(−16 suits most rooms; −23 is broadcast)</span></span>
      <label for="prep-jobs">Jobs at once</label>
      <span class="pair"><input id="prep-jobs" type="number" min="1" max="4" step="1" bind:value={edit.maxJobs} /></span>
    </div>
    <div class="row">
      <Button size="sm" variant="primary" disabled={!dirty || rate === null || mediaPrep.busy} onclick={save}>Save</Button>
      {#if dirty}<Button size="sm" onclick={() => (edit = null)}>Cancel</Button>{/if}
    </div>
  </section>
  <section class="card" data-testid="media-prep-box">
    <div class="head"><h2>This box</h2>{#if st.live}<span class="pill">LIVE</span>{/if}</div>
    {#if st.converter}
      <span class="meta">Converter: {st.converter}{st.encoder ? ` · encoder now ${st.encoder}` : " · no encoder for the house format"} · each conversion runs in a process of its own</span>
    {:else}
      <span class="err">Nothing converts on this box: videos are analysed but not converted.</span>
    {/if}
    {#if st.live}<span class="meta">{st.settings.whileLive === "wait" ? "Something is live: conversions wait until it's cleared." : "Something is live: conversions run at the lowest priority."}</span>{/if}
    {#if running.length}
      {#each running as j (j.asset)}
        <div class="job">
          <span class="jname">{j.name}</span>
          <span class="meta">{j.step === "converting" ? `Converting ${Math.round(j.progress * 100)} %` : j.step === "waitingForLive" ? "After the show" : j.step === "failed" ? `Failed: ${j.error ?? ""}` : j.step === "analysing" ? "Analysing" : "Waiting"}</span>
        </div>
      {/each}
    {:else}
      <span class="meta">Nothing is being prepared.</span>
    {/if}
  </section>
{/if}

<style>
  .card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border-radius: var(--r-card);
    background: var(--surface-panel);
    border: 1px solid var(--line);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: var(--fs);
  }
  h3 {
    margin: 6px 0 0;
    font-size: var(--fs-sm);
    color: var(--text-label);
    font-weight: 600;
  }
  .grid {
    display: grid;
    grid-template-columns: 140px minmax(0, 420px);
    gap: 8px 14px;
    align-items: center;
  }
  .grid > label,
  .lab {
    color: var(--text-label);
  }
  .pair {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .pair input[type="number"] {
    width: 72px;
  }
  .pair input[type="text"] {
    width: 96px;
  }
  .pair input[aria-invalid="true"] {
    border-color: var(--live);
  }
  .row {
    display: flex;
    gap: 8px;
  }
  .meta {
    color: var(--text-meta);
  }
  p.meta {
    margin: 0;
    line-height: 1.45;
  }
  .err {
    color: var(--live);
  }
  .pill {
    padding: 1px 8px;
    border-radius: var(--r-pill);
    background: var(--live-soft);
    color: var(--live);
    font-size: var(--fs-xs);
    font-weight: 700;
  }
  .job {
    display: flex;
    justify-content: space-between;
    gap: 12px;
  }
  .jname {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
