// The control WebSocket: auto-reconnect with exponential backoff, command/result matching,
// state resync on every (re)connect, and preview subscriptions that survive reconnects.

import type { ApiError, ClientMessage, Command, CommandResult, ServerHello, ServerMessage, TimerTick } from "$protocol";
import { PROTOCOL_VERSION } from "$protocol";
import type { ShowStore } from "./state";

/** The subset of the browser WebSocket this class uses (so tests can fake it). */
export interface SocketLike {
  binaryType: string;
  readyState: number;
  onopen: ((ev: unknown) => void) | null;
  onclose: ((ev: { code: number; reason: string }) => void) | null;
  onerror: ((ev: unknown) => void) | null;
  onmessage: ((ev: { data: unknown }) => void) | null;
  send(data: string): void;
  close(code?: number, reason?: string): void;
}

export type SocketPhase =
  /** Not started, or stopped by the app. */
  | "idle"
  /** First connection attempt in progress. */
  | "connecting"
  /** Connected and synced (the first state arrived). */
  | "online"
  /** Lost the connection; retrying with backoff. */
  | "reconnecting"
  /** The server speaks another protocol major; retrying would not help. */
  | "incompatible"
  /** The server closed the socket with 4003: this client's key was revoked (removed from the
   *  site). Retrying would not help; the client has to pair again. */
  | "revoked";

export interface SocketStatus {
  phase: SocketPhase;
  /** Consecutive failed attempts since the last good connection. */
  attempt: number;
  /** When the next attempt starts (ms since epoch), while reconnecting. */
  nextRetryAt: number | null;
  /** The server's hello from the current (or last) connection. */
  server: ServerHello | null;
  /** Why the last connection ended, for the UI. */
  lastError: string | null;
}

export interface Backoff {
  initialMs: number;
  maxMs: number;
  factor: number;
  /** 0..1, share of the delay randomised to avoid every client retrying at once. */
  jitter: number;
}

export const DEFAULT_BACKOFF: Backoff = { initialMs: 500, maxMs: 8000, factor: 2, jitter: 0.2 };

/** Delay before attempt `n` (1-based) of a reconnect. */
export function backoffDelay(n: number, b: Backoff = DEFAULT_BACKOFF, rand: () => number = Math.random): number {
  const base = Math.min(b.maxMs, b.initialMs * Math.pow(b.factor, Math.max(0, n - 1)));
  const j = base * b.jitter;
  return Math.round(base - j + rand() * 2 * j);
}

/** A failed command: the server's error, or a local one (`disconnected`, `timeout`). */
export class CommandError extends Error {
  constructor(
    public readonly code: ApiError["code"] | "disconnected" | "timeout",
    message: string,
  ) {
    super(message);
    this.name = "CommandError";
  }
}

export interface SocketEvents {
  onStatus?(s: SocketStatus): void;
  onNotice?(level: "info" | "warning" | "error", message: string, key: string | null): void;
  onDocumentChanged?(msg: Extract<ServerMessage, { type: "documentChanged" }>): void;
  /** `codexChanged`: parts of the Codex to refetch. */
  onCodexChanged?(msg: Extract<ServerMessage, { type: "codexChanged" }>): void;
  /** `siteChanged`: parts of the site's state to refetch. */
  onSiteChanged?(msg: Extract<ServerMessage, { type: "siteChanged" }>): void;
  /** `siteEvent`: one of the site's typed events, as it happens. */
  onSiteEvent?(msg: Extract<ServerMessage, { type: "siteEvent" }>): void;
  onBinary?(data: ArrayBuffer): void;
  /** Live master meters (dBFS per channel), at most 10 a second. */
  onAudioLevels?(levels: number[], xruns: number): void;
  /** Live timer values (`timerTick`, at most once a second while timers run). */
  onTimerTick?(timers: TimerTick[]): void;
  /** `presenceChanged`: who has what open, and the edit locks (the whole list). */
  onPresenceChanged?(msg: Extract<ServerMessage, { type: "presenceChanged" }>): void;
  /** `sourcesChanged`: the live network sources (the whole list). */
  onSourcesChanged?(msg: Extract<ServerMessage, { type: "sourcesChanged" }>): void;
  /** Called after every (re)connect once the first state arrived. */
  onSynced?(reconnect: boolean): void;
}

