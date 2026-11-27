<script lang="ts">
  import Button from "../../ui/Button.svelte";
  // Settings ▸ Audio: the machine's audio device and master gain (stored on the server and
  // applied live), the bus's state, and a test tone.
  import RefreshCw from "lucide-svelte/icons/refresh-cw";
  import type { AudioDeviceConfig, AudioDeviceInfo } from "$protocol";
  import { ApiRequestError } from "../../server";
  import { session } from "../../state/session.svelte";
  import { shell } from "../../state/shell.svelte";
  import { useLive } from "../../live";
  import { FADER_MAX_DB, FADER_MIN_DB, deviceOptions, formatDb } from "../show/audio";

  const audio = useLive().audio;

  let devices = $state.raw<AudioDeviceInfo[]>([]);
  let saved = $state.raw<AudioDeviceConfig | null>(null);
  let device = $state("");
  let gain = $state(0);
  /** Gains are shown and edited to 0.1 dB. */
  const r1 = (v: number) => Math.round(v * 10) / 10;
  let loading = $state(true);
  let saving = $state(false);
  let error = $state<string | null>(null);

  let options = $derived(deviceOptions(devices, device));
  let dirty = $derived(!!saved && (device !== (saved.device ?? "") || gain !== r1(saved.masterGainDb ?? 0)));

  function message(e: unknown): string {
    if (e instanceof ApiRequestError && (e.code === "notFound" || e.status === 404 || e.status === 405))
      return "This server does not have audio settings. Update the server.";
    return e instanceof Error ? e.message : String(e);
  }

  async function load(config = true) {
    const conn = session.conn;
    if (!conn) return;
    loading = true;
    error = null;
    try {
      const [devs, cfg] = await Promise.all([conn.rest.audioDevices(), config ? conn.rest.audioDevice() : Promise.resolve(saved)]);
      devices = devs;
      if (config && cfg) {
        saved = cfg;
        device = cfg.device ?? "";
        gain = r1(cfg.masterGainDb ?? 0);
      }
    } catch (e) {
      error = message(e);
    } finally {
      loading = false;
    }
  }
  void load();

  async function save() {
    const conn = session.conn;
    if (!conn || !saved) return;
    saving = true;
    error = null;
    try {
      const cfg: AudioDeviceConfig = { ...saved, schemaVersion: saved.schemaVersion || 1, device, masterGainDb: gain };
      saved = await conn.rest.saveAudioDevice(cfg);
      device = saved.device ?? "";
      gain = r1(saved.masterGainDb ?? 0);
      conn.pushNotice("info", device ? `Audio now plays on ${device}.` : "Local audio is off.");
    } catch (e) {
      error = message(e);
    } finally {
      saving = false;
    }
  }

  function setGain(v: number) {
    if (!Number.isFinite(v)) return;
    gain = Math.round(Math.min(FADER_MAX_DB, Math.max(FADER_MIN_DB, v)) * 10) / 10;
  }

  let status = $derived.by((): { cls: string; text: string } => {
    const a = $audio;
    if (!a) return { cls: "off", text: "Not connected" };
    if (a.outputState === "ok") return { cls: "ok", text: `Playing on ${a.device}` };
    if (a.outputState === "error") return { cls: "bad", text: `Cannot use ${a.device || "the device"}${a.detail ? `: ${a.detail}` : ""}` };
    return { cls: "off", text: "No local audio device (sound still goes to IDN outputs that embed audio)" };
  });
</script>

<div class="audio">
  <div class="status {status.cls}" data-testid="audio-status">
    <span class="dot"></span>
    <span class="text">{status.text}</span>
  </div>
  {#if $audio}
    <div class="sub">
      Master {formatDb($audio.masterDb)} dB{$audio.muted ? " · muted" : ""} · buffer underruns: {$audio.xruns}
    </div>
  {/if}

  {#if error}<p class="err" role="alert">{error}</p>{/if}

  <label class="fld">
    <span>Output device</span>
    <span class="line">
      <select bind:value={device} disabled={loading || !saved} data-testid="audio-device">
        {#each options as o (o.value)}
          <option value={o.value} disabled={o.disabled} title={o.title}>{o.label}</option>
        {/each}
      </select>
      <button class="ibtn" onclick={() => load(false)} disabled={loading} title="Look for devices again" aria-label="Refresh device list">
        <RefreshCw size={14} />
      </button>
    </span>
  </label>

  <label class="fld">
    <span>Master gain</span>
    <span class="line">
      <input
        class="slim gain"
        type="range"
        min={FADER_MIN_DB}
        max={FADER_MAX_DB}
        step="0.5"
        value={gain}
        disabled={!saved}
        oninput={(e) => setGain(Number(e.currentTarget.value))}
        aria-label="Master gain"
      />
      <input
        class="num"
        type="number"
        min={FADER_MIN_DB}
        max={FADER_MAX_DB}
        step="0.5"
        value={gain}
        disabled={!saved}
        onchange={(e) => setGain(Number(e.currentTarget.value))}
        aria-label="Master gain, dB"
      />
      <span class="unit">dB</span>
    </span>
  </label>

  <div class="actions">
    <Button onclick={() => session.run({ type: "playTestTone", frequency: 1000, seconds: 2 })} title="Play a 1 kHz tone for 2 seconds on the master bus">
      Test tone
    </Button>
    <span class="grow"></span>
    {#if dirty}<Button onclick={() => load()} disabled={saving}>Revert</Button>{/if}
    <Button variant="primary" onclick={save} disabled={!dirty || saving} data-testid="audio-save">{saving ? "Saving…" : "Save"}</Button>
  </div>

  <p class="note">
    Saving applies at once: the device is reopened and the master gain set.
    Audio over the network (IDN Open Protocol) is switched on per output, with <em>Embed audio</em> in
    <button class="link" onclick={() => shell.appAction("pageScreens")}>Configure Screens</button>.
  </p>
</div>

<style>
  .audio {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-width: 480px;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dot {
    flex: none;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--line-hover);
  }
  .status.ok .dot {
    background: var(--ok);
  }
  .status.bad .dot {
    background: var(--danger);
  }
  .status.bad .text {
    color: var(--danger-text);
  }
  .text {
    user-select: text;
  }
  .sub {
    margin-top: -6px;
    color: var(--text-label);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .err {
    margin: 0;
    color: var(--danger-text);
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .fld > span:first-child {
    color: var(--text-2);
  }
  .line {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .line select {
    flex: 1;
    min-width: 0;
    min-height: 28px;
  }
  .gain {
    flex: 1;
  }
  .num {
    width: 64px;
  }
  .unit {
    color: var(--text-label);
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .grow {
    flex: 1;
  }
  .note {
    margin: 0;
    color: var(--text-label);
    font-size: 12.5px;
    line-height: 1.45;
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-ring);
    text-decoration: underline;
  }
</style>
