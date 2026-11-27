// Typed REST client for recording and streaming (crates/protocol/README.md, "Recording and
// streaming"; docs/RECORD-STREAM-PLAN.md): the box's settings and stream targets (keys are
// write-only), the live status, and starting / stopping the recording and each target.

import type { RecordingSettings, StartGateway, StreamTarget, StreamingSettings, StreamingStatus } from "$protocol";
import type { RestClient } from "./rest";

const enc = encodeURIComponent;

export class StreamingClient {
  constructor(readonly rest: RestClient) {}

  settings = () => this.rest.get<StreamingSettings>("/streaming");
  status = () => this.rest.get<StreamingStatus>("/streaming/status");
  setRecording = (r: RecordingSettings) => this.rest.request<StreamingSettings>("PUT", "/streaming/recording", r);
  /** A new target (`key` sent once; the server answers `hasKey`). */
  addTarget = (t: StreamTarget) => this.rest.request<StreamTarget>("POST", "/streaming/targets", t);
  /** `key` left out keeps the saved key; `""` clears it. */
  updateTarget = (t: StreamTarget) => this.rest.request<StreamTarget>("PUT", `/streaming/targets/${enc(t.id)}`, t);
  deleteTarget = (id: string) => this.rest.request<void>("DELETE", `/streaming/targets/${enc(id)}`);
  startStream = (id: string, body: StartGateway = { screen: null }) =>
    this.rest.request<StreamingStatus>("POST", `/streaming/targets/${enc(id)}/start`, body);
  stopStream = (id: string) => this.rest.request<StreamingStatus>("POST", `/streaming/targets/${enc(id)}/stop`);
  startRecording = (body: StartGateway = { screen: null }) => this.rest.request<StreamingStatus>("POST", "/recording/start", body);
  stopRecording = () => this.rest.request<StreamingStatus>("POST", "/recording/stop");
  splitRecording = () => this.rest.request<StreamingStatus>("POST", "/recording/split");
}

/**
 * An SRT URL with a value the server hid (`***`: its passphrase or stream ID). The server sends
 * SRT URLs this way and keeps the saved one only when the shown URL comes back unchanged, so an
 * edited URL must be typed again in full.
 */
export function hasHiddenValue(url: string): boolean {
  const q = url.split("?").slice(1).join("?");
  return !!q && q.split("&").some((kv) => kv.includes("=") && kv.slice(kv.indexOf("=") + 1) === "***");
}

/**
 * The server a stream URL names: its scheme, host and port (lowercase), without user info, path
 * or query — the server's rule for keeping a saved stream key (another server needs the key
 * again).
 */
export function serverOf(url: string): string {
  const at = url.indexOf("://");
  const scheme = at >= 0 ? url.slice(0, at) : "";
  const rest = at >= 0 ? url.slice(at + 3) : url;
  const authority = rest.split(/[/?#]/)[0] ?? "";
  const host = authority.slice(authority.lastIndexOf("@") + 1);
  return `${scheme.toLowerCase()}://${host.toLowerCase()}`;
}

/** A target's server as the server compares them when keeping a key: its kind and `serverOf`
 *  its URL (an SRT target switched to RTMP is another server, R1-2). */
export function serverKey(kind: string, url: string): string {
  return `${kind}|${serverOf(url)}`;
}

/** `seconds` as `H:MM:SS` (or `M:SS` under an hour). */
export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** Bytes for people: `812 MB`, `1.4 GB`. */
export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} kB`;
  return `${n} B`;
}

/** The Show page indicator's look: what runs and whether anything is in trouble. */
export function indicator(st: StreamingStatus | null): { recording: boolean; streaming: number; trouble: boolean; seconds: number } {
  if (!st) return { recording: false, streaming: 0, trouble: false, seconds: 0 };
  const bad = (s: string) => s === "retrying" || s === "failed";
  const secs = [st.recording?.seconds ?? 0, ...st.streams.map((s) => s.seconds)];
  return {
    recording: !!st.recording,
    streaming: st.streams.length,
    trouble: (!!st.recording && bad(st.recording.state)) || st.streams.some((s) => bad(s.state)),
    seconds: Math.max(0, ...secs),
  };
}