export interface SocketOptions {
  url: () => string;
  store: ShowStore;
  clientName: string;
  events?: SocketEvents;
  backoff?: Backoff;
  /** ms before an unanswered command fails. */
  commandTimeoutMs?: number;
  /** Fails the connection if nothing arrives for this long while connecting (ms). */
  connectTimeoutMs?: number;
  create?: (url: string) => SocketLike;
  timers?: {
    setTimeout: (fn: () => void, ms: number) => unknown;
    clearTimeout: (h: unknown) => void;
  };
  now?: () => number;
}

interface PendingPing {
  resolve: (ok: boolean) => void;
  timer: unknown;
}

interface Pending {
  resolve: (r: CommandResult) => void;
  reject: (e: CommandError) => void;
  timer: unknown;
}

const OPEN = 1;

export class ShowSocket {
  private ws: SocketLike | null = null;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private pings = new Map<number, PendingPing>();
  private retryTimer: unknown = null;
  private connectTimer: unknown = null;
  private stopped = true;
  private everSynced = false;
  private synced = false;
  private previews = new Map<string, { fps: number; width: number }>();
  /** The last presence message: sent again after every (re)connect. */
  private presence: string | null = null;
  private status: SocketStatus = { phase: "idle", attempt: 0, nextRetryAt: null, server: null, lastError: null };
  private o: Required<Omit<SocketOptions, "events">> & { events: SocketEvents };

  constructor(opts: SocketOptions) {
    // `??` (not object spread) so an explicit `undefined` option keeps the default.
    this.o = {
      url: opts.url,
      store: opts.store,
      clientName: opts.clientName,
      events: opts.events ?? {},
      backoff: opts.backoff ?? DEFAULT_BACKOFF,
      commandTimeoutMs: opts.commandTimeoutMs ?? 5000,
      connectTimeoutMs: opts.connectTimeoutMs ?? 6000,
      create: opts.create ?? ((url) => new WebSocket(url) as unknown as SocketLike),
      timers: opts.timers ?? {
        setTimeout: (fn, ms) => setTimeout(fn, ms),
        clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
      },
      now: opts.now ?? (() => Date.now()),
    };
  }

  get current(): SocketStatus {
    return this.status;
  }

  get isOnline(): boolean {
    return this.status.phase === "online";
  }

  start(): void {
    if (!this.stopped) return;
    this.stopped = false;
    this.setStatus({ phase: this.everSynced ? "reconnecting" : "connecting", nextRetryAt: null });
    this.open();
  }

  stop(): void {
    this.stopped = true;
    this.clearTimers();
    const ws = this.ws;
    this.ws = null;
    if (ws) {
      this.detach(ws);
      try {
        ws.close(1000, "client stopped");
      } catch {
        /* already closed */
      }
    }
    this.failPending("disconnected", "Disconnected");
    this.setStatus({ phase: "idle", nextRetryAt: null });
  }

  /** Drops the current connection and reconnects (e.g. a heartbeat noticed it is dead). */
  kick(reason: string): void {
    if (this.stopped || !this.ws) return;
    this.handleClose(this.ws, reason);
  }

  /** Reconnects now instead of waiting for the backoff timer. */
  retryNow(): void {
    if (this.stopped || this.ws) return;
    this.clearTimers();
    this.open();
  }

  /**
   * Sends a command and resolves with its result once the server applied it (the state
   * that includes it has already been applied to the store by then). Rejects with a
   * CommandError when the server refuses it, the socket is down, or no answer comes.
   * Commands are never queued while offline: a slide change must not fire late.
   */
  command(command: Command): Promise<CommandResult> {
    const ws = this.ws;
    if (!ws || ws.readyState !== OPEN || !this.synced) {
      return Promise.reject(new CommandError("disconnected", "Not connected to the server"));
    }
    const id = this.nextId++;
    return new Promise<CommandResult>((resolve, reject) => {
      const timer = this.o.timers.setTimeout(() => {
        this.pending.delete(id);
        reject(new CommandError("timeout", "The server did not answer"));
      }, this.o.commandTimeoutMs);
      this.pending.set(id, {
        resolve: (r) => {
          if (r.ok) resolve(r);
          else reject(new CommandError(r.error?.code ?? "unknown", r.error?.message ?? "Command failed"));
        },
        reject,
        timer,
      });
      ws.send(JSON.stringify({ type: "command", id, command }));
    });
  }

