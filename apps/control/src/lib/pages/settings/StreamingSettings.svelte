<script lang="ts">
  // Settings ▸ Recording & streaming (docs/RECORD-STREAM-PLAN.md): the box's recording settings
  // (screen, format, codec, bit rates, split, folder, free-space floor) with Start / Stop, and
  // its stream targets (RTMP(S) with a key, SRT), each with its status and Start / Stop. Stream
  // keys are write-only: the field shows "Key saved" and is never filled from the server. An SRT
  // URL comes with its passphrase and stream ID hidden (`***`): it is kept as it is, or replaced
  // by typing the whole URL again (an edited copy would save `***` as the passphrase).
  import { onMount, tick } from "svelte";
  import type { RecordingSettings, StreamCodec, StreamKind, StreamTarget, RecordingContainer } from "$protocol";
  import { Button, NewButton, Select } from "../../ui";
  import { streaming } from "../../state/streaming.svelte";
  import { useLive } from "../../live";
  import { formatBytes, formatDuration, hasHiddenValue, serverKey } from "../../server/streaming";

  const live = useLive();
  const screens = live.screens;

  onMount(() => {
    void streaming.loadSettings();
    return streaming.watch();
  });

  let st = $derived(streaming.status);
  let cfg = $derived(streaming.settings);
  let encoders = $derived(cfg?.encoders ?? []);

  // ---- recording -----------------------------------------------------------------------------
  let rec = $state<RecordingSettings | null>(null);
  $effect(() => {
    if (cfg && !rec) rec = { ...cfg.recording };
  });
  let recDirty = $derived(!!rec && !!cfg && JSON.stringify(rec) !== JSON.stringify(cfg.recording));
  const CODECS: { value: StreamCodec; label: string }[] = [
    { value: "h264", label: "H.264" },
    { value: "hevc", label: "HEVC (H.265)" },
    { value: "av1", label: "AV1" },
  ];
  function codecOptions(current: StreamCodec) {
    return CODECS.map((c) => {
      const e = encoders.find((x) => x.codec === c.value);
      return { value: c.value, label: e ? `${c.label} · ${e.label}` : `${c.label} · no encoder`, disabled: !e && c.value !== current };
    });
  }
  let screenOptions = $derived([{ value: "", label: "First screen" }, ...$screens.map((s) => ({ value: s.id, label: s.name }))]);
  async function saveRec() {
    if (rec && (await streaming.saveRecording({ ...rec, folder: rec.folder?.trim() || null }))) rec = null;
  }

  // ---- targets -------------------------------------------------------------------------------
  let editing = $state<StreamTarget | null>(null);
  let keyText = $state("");
  let clearKey = $state(false);
  /** The SRT URL as the server shows it (values hidden), while the operator keeps it. */
  let shownUrl = $state<string | null>(null);
  /** The saved target's server (its key is kept only for the same server). */
  let savedServer = $state<string | null>(null);
  /** An RTMP target with a saved key pointed at another server: the key is entered again. */
  let keyForNewServer = $derived(
    !!editing && editing.kind === "rtmp" && editing.hasKey && savedServer !== null && serverKey(editing.kind, editing.url.trim()) !== savedServer,
  );
  let keyMissing = $derived(keyForNewServer && !keyText.trim() && !clearKey);
  let urlInput = $state<HTMLInputElement | null>(null);
  let keepsShownUrl = $derived(!!editing && editing.kind === "srt" && shownUrl !== null && editing.url === shownUrl);
  let urlHidesValues = $derived(!!editing && editing.kind === "srt" && !keepsShownUrl && hasHiddenValue(editing.url));
  function replaceUrl() {
    if (!editing) return;
    editing.url = "";
    shownUrl = null;
    void tick().then(() => urlInput?.focus());
  }
  function blank(): StreamTarget {
    return { id: "", name: "", kind: "rtmp", url: "", hasKey: false, screen: null, codec: "h264", videoMbps: 6, audioKbps: 160 };
  }
  function edit(t: StreamTarget | null) {
    editing = t ? { ...t } : blank();
    keyText = "";
    clearKey = false;
    shownUrl = t && t.kind === "srt" && hasHiddenValue(t.url) ? t.url : null;
    // The server's rule: the same server is the same kind and the same scheme, host and port
    // (R1-2: an SRT target switched to RTMP is another server).
    savedServer = t ? serverKey(t.kind, t.url) : null;
  }
  async function saveTarget() {
    if (!editing) return;
    const t: StreamTarget = { ...editing };
    if (t.kind === "srt") delete t.key; // an SRT target has no stream key (its field is hidden)
    else if (keyText.trim()) t.key = keyText.trim();
    else if (clearKey) t.key = "";
    else delete t.key;
    if (await streaming.saveTarget(t)) editing = null;
  }
  function streamOf(id: string) {
    return st?.streams.find((s) => s.id === id) ?? null;
  }
  const STATE_TEXT: Record<string, string> = {
    connecting: "Connecting…",
    live: "Live",
    retrying: "Reconnecting",
    failed: "Failed",
    idle: "Stopped",
  };
  const KINDS: { value: StreamKind; label: string }[] = [
    { value: "rtmp", label: "RTMP / RTMPS (YouTube, Facebook, …)" },
    { value: "srt", label: "SRT" },
  ];
  const CONTAINERS: { value: RecordingContainer; label: string }[] = [
    { value: "mp4", label: "MP4 (fragmented: safe if the power fails)" },
    { value: "mkv", label: "MKV (Matroska)" },
  ];
