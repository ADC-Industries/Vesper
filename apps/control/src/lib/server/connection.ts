// One connection to one server: REST client, WebSocket, show-state store, previews, notices.
// Plain TypeScript (Svelte stores, no runes) so it can be unit tested without a browser.

import type { Command, CommandResult, DocumentKind, LiveSource, NoticeLevel, PresenceList, ServerMessage, TimerState, TimerTick } from "$protocol";
import { PROTOCOL_VERSION } from "$protocol";
import { writable, type Readable } from "svelte/store";
import { CLIENT_NAME } from "../config";
import { PreviewDecoder, type PreviewImage } from "./preview";
import { RestClient } from "./rest";
import { ShowSocket, type SocketLike, type SocketStatus } from "./socket";
import { createShowStore, type ShowStore } from "./state";

/** The server's notice key for change events it dropped for this client (`ws.rs`). */
const LAGGED = "lagged";

export interface Notice {
  id: number;
  level: NoticeLevel;
  message: string;
  /** Rolling notices with the same key replace each other instead of stacking. */
  key: string | null;
  /** How many identical notices were folded into this one. */
  count: number;
  at: number;
}

export interface ConnectionOptions {
  baseUrl: string;
  token: string | null;
  createSocket?: (url: string) => SocketLike;
  fetch?: typeof fetch;
  /** Heartbeat period while the socket is quiet (ms); 0 disables it. */
  heartbeatMs?: number;
  /** How long a heartbeat ping may go unanswered before the link counts as dead (ms). */
  pingTimeoutMs?: number;
}

export type DocumentChangedMessage = Extract<ServerMessage, { type: "documentChanged" }>;
export type CodexChangedMessage = Extract<ServerMessage, { type: "codexChanged" }>;
export type SiteChangedMessage = Extract<ServerMessage, { type: "siteChanged" }>;
export type SiteEventMessage = Extract<ServerMessage, { type: "siteEvent" }>;
/** `msg` carries the revision, the sender's client ID and the applied ops of content edits. */
export type DocumentListener = (kind: DocumentKind, id: string, msg: DocumentChangedMessage) => void;

/** Master bus meters: the latest `audioLevels` message or state, whichever came last. */
export interface AudioLevels {
  /** Peak per channel, dBFS (−60 = silence). */
  levels: number[];
  /** Output buffer underruns since the server started. */
  xruns: number;
}

/** A timer's live value: from the latest state or `timerTick`, whichever came last. */
export type TimerValue = TimerTick;
export type TimerValues = Record<string, TimerValue>;

/** Timer values from a state (every timer). */
export function timerValuesFromState(timers: TimerState[]): TimerValues {
  const out: TimerValues = {};
  for (const t of timers) out[t.id] = { id: t.id, state: t.state, currentSeconds: t.currentSeconds, display: t.display };
  return out;
}

/** Applies a `timerTick` (only the timers it lists change). */
export function applyTimerTick(cur: TimerValues, ticks: TimerTick[]): TimerValues {
  const out = { ...cur };
  for (const t of ticks) out[t.id] = t;
  return out;
}

export class Connection {
  readonly rest: RestClient;
  readonly socket: ShowSocket;
  readonly show: ShowStore;
  readonly previews: PreviewDecoder;
  readonly status: Readable<SocketStatus>;
  readonly notices: Readable<Notice[]>;
  /** Live timer values by timer ID (state values, then `timerTick` updates). */
  readonly timerValues: Readable<TimerValues>;

  private statusW = writable<SocketStatus>({ phase: "idle", attempt: 0, nextRetryAt: null, server: null, lastError: null });
  private noticesW = writable<Notice[]>([]);
  private timerValuesW = writable<TimerValues>({});
  private stateTimers: TimerState[] | undefined = undefined;
  private noticeSeq = 1;
  private seenServerNotices = new Map<string, number>();
  private docListeners = new Set<DocumentListener>();
  private codexListeners = new Set<(msg: CodexChangedMessage) => void>();
  private siteListeners = new Set<(msg: SiteChangedMessage) => void>();
  private siteEventListeners = new Set<(msg: SiteEventMessage) => void>();
  private resyncListeners = new Set<(reconnect: boolean) => void>();
  private presenceListeners = new Set<(p: PresenceList) => void>();
  private sourcesListeners = new Set<(s: LiveSource[]) => void>();
  private levelListeners = new Set<(l: AudioLevels) => void>();
  private stateLevels: number[] | undefined = undefined;
  /** The latest master meters (read it in an animation frame; see `onAudioLevels`). */
  audioLevels: AudioLevels = { levels: [], xruns: 0 };
  private lastMessageAt = Date.now();
  private heartbeat: ReturnType<typeof setInterval> | null = null;
  private heartbeatMs: number;
  private pingTimeoutMs: number;
  private beating = false;