  /**
   * Application-level heartbeat: sends `ping` and resolves `true` when the matching `pong`
   * arrives within `timeoutMs`, `false` otherwise (or when not connected). Browsers cannot
   * see WebSocket ping frames, so this is how a dead link is noticed.
   */
  ping(timeoutMs = 3000): Promise<boolean> {
    const ws = this.ws;
    if (!ws || ws.readyState !== OPEN || !this.synced) return Promise.resolve(false);
    const id = this.nextId++;
    return new Promise<boolean>((resolve) => {
      const timer = this.o.timers.setTimeout(() => {
        this.pings.delete(id);
        resolve(false);
      }, timeoutMs);
      this.pings.set(id, { resolve, timer });
      ws.send(JSON.stringify({ type: "ping", id }));
    });
  }

  /** Subscribes to a screen's preview; kept across reconnects until unsubscribed. */
  subscribePreview(screen: string, fps: number, width: number): void {
    this.previews.set(screen, { fps, width });
    this.sendRaw({ type: "command", id: this.nextId++, command: { type: "subscribePreview", screen, fps, width } });
  }

  /** What this client has open (`presence`): sent now when online, and after every reconnect. */
  setPresence(msg: Extract<ClientMessage, { type: "presence" }>): void {
    const text = JSON.stringify(msg);
    if (text === this.presence) return;
    this.presence = text;
    if (this.ws && this.ws.readyState === OPEN && this.synced) this.ws.send(text);
  }

  unsubscribePreview(screen: string | null): void {
    if (screen === null) this.previews.clear();
    else this.previews.delete(screen);
    this.sendRaw({ type: "command", id: this.nextId++, command: { type: "unsubscribePreview", screen } });
  }

  // ---------------------------------------------------------------------------------

  private sendRaw(msg: unknown): void {
    if (this.ws && this.ws.readyState === OPEN && this.synced) this.ws.send(JSON.stringify(msg));
  }

  private setStatus(p: Partial<SocketStatus>): void {
    this.status = { ...this.status, ...p };
    this.o.events.onStatus?.(this.status);
  }

  private clearTimers(): void {
    if (this.retryTimer !== null) this.o.timers.clearTimeout(this.retryTimer);
    if (this.connectTimer !== null) this.o.timers.clearTimeout(this.connectTimer);
    this.retryTimer = null;
    this.connectTimer = null;
  }

  private open(): void {
    this.synced = false;
    let ws: SocketLike;
    try {
      ws = this.o.create(this.o.url());
    } catch (e) {
      this.scheduleRetry(String(e));
      return;
    }
    this.ws = ws;
    ws.binaryType = "arraybuffer";
    this.connectTimer = this.o.timers.setTimeout(() => {
      this.connectTimer = null;
      if (this.ws === ws && !this.synced) this.handleClose(ws, "Connection timed out");
    }, this.o.connectTimeoutMs);
    ws.onopen = () => {
      // The URL carries protocolVersion; the hello also names this client in server logs.
      ws.send(JSON.stringify({ type: "hello", protocolVersion: PROTOCOL_VERSION, clientName: this.o.clientName }));
    };
    ws.onmessage = (ev) => this.handleMessage(ws, ev.data);
    ws.onerror = () => {
      /* onclose follows */
    };
    ws.onclose = (ev) => {
      if (ev.code === 4001 || ev.code === 4003) {
        this.detach(ws);
        this.ws = null;
        this.failPending("disconnected", "Disconnected");
        this.clearTimers();
        this.stopped = true;
        if (ev.code === 4003) this.setStatus({ phase: "revoked", lastError: ev.reason || "This client's key was revoked", nextRetryAt: null });
        else this.setStatus({ phase: "incompatible", lastError: ev.reason || "Incompatible protocol version", nextRetryAt: null });
        return;
      }
      this.handleClose(ws, ev.reason || (this.synced ? "Connection lost" : "Could not connect"));
    };
  }

  private detach(ws: SocketLike): void {
    ws.onopen = ws.onclose = ws.onerror = ws.onmessage = null;
  }

