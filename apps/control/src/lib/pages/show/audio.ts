// Audio helpers (plain TypeScript, unit tested): dB labels and fader law, meter ballistics,
// the audio panel's rows and which one is playing, and the audio layer as a transport.

import type {
  AudioDeviceInfo,
  MediaBinDetail,
  MediaLayerState,
  MediaTransport,
  PlaylistContext,
  PlaylistDetail,
  PlaylistTreeNode,
} from "$protocol";
import { formatTime } from "./transport";

/** Bottom of the meter scale (the server reports silence as −60 dBFS). */
export const METER_FLOOR = -60;
/** What the fader sends at its very bottom (the server clamps to −96 … +12 dB). */
export const FADER_OFF_DB = -96;
export const FADER_MIN_DB = -60;
export const FADER_MAX_DB = 12;
/** Fader position (0–1) of unity gain. */
const UNITY_POS = 0.75;

// ---- dB ------------------------------------------------------------------------------------------

/** `0.0`, `-6.5`, `+3.0`; at or below the fader's off position `-∞`. */
export function formatDb(db: number): string {
  if (!Number.isFinite(db) || db <= FADER_OFF_DB) return "-∞";
  const r = Math.round(db * 10) / 10;
  const s = (Object.is(r, -0) ? 0 : r).toFixed(1);
  return r > 0 ? `+${s}` : s;
}

/** Fader position (0–1) → gain, dB: 0 = off, 0.75 = unity, 1 = +12 dB; −60 … 0 dB is linear
 * over the first three quarters. */
export function faderToDb(pos: number): number {
  const p = Math.min(1, Math.max(0, pos));
  if (p <= 0) return FADER_OFF_DB;
  if (p <= UNITY_POS) return FADER_MIN_DB + (p / UNITY_POS) * -FADER_MIN_DB;
  return ((p - UNITY_POS) / (1 - UNITY_POS)) * FADER_MAX_DB;
}

/** Gain, dB → fader position (inverse of `faderToDb`). */
export function dbToFader(db: number): number {
  if (!(db > FADER_MIN_DB)) return 0;
  if (db <= 0) return ((db - FADER_MIN_DB) / -FADER_MIN_DB) * UNITY_POS;
  return UNITY_POS + (Math.min(db, FADER_MAX_DB) / FADER_MAX_DB) * (1 - UNITY_POS);
}

/** Rounds a fader's dB to what is worth sending (0.1 dB, or off). */
export function roundDb(db: number): number {
  return db <= FADER_OFF_DB ? FADER_OFF_DB : Math.round(db * 10) / 10;
}

// ---- meters --------------------------------------------------------------------------------------

/** Meter height fraction (0–1) of a level on the −60 … 0 dBFS scale. */
export function meterFraction(db: number): number {
  if (!Number.isFinite(db)) return 0;
  return Math.min(1, Math.max(0, (db - METER_FLOOR) / -METER_FLOOR));
}

export type MeterZone = "green" | "yellow" | "red";

/** Colour zone of a level: green below −18 dBFS, yellow to −6, red above. */
export function meterZone(db: number): MeterZone {
  return db >= -6 ? "red" : db >= -18 ? "yellow" : "green";
}

export interface MeterChannel {
  /** Shown level, dBFS. */
  level: number;
  /** Peak hold, dBFS. */
  peak: number;
  /** When the peak was last raised (ms). */
  peakAt: number;
}

/**
 * Meter ballistics: a new level rises at once and falls at `fallDbPerSec`; the peak marker
 * holds for `holdMs`, then falls the same way. `step` is called once per animation frame.
 */
export class MeterBallistics {
  channels: MeterChannel[] = [];
  private targets: number[] = [];
  private last: number | null = null;

  constructor(
    private readonly fallDbPerSec = 24,
    private readonly holdMs = 1500,
  ) {}

  /** New levels from the server (dBFS per channel). */
  setLevels(levels: number[], now: number): void {
    this.targets = levels.map((l) => (Number.isFinite(l) ? Math.max(METER_FLOOR, l) : METER_FLOOR));
    while (this.channels.length < this.targets.length) this.channels.push({ level: METER_FLOOR, peak: METER_FLOOR, peakAt: now });
    this.channels.length = this.targets.length;
    for (let i = 0; i < this.targets.length; i++) {
      const c = this.channels[i]!;
      const t = this.targets[i]!;
      if (t >= c.level) c.level = t;
      if (t >= c.peak) {
        c.peak = t;
        c.peakAt = now;
      }
    }
  }

  /** Advances to `now` (ms). Returns whether anything is still moving. */
  step(now: number): boolean {
    const dt = this.last === null ? 0 : Math.max(0, Math.min(250, now - this.last)) / 1000;
    this.last = now;
    const fall = this.fallDbPerSec * dt;
    let moving = false;
    for (let i = 0; i < this.channels.length; i++) {
      const c = this.channels[i]!;
      const t = this.targets[i] ?? METER_FLOOR;
      c.level = c.level > t ? Math.max(t, c.level - fall) : t;
      if (now - c.peakAt > this.holdMs) c.peak = Math.max(c.level, c.peak - fall);
      if (c.level > t || c.peak > c.level || t > METER_FLOOR) moving = true;
    }
    return moving;
  }

  /** Forget the clock (call when the animation restarts after a pause). */
  resetClock(): void {
    this.last = null;
  }
}

