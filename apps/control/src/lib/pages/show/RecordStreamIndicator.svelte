<script lang="ts">
  // The Show page's recording / streaming indicator (docs/RECORD-STREAM-PLAN.md): beside the
  // output buttons, "REC 12:34" and "LIVE 2" in amber (on air) while the server records or
  // streams, red while a stream retries or a recording failed, grey when idle. Its popover starts and stops
  // the recording (and splits it) and each stream target; the targets themselves are set up in
  // Settings ▸ Recording & streaming.
  import { onMount } from "svelte";
  import Circle from "lucide-svelte/icons/circle";
  import Radio from "lucide-svelte/icons/radio";
  import { Button, Popover } from "../../ui";
  import { streaming } from "../../state/streaming.svelte";
  import { shell } from "../../state/shell.svelte";
  import { formatBytes, formatDuration, indicator } from "../../server/streaming";

  let open = $state(false);
  onMount(() => streaming.watch());
  // The targets afresh each time it opens: another Oriel may have changed them (F10-2).
  $effect(() => {
    if (open) void streaming.loadSettings();
  });

  let st = $derived(streaming.status);
  let ind = $derived(indicator(st));
  let busy = $derived(ind.recording || ind.streaming > 0);
  let label = $derived(
    ind.recording && ind.streaming
      ? `REC · LIVE ${ind.streaming}`
      : ind.recording
        ? "REC"
        : ind.streaming
          ? `LIVE${ind.streaming > 1 ? ` ${ind.streaming}` : ""}`
          : "REC / Stream",
  );
  let targets = $derived(streaming.settings?.targets ?? []);
  function streamOf(id: string) {
    return st?.streams.find((s) => s.id === id) ?? null;
  }
  const STATE_TEXT: Record<string, string> = {
    idle: "Stopped",
    connecting: "Connecting…",
    live: "Live",
    retrying: "Reconnecting",
    failed: "Failed",
  };
  function openSettings() {
    open = false;
    shell.openSettings("Streaming");
  }
</script>

<div class="anchor">
  <button
    class="ind"
    class:busy
    class:trouble={ind.trouble}
    onclick={() => (open = !open)}
    aria-haspopup="dialog"
    aria-expanded={open}
    title={busy ? `Recording and streaming: ${label}` : "Record or stream a screen"}
    aria-label={busy ? `${label} ${formatDuration(ind.seconds)}` : "Record or stream"}
    data-testid="rec-indicator"
  >
    {#if ind.recording}<Circle size={10} fill="currentColor" aria-hidden="true" />{:else}<Radio size={13} aria-hidden="true" />{/if}
    <span class="lbl">{label}</span>
    {#if busy}<span class="num">{formatDuration(ind.seconds)}</span>{/if}
  </button>
  {#if open}
    <Popover label="Recording and streaming" width={300} align="right" onclose={() => (open = false)}>
      <div class="pop" data-testid="rec-popover">
        <h3>Recording</h3>
        {#if st?.recording}
          {@const r = st.recording}
          <div class="line">
            <span class="dot" class:on={r.state === "live"} class:warn={r.state === "failed" || r.state === "retrying"}></span>
            <span class="grow">{r.screenName} · {formatDuration(r.seconds)} · {formatBytes(r.bytes)}</span>
          </div>
          <div class="meta">{r.file}{r.part > 1 ? ` (part ${r.part})` : ""} · {formatBytes(r.freeBytes)} free</div>
          {#if r.detail}<div class="err">{r.detail}</div>{/if}
          <div class="row">
            <Button size="sm" variant="danger" disabled={streaming.busy} onclick={() => streaming.stopRecording()}>Stop recording</Button>
            <Button size="sm" disabled={streaming.busy} onclick={() => streaming.splitRecording()} title="Close this file and go on in a new one">Split</Button>
          </div>
        {:else}
          <div class="row">
            <Button size="sm" variant="primary" disabled={streaming.busy} onclick={() => streaming.startRecording()}>Start recording</Button>
          </div>
        {/if}
        <h3>Streams</h3>
        {#if !targets.length}
          <div class="meta">No stream targets yet.</div>
        {/if}
        {#each targets as t (t.id)}
          {@const s = streamOf(t.id)}
          <div class="line">
            <span class="dot" class:on={s?.state === "live"} class:warn={s?.state === "retrying" || s?.state === "failed"}></span>
            <span class="grow">
              {t.name}
              <span class="meta">{s ? `${STATE_TEXT[s.state] ?? s.state} · ${formatDuration(s.seconds)}${s.sentKbps ? ` · ${(s.sentKbps / 1000).toFixed(1)} Mbit/s` : ""}` : t.kind.toUpperCase()}</span>
            </span>
            {#if s}
              <Button size="sm" variant="danger" disabled={streaming.busy} onclick={() => streaming.stopStream(t.id)}>Stop</Button>
            {:else}
              <Button size="sm" disabled={streaming.busy || (t.kind === "rtmp" && !t.hasKey)} title={t.kind === "rtmp" && !t.hasKey ? "Add the stream key in Settings" : `Start streaming to ${t.name}`} onclick={() => streaming.startStream(t.id)}>Start</Button>
            {/if}
          </div>
          {#if s?.detail}<div class="err">{s.detail}</div>{/if}
        {/each}
        <div class="row foot"><Button size="sm" variant="ghost" onclick={openSettings}>Recording & streaming settings…</Button></div>
      </div>
    </Popover>
  {/if}
</div>

<style>
  .anchor {
    position: relative;
    flex: none;
  }
  .ind {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 100%;
    padding: 6px 10px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text-2);
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
  }
  .ind.busy {
    border-color: var(--live);
    background: var(--live-soft);
    color: var(--live);
  }
  .ind.trouble {
    border-color: var(--danger);
    background: var(--danger-soft);
    color: var(--danger-text);
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  /* A narrow right column (1366 px wide screens): the dot and the time only. */
  @container (max-width: 360px) {
    .lbl {
      display: none;
    }
    .ind {
      padding: 6px 8px;
    }
  }
  .pop {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
  }
  h3 {
    margin: 4px 0 0;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-label);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .grow {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .row {
    display: flex;
    gap: 6px;
  }
  .foot {
    margin-top: 4px;
    border-top: 1px solid var(--line);
    padding-top: 6px;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-meta);
    flex: none;
  }
  .dot.on {
    background: var(--live);
  }
  .dot.warn {
    background: var(--danger);
  }
  .meta {
    color: var(--text-meta);
    font-size: 11px;
  }
  .err {
    color: var(--danger-text);
    font-size: 11px;
  }
</style>