  private handleClose(ws: SocketLike, reason: string): void {
    if (this.ws !== ws) return;
    this.detach(ws);
    try {
      ws.close();
    } catch {
      /* ignore */
    }
    this.ws = null;
    this.synced = false;
    if (this.connectTimer !== null) {
      this.o.timers.clearTimeout(this.connectTimer);
      this.connectTimer = null;
    }
    this.failPending("disconnected", "Connection lost before the server answered");
    if (this.stopped) return;
    this.scheduleRetry(reason);
  }

  private scheduleRetry(reason: string): void {
    const attempt = this.status.attempt + 1;
    const delay = backoffDelay(attempt, this.o.backoff);
    this.setStatus({
      phase: this.everSynced ? "reconnecting" : "connecting",
      attempt,
      nextRetryAt: this.o.now() + delay,
      lastError: reason,
    });
    this.retryTimer = this.o.timers.setTimeout(() => {
      this.retryTimer = null;
      if (!this.stopped) this.open();
    }, delay);
  }

  private failPending(code: "disconnected", message: string): void {
    for (const [, p] of this.pending) {
      this.o.timers.clearTimeout(p.timer);
      p.reject(new CommandError(code, message));
    }
    this.pending.clear();
    for (const [, p] of this.pings) {
      this.o.timers.clearTimeout(p.timer);
      p.resolve(false);
    }
    this.pings.clear();
  }

  private handleMessage(ws: SocketLike, data: unknown): void {
    if (this.ws !== ws) return;
    if (typeof data !== "string") {
      if (data instanceof ArrayBuffer) this.o.events.onBinary?.(data);
      return;
    }
    let msg: ServerMessage;
    try {
      msg = JSON.parse(data) as ServerMessage;
    } catch {
      return;
    }
    switch (msg.type) {
      case "hello":
        this.setStatus({ server: msg.server });
        // Every connection starts with a full state: accept it whatever its revision.
        this.o.store.resync();
        break;
      case "state": {
        this.o.store.apply(msg.revision, msg.state);
        if (!this.synced) {
          this.synced = true;
          const reconnect = this.everSynced;
          this.everSynced = true;
          if (this.connectTimer !== null) {
            this.o.timers.clearTimeout(this.connectTimer);
            this.connectTimer = null;
          }
          this.setStatus({ phase: "online", attempt: 0, nextRetryAt: null, lastError: null });
          for (const [screen, p] of this.previews) {
            ws.send(JSON.stringify({ type: "command", id: this.nextId++, command: { type: "subscribePreview", screen, ...p } }));
          }
          if (this.presence) ws.send(this.presence);
          this.o.events.onSynced?.(reconnect);
        }
        break;
      }
      case "result": {
        const p = this.pending.get(msg.id);
        if (p) {
          this.pending.delete(msg.id);
          this.o.timers.clearTimeout(p.timer);
          p.resolve(msg);
        }
        break;
      }
      case "pong": {
        const p = this.pings.get(msg.id);
        if (p) {
          this.pings.delete(msg.id);
          this.o.timers.clearTimeout(p.timer);
          p.resolve(true);
        }
        break;
      }
      case "notice":
        this.o.events.onNotice?.(msg.level, msg.message, msg.key ?? null);
        break;
      case "documentChanged":
        this.o.events.onDocumentChanged?.(msg);
        break;
      case "codexChanged":
        this.o.events.onCodexChanged?.(msg);
        break;
      case "siteChanged":
        this.o.events.onSiteChanged?.(msg);
        break;
      case "siteEvent":
        this.o.events.onSiteEvent?.(msg);
        break;
      case "audioLevels":
        this.o.events.onAudioLevels?.(msg.levels, msg.xruns);
        break;
      case "timerTick":
        this.o.events.onTimerTick?.(msg.timers);
        break;
      case "presenceChanged":
        this.o.events.onPresenceChanged?.(msg);
        break;
      case "sourcesChanged":
        this.o.events.onSourcesChanged?.(msg);
        break;
      case "error":
        this.setStatus({ lastError: msg.error.message });
        break;
      default:
        // Unknown message types are ignored (compatible protocol additions).
        break;
    }
  }
}