</script>

{#if !streaming.available}
  <section class="card"><span class="meta">This server can't record or stream yet (update the server).</span></section>
{:else}
  <section class="card" data-testid="recording-settings">
    <div class="head">
      <h2>Recording</h2>
      {#if st?.recording}
        <span class="pill rec">REC {formatDuration(st.recording.seconds)}</span>
      {/if}
      <span class="grow"></span>
      {#if st?.recording}
        <Button size="sm" disabled={streaming.busy} onclick={() => streaming.splitRecording()}>Split</Button>
        <Button size="sm" variant="danger" disabled={streaming.busy} onclick={() => streaming.stopRecording()}>Stop recording</Button>
      {:else}
        <Button size="sm" variant="primary" disabled={streaming.busy || !encoders.length} onclick={() => streaming.startRecording()}>Start recording</Button>
      {/if}
    </div>
    {#if st?.recording}
      {@const r = st.recording}
      <div class="status">
        <span>{r.screenName}</span><span class="meta">{r.file}{r.part > 1 ? ` · part ${r.part}` : ""}</span>
        <span class="meta">{formatBytes(r.bytes)} · {(r.videoKbps / 1000).toFixed(1)} Mbit/s · {formatBytes(r.freeBytes)} free{r.skippedPictures ? ` · ${r.skippedPictures} pictures skipped` : ""}</span>
        {#if r.detail}<span class="err">{r.detail}</span>{/if}
      </div>
    {/if}
    {#if !encoders.length && cfg}
      <span class="err">This server has no video encoder (no GPU encoder and no CPU fallback in its FFmpeg).</span>
    {/if}
    {#if rec}
      <div class="grid">
        <label for="rec-screen">Screen</label>
        <Select id="rec-screen" options={screenOptions} value={rec.screen ?? ""} onchange={(v) => rec && (rec.screen = v || null)} />
        <label for="rec-format">Format</label>
        <Select id="rec-format" options={CONTAINERS} value={rec.container} onchange={(v) => rec && (rec.container = v)} />
        <label for="rec-codec">Video</label>
        <Select id="rec-codec" options={codecOptions(rec.codec)} value={rec.codec} onchange={(v) => rec && (rec.codec = v)} />
        <label for="rec-mbps">Bit rate</label>
        <span class="pair"><input id="rec-mbps" type="number" min="1" max="100" step="1" bind:value={rec.videoMbps} /> Mbit/s video · <input aria-label="Audio bit rate" type="number" min="64" max="512" step="32" bind:value={rec.audioKbps} /> kbit/s AAC</span>
        <label for="rec-split">New file every</label>
        <span class="pair"><input id="rec-split" type="number" min="0" max="1440" step="5" bind:value={rec.splitMinutes} /> minutes <span class="meta">(0: one file)</span></span>
        <label for="rec-folder">Folder</label>
        <input id="rec-folder" type="text" placeholder={cfg?.recordingsFolder ?? ""} value={rec.folder ?? ""} oninput={(e) => rec && (rec.folder = e.currentTarget.value)} />
        <label for="rec-free">Keep free</label>
        <span class="pair"><input id="rec-free" type="number" min="0" step="1" bind:value={rec.minFreeGb} /> GB <span class="meta">(a recording stops at half of it)</span></span>
      </div>
      <div class="row">
        <Button size="sm" variant="primary" disabled={!recDirty || streaming.busy} onclick={saveRec}>Save</Button>
        <Button size="sm" disabled={!recDirty} onclick={() => (rec = cfg ? { ...cfg.recording } : null)}>Revert</Button>
        <span class="meta">Recordings go to {cfg?.recordingsFolder} on the server.</span>
      </div>
    {/if}
  </section>

  <section class="card" data-testid="stream-targets">
    <div class="head"><h2>Stream targets</h2></div>
    {#if cfg && !cfg.targets.length && !editing}
      <span class="meta">No targets yet. Add your platform's RTMP server and stream key, or an SRT address.</span>
    {/if}
    {#each cfg?.targets ?? [] as t (t.id)}
      {@const s = streamOf(t.id)}
      <div class="target" data-testid="stream-target">
        <span class="dot" class:live={s?.state === "live"} class:warn={s?.state === "retrying" || s?.state === "failed"}></span>
        <div class="tinfo">
          <span>{t.name} <span class="meta">· {t.kind.toUpperCase()} · {t.videoMbps} Mbit/s</span></span>
          <span class="meta url">{t.url}{t.kind === "rtmp" ? (t.hasKey ? " · key saved" : " · no key") : ""}</span>
          {#if s}
            <span class="meta">{STATE_TEXT[s.state] ?? s.state} · {formatDuration(s.seconds)} · {(s.sentKbps / 1000).toFixed(1)} Mbit/s sent{s.reconnects ? ` · ${s.reconnects} reconnects` : ""}{s.dropped ? ` · ${s.dropped} dropped` : ""}</span>
            {#if s.detail}<span class="err">{s.detail}</span>{/if}
          {/if}
        </div>
        {#if s}
          <Button size="sm" variant="danger" disabled={streaming.busy} onclick={() => streaming.stopStream(t.id)}>Stop</Button>
        {:else}
          <Button size="sm" disabled={streaming.busy || !encoders.length || (t.kind === "rtmp" && !t.hasKey)} onclick={() => streaming.startStream(t.id)}>Start</Button>
        {/if}
        <Button size="sm" variant="ghost" onclick={() => edit(t)}>Edit</Button>
      </div>
    {/each}
    {#if editing}
      <div class="grid edit" data-testid="target-editor">
        <label for="t-name">Name</label>
        <input id="t-name" type="text" maxlength="64" placeholder="YouTube" bind:value={editing.name} />
        <label for="t-kind">Protocol</label>
        <Select id="t-kind" options={KINDS} value={editing.kind} onchange={(v) => editing && (editing.kind = v)} />
        <label for="t-url">{editing.kind === "rtmp" ? "Server URL" : "SRT URL"}</label>
        {#if keepsShownUrl}
          <span class="pair">
            <input id="t-url" class="grow" type="text" readonly value={editing.url} />
            <Button size="sm" onclick={replaceUrl}>Replace URL</Button>
          </span>
          <span></span>
          <span class="meta">The passphrase and stream ID stay hidden (***). To change the URL, type it again in full.</span>
        {:else}
          <input id="t-url" bind:this={urlInput} type="text" placeholder={editing.kind === "rtmp" ? "rtmps://a.rtmps.example.com/live2" : "srt://host:port?passphrase=…"} bind:value={editing.url} />
          {#if urlHidesValues}
            <span></span>
            <span class="err">Type the real passphrase and stream ID: *** stands for a hidden value.</span>
          {/if}
        {/if}
        {#if editing.kind === "rtmp"}
          <label for="t-key">Stream key</label>
          <span class="pair">
            <input id="t-key" type="password" autocomplete="off" placeholder={editing.hasKey && !clearKey && !keyForNewServer ? "Key saved (type to replace)" : "Paste the stream key"} bind:value={keyText} />
            {#if editing.hasKey}<label class="chk"><input type="checkbox" bind:checked={clearKey} /> Remove the key</label>{/if}
          </span>
          {#if keyMissing}
            <span></span>
            <span class="err" data-testid="key-again">Enter the stream key again for the new server: the saved key is kept only for the same server.</span>
          {/if}
        {/if}
        <label for="t-screen">Screen</label>
        <Select id="t-screen" options={screenOptions} value={editing.screen ?? ""} onchange={(v) => editing && (editing.screen = v || null)} />
        <label for="t-codec">Video</label>
        <Select id="t-codec" options={codecOptions(editing.codec)} value={editing.codec} onchange={(v) => editing && (editing.codec = v)} />
        <label for="t-mbps">Bit rate</label>
        <span class="pair"><input id="t-mbps" type="number" min="0.5" max="50" step="0.5" bind:value={editing.videoMbps} /> Mbit/s video · <input aria-label="Audio bit rate" type="number" min="64" max="320" step="32" bind:value={editing.audioKbps} /> kbit/s AAC</span>
      </div>
      <div class="row">
        <Button size="sm" variant="primary" disabled={streaming.busy || !editing.name.trim() || !editing.url.trim() || urlHidesValues || keyMissing} onclick={saveTarget}>{editing.id ? "Save" : "Add target"}</Button>
        <Button size="sm" onclick={() => (editing = null)}>Cancel</Button>
        {#if editing.id}
          {@const id = editing.id}
          <span class="grow"></span>
          <Button size="sm" variant="danger" onclick={async () => { if (await streaming.deleteTarget(id)) editing = null; }}>Delete target</Button>
        {/if}
      </div>
      <span class="meta">The key is kept on the server only: it is never shown again, backed up or sent to other computers.</span>
    {:else}
      <NewButton label="New stream target" onclick={() => edit(null)} />
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
    gap: 8px;
  }
  h2 {
    margin: 0;
    font-size: var(--fs);
  }
  .grow {
    flex: 1;
  }
  .grid {
    display: grid;
    grid-template-columns: 140px minmax(0, 420px);
    gap: 8px 14px;
    align-items: center;
  }
  .grid > label {
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
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .status {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .target {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 0;
    border-bottom: 1px solid var(--surface-raised);
  }
  .tinfo {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .url {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-meta);
    flex: none;
  }
  .dot.live {
    background: var(--live);
  }
  .dot.warn {
    background: var(--danger);
  }
  .pill {
    padding: 1px 8px;
    border-radius: 10px;
    font-size: 11px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .pill.rec {
    background: var(--live);
    color: var(--live-ink);
  }
  .chk {
    display: flex;
    align-items: center;
    gap: 4px;
    color: var(--text-2);
  }
  .edit {
    padding-top: 6px;
    border-top: 1px solid var(--line);
  }
  .meta {
    color: var(--text-meta);
  }
  .err {
    color: var(--danger-text);
  }
</style>