// ---- audio panel ---------------------------------------------------------------------------------

/** What the audio panel's list shows. */
export type AudioSource = { kind: "bin"; id: string } | { kind: "playlist"; id: string };

export interface AudioRow {
  /** Bin item or playlist item ID. */
  id: string;
  name: string;
  /** Media asset (`null` = not a media item: a header or presentation in the playlist). */
  asset: string | null;
  /** Seconds (`null` = unknown). */
  duration: number | null;
  /** The file is on the server. */
  online: boolean;
}

/** Rows of a bin (`durations` fills in unknown lengths, by asset). */
export function binRows(bin: MediaBinDetail): AudioRow[] {
  return bin.items.map((it) => ({
    id: it.id,
    name: it.name,
    asset: it.asset,
    duration: it.duration,
    online: it.availability !== "missing",
  }));
}

/** Rows of a playlist; lengths come from `durations` (asset → seconds) when known. */
export function playlistRows(pl: PlaylistDetail, durations: ReadonlyMap<string, number | null>): AudioRow[] {
  return pl.items.map((it) => {
    const asset = it.kind.type === "media" ? it.kind.asset : null;
    return {
      id: it.id,
      name: it.displayName,
      asset,
      duration: asset ? (durations.get(asset) ?? null) : null,
      online: true,
    };
  });
}

/** Whether `row` of `source` is what the audio layer is playing. */
export function isPlayingRow(
  row: AudioRow,
  source: AudioSource,
  layer: MediaLayerState | null,
  playlist: PlaylistContext | null,
): boolean {
  if (!layer) return false;
  if (source.kind === "playlist") return !!playlist && playlist.playlist === source.id && playlist.item === row.id;
  return !playlist && row.asset !== null && row.asset === layer.asset;
}

/** `3:05`, or `–:––` when unknown. */
export function durationLabel(seconds: number | null | undefined): string {
  return seconds == null || !Number.isFinite(seconds) ? "–:––" : formatTime(seconds);
}

/** Every audio playlist of the tree, depth first, labelled with its folder path. */
export function audioPlaylists(nodes: PlaylistTreeNode[], path = ""): { id: string; name: string; label: string; itemCount: number }[] {
  return nodes.flatMap((n) =>
    n.kind === "folder"
      ? audioPlaylists(n.children, `${path}${n.name} / `)
      : n.purpose === "audio"
        ? [{ id: n.id, name: n.name, label: `${path}${n.name}`, itemCount: n.itemCount }]
        : [],
  );
}

/** The tree without audio playlists (they live in the audio panel). Empty folders stay. */
export function withoutAudioPlaylists(nodes: PlaylistTreeNode[]): PlaylistTreeNode[] {
  const out: PlaylistTreeNode[] = [];
  for (const n of nodes) {
    if (n.kind === "folder") out.push(n.children.length ? { ...n, children: withoutAudioPlaylists(n.children) } : n);
    else if (n.purpose !== "audio") out.push(n);
  }
  return out;
}

/** The audio layer as a transport target (`null` = nothing on it). */
export function audioTransport(layer: MediaLayerState | null): MediaTransport | null {
  if (!layer) return null;
  const p = layer.playback;
  return {
    layer: "audio",
    asset: layer.asset,
    name: layer.name,
    state: p.state,
    position: p.position,
    duration: p.duration,
    loop: p.loop,
    rate: p.rate,
    hw: p.hw,
  };
}

/** Calls `fn` at most every `ms` while values keep coming, always with the latest one last. */
export function throttle<T>(fn: (v: T) => void, ms: number): ((v: T) => void) & { flush(): void } {
  let last = -Infinity;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let pending: { v: T } | null = null;
  const fire = () => {
    timer = null;
    if (!pending) return;
    const { v } = pending;
    pending = null;
    last = Date.now();
    fn(v);
  };
  const call = (v: T) => {
    pending = { v };
    const wait = last + ms - Date.now();
    if (wait <= 0) {
      if (timer) clearTimeout(timer);
      fire();
    } else if (!timer) timer = setTimeout(fire, wait);
  };
  call.flush = () => {
    if (timer) clearTimeout(timer);
    fire();
  };
  return call;
}

// ---- settings ------------------------------------------------------------------------------------

export interface DeviceOption {
  /** Stored name (`""` = no local audio). */
  value: string;
  label: string;
  disabled: boolean;
  title: string;
}

/** The device picker's options: none, every device the server found, and the configured one
 * when the server no longer lists it. */
export function deviceOptions(devices: AudioDeviceInfo[], current: string): DeviceOption[] {
  const out: DeviceOption[] = [{ value: "", label: "None (no local audio)", disabled: false, title: "Play nowhere on this machine; audio embedded in IDN outputs still works" }];
  for (const d of devices) {
    const what = d.description && d.description !== d.name ? `${d.description} (${d.name})` : d.name;
    const ch = d.channels ? ` · ${d.channels} ch` : "";
    out.push({
      value: d.name,
      label: `${what}${ch}${d.available ? "" : " (unavailable)"}`,
      // The configured device stays selectable even when it is busy right now.
      disabled: !d.available && d.name !== current,
      title: d.problem ?? what,
    });
  }
  if (current && !devices.some((d) => d.name === current)) {
    out.push({ value: current, label: `${current} (not found)`, disabled: false, title: "The server does not list this device right now" });
  }
  return out;
}