  constructor(opts: ConnectionOptions) {
    const token = opts.token;
    this.rest = new RestClient({ baseUrl: opts.baseUrl, token: () => token, fetch: opts.fetch });
    this.show = createShowStore();
    this.previews = new PreviewDecoder();
    this.status = { subscribe: this.statusW.subscribe };
    this.notices = { subscribe: this.noticesW.subscribe };
    this.timerValues = { subscribe: this.timerValuesW.subscribe };
    this.heartbeatMs = opts.heartbeatMs ?? 4000;
    this.pingTimeoutMs = opts.pingTimeoutMs ?? 3000;
    this.socket = new ShowSocket({
      url: () => this.rest.wsUrl(PROTOCOL_VERSION),
      store: this.show,
      clientName: CLIENT_NAME,
      create: opts.createSocket,
      events: {
        onStatus: (s) => {
          this.statusW.set(s);
          if (s.phase === "reconnecting") this.previews.resetCounters();
        },
        onNotice: (level, message, key) => {
          // The server dropped change events for this client (it was slow): what they named
          // may have changed, so everything is read again, as after a reconnect.
          if (key === LAGGED) {
            this.touch();
            this.pushNotice("info", "Oriel missed some changes from the server and read its lists again.", LAGGED);
            for (const l of this.resyncListeners) l(true);
            return;
          }
          this.serverNotice(level, message, key);
        },
        onDocumentChanged: (m) => {
          this.touch();
          for (const l of this.docListeners) l(m.kind, m.id, m);
        },
        onCodexChanged: (m) => {
          this.touch();
          for (const l of this.codexListeners) l(m);
        },
        onSiteChanged: (m) => {
          this.touch();
          for (const l of this.siteListeners) l(m);
        },
        onSiteEvent: (m) => {
          this.touch();
          for (const l of this.siteEventListeners) l(m);
        },
        onBinary: (data) => {
          this.touch();
          this.previews.push(data);
        },
        onAudioLevels: (levels, xruns) => {
          this.touch();
          this.setLevels({ levels, xruns });
        },
        onTimerTick: (timers) => {
          this.touch();
          this.timerValuesW.update((cur) => applyTimerTick(cur, timers));
        },
        onPresenceChanged: (m) => {
          this.touch();
          for (const l of this.presenceListeners) l(m.presence);
        },
        onSourcesChanged: (m) => {
          this.touch();
          for (const l of this.sourcesListeners) l(m.sources);
        },
        onSynced: (reconnect) => {
          this.touch();
          for (const l of this.resyncListeners) l(reconnect);
        },
      },
    });
    // Any state counts as a sign of life. A state also carries the meters as of its revision
    // (the fallback when no `audioLevels` messages come).
    this.show.subscribe((v) => {
      this.touch();
      const ts = v?.state.timers;
      if (ts && ts !== this.stateTimers) {
        this.stateTimers = ts;
        this.timerValuesW.set(timerValuesFromState(ts));
      }
      const a = v?.state.audio;
      if (!a || a.levels === this.stateLevels) return;
      const changed = !this.stateLevels || a.levels.join() !== this.stateLevels.join();
      this.stateLevels = a.levels;
      if (changed) this.setLevels({ levels: a.levels, xruns: a.xruns });
    });
  }

  start(): void {
    this.socket.start();
    if (this.heartbeatMs > 0 && typeof setInterval !== "undefined") {
      this.heartbeat = setInterval(() => void this.beat(), this.heartbeatMs);
    }
  }

  stop(): void {
    if (this.heartbeat) clearInterval(this.heartbeat);
    this.heartbeat = null;
    this.socket.stop();
    this.previews.clear();
  }

  /** Sends a command over the WebSocket. Resolves when applied, rejects on error. */
  command(c: Command): Promise<CommandResult> {
    return this.socket.command(c);
  }

  /** Fires for every `documentChanged` event (refetch that document). */
  onDocumentChanged(fn: DocumentListener): () => void {
    this.docListeners.add(fn);
    return () => this.docListeners.delete(fn);
  }

