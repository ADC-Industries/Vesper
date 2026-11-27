// Recording and streaming (docs/RECORD-STREAM-PLAN.md): the server's status, polled every 2 s
// while a view watches it (the Show page's indicator, Settings ▸ Recording & streaming), the
// box's settings and targets, and the actions. Only on servers with the `streaming`
// capability. Failures become toasts. What it holds belongs to the connection it came from:
// after Oriel connects elsewhere it reads as nothing until loaded again (F10-2).

import type { RecordingSettings, StreamTarget, StreamingSettings, StreamingStatus } from "$protocol";
import { StreamingClient } from "../server/streaming";
import { errorText } from "./errors";
import { session } from "./session.svelte";

const POLL_MS = 2000;

type Conn = typeof session.conn;

class StreamingState {
  private st = $state.raw<{ conn: Conn; v: StreamingStatus | null }>({ conn: null, v: null });
  private cfg = $state.raw<{ conn: Conn; v: StreamingSettings | null }>({ conn: null, v: null });
  busy = $state(false);
  private watchers = 0;
  private timer: ReturnType<typeof setInterval> | null = null;

  /** The status, for the current connection only. */
  get status(): StreamingStatus | null {
    return this.st.conn === session.conn ? this.st.v : null;
  }
  set status(v: StreamingStatus | null) {
    this.st = { conn: session.conn, v };
  }
  /** The settings and targets, for the current connection only. */
  get settings(): StreamingSettings | null {
    return this.cfg.conn === session.conn ? this.cfg.v : null;
  }
  set settings(v: StreamingSettings | null) {
    this.cfg = { conn: session.conn, v };
  }

  /** The server records and streams. */
  get available(): boolean {
    return !!session.server?.capabilities.includes("streaming");
  }

  private client(): StreamingClient | null {
    const conn = session.conn;
    return conn && this.available ? new StreamingClient(conn.rest) : null;
  }

  /** Polls the status while the returned function hasn't been called. */
  watch(): () => void {
    this.watchers++;
    if (this.watchers === 1) {
      void this.refresh();
      this.timer = setInterval(() => void this.refresh(), POLL_MS);
    }
    let done = false;
    return () => {
      if (done) return;
      done = true;
      this.watchers--;
      if (this.watchers === 0 && this.timer) {
        clearInterval(this.timer);
        this.timer = null;
      }
    };
  }

  async refresh(): Promise<void> {
    const conn = session.conn;
    const c = this.client();
    if (!c) {
      this.status = null;
      return;
    }
    try {
      const v = await c.status();
      if (session.conn === conn) this.status = v;
    } catch {
      // Quietly: the connection banner says when the server is gone.
    }
  }

  async loadSettings(): Promise<void> {
    const conn = session.conn;
    const c = this.client();
    if (!c) return;
    try {
      const v = await c.settings();
      if (session.conn === conn) this.settings = v;
    } catch (e) {
      if (session.conn === conn) conn?.pushNotice("error", errorText(e));
    }
  }

  /** Runs an action; a status it answers replaces the current one. */
  private async act(f: (c: StreamingClient) => Promise<StreamingStatus | unknown>, reloadSettings = false): Promise<boolean> {
    const conn = session.conn;
    const c = this.client();
    if (!c || this.busy) return false;
    this.busy = true;
    try {
      const r = await f(c);
      if (session.conn !== conn) return false;
      if (r && typeof r === "object" && "streams" in r) this.status = r as StreamingStatus;
      else await this.refresh();
      if (reloadSettings) await this.loadSettings();
      return true;
    } catch (e) {
      if (session.conn === conn) conn?.pushNotice("error", errorText(e));
      return false;
    } finally {
      this.busy = false;
    }
  }

  startRecording = () => this.act((c) => c.startRecording());
  stopRecording = () => this.act((c) => c.stopRecording());
  splitRecording = () => this.act((c) => c.splitRecording());
  startStream = (id: string) => this.act((c) => c.startStream(id));
  stopStream = (id: string) => this.act((c) => c.stopStream(id));
  saveRecording = (r: RecordingSettings) =>
    this.act(async (c) => {
      const conn = session.conn;
      const v = await c.setRecording(r);
      if (session.conn === conn) this.settings = v;
    });
  saveTarget = (t: StreamTarget) => this.act((c) => (t.id ? c.updateTarget(t) : c.addTarget(t)), true);
  deleteTarget = (id: string) => this.act((c) => c.deleteTarget(id), true);
}

export const streaming = new StreamingState();