  /** Fires for every `codexChanged` event (refetch those parts of the Codex). */
  onCodexChanged(fn: (msg: CodexChangedMessage) => void): () => void {
    this.codexListeners.add(fn);
    return () => this.codexListeners.delete(fn);
  }

  /** Fires for every `siteChanged` event (refetch those parts of the site's state). */
  onSiteChanged(fn: (msg: SiteChangedMessage) => void): () => void {
    this.siteListeners.add(fn);
    return () => this.siteListeners.delete(fn);
  }

  /** Fires for every `siteEvent` (one of the site's typed events, as it happens). */
  onSiteEvent(fn: (msg: SiteEventMessage) => void): () => void {
    this.siteEventListeners.add(fn);
    return () => this.siteEventListeners.delete(fn);
  }

  /** Fires for every `presenceChanged` (who has what open, and the edit locks). */
  onPresenceChanged(fn: (p: PresenceList) => void): () => void {
    this.presenceListeners.add(fn);
    return () => this.presenceListeners.delete(fn);
  }

  /** Fires for every `sourcesChanged` (the live network sources, the whole list). */
  onSourcesChanged(fn: (s: LiveSource[]) => void): () => void {
    this.sourcesListeners.add(fn);
    return () => this.sourcesListeners.delete(fn);
  }

  /** Fires after every (re)connect once the state is in sync, and (`true`) after the server
   *  dropped change events for this client: read again what the events would have named. */
  onSynced(fn: (reconnect: boolean) => void): () => void {
    this.resyncListeners.add(fn);
    return () => this.resyncListeners.delete(fn);
  }

  /**
   * Fires for every meter update (up to 10 a second). Meant for components that draw meters
   * themselves, outside Svelte's reactivity, so the rest of the UI does not re-render.
   */
  onAudioLevels(fn: (l: AudioLevels) => void): () => void {
    this.levelListeners.add(fn);
    return () => this.levelListeners.delete(fn);
  }

  private setLevels(l: AudioLevels): void {
    this.audioLevels = l;
    for (const fn of this.levelListeners) fn(l);
  }

  onPreview(fn: (img: PreviewImage) => void): () => void {
    return this.previews.onFrame(fn);
  }

  /**
   * Shows a notice. A notice with a `key` replaces the one with the same key (it moves to the
   * end with a new id, so its toast shows again for its full time); one without a key folds
   * into an identical message as a count.
   */
  pushNotice(level: NoticeLevel, message: string, key: string | null = null): void {
    this.noticesW.update((list) => mergeNotice(list, { id: this.noticeSeq++, level, message, key, count: 1, at: Date.now() }));
  }

  /**
   * The server repeats its recent notices to every new connection. Show each one once per
   * half hour, so a reconnect doesn't bring back toasts the operator already dismissed.
   */
  private serverNotice(level: NoticeLevel, message: string, key: string | null): void {
    const k = `${key ?? ""}\u0000${level}\u0000${message}`;
    const now = Date.now();
    const seen = this.seenServerNotices.get(k);
    this.seenServerNotices.set(k, now);
    if (seen !== undefined && now - seen < 30 * 60_000) return;
    this.pushNotice(level, message, key);
  }

  dismissNotice(id: number): void {
    this.noticesW.update((list) => list.filter((n) => n.id !== id));
  }

  private touch(): void {
    this.lastMessageAt = Date.now();
  }

  /**
   * A dead network link can leave a WebSocket "open" for minutes. When the socket has been
   * quiet for a while, send an application-level `ping`; if no `pong` comes back in time, drop
   * the socket so the offline banner shows and reconnection starts.
   */
  async beat(): Promise<void> {
    if (!this.socket.isOnline || this.beating) return;
    if (Date.now() - this.lastMessageAt < this.heartbeatMs) return;
    this.beating = true;
    try {
      const ok = await this.socket.ping(this.pingTimeoutMs);
      if (ok) this.touch();
      else if (this.socket.isOnline) this.socket.kick("The server stopped answering");
    } finally {
      this.beating = false;
    }
  }
}

/** Adds a notice to the list (see `Connection.pushNotice`); keeps the newest six. */
export function mergeNotice(list: Notice[], n: Notice): Notice[] {
  if (n.key !== null) {
    return [...list.filter((o) => o.key !== n.key), n].slice(-6);
  }
  const same = list.find((o) => o.key === null && o.message === n.message && o.level === n.level);
  if (same) return list.map((o) => (o === same ? { ...o, count: o.count + 1, at: n.at } : o));
  return [...list, n].slice(-6);
}
