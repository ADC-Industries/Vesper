// Live network sources (G7, docs/LIVE-SOURCES-PLAN.md): the server's list of IDN sources
// (`GET /sources`, then `sourcesChanged`), read on first use (the bin's Video Inputs tab, or a
// source live on the video-input layer) and again after every reconnect. With them, the show's
// logical video inputs and this box's bindings (`GET /video-inputs`, refetched on
// `documentChanged` videoInput / inputBinding) and their changes.

import type { LiveSource, VideoInputEntry } from "$protocol";
import { session } from "./session.svelte";
import { errorText } from "./errors";
import { changes } from "./events";
import { bindingFor } from "../pages/show/sources";

class SourcesState {
  list = $state.raw<LiveSource[]>([]);
  loaded = $state(false);
  error = $state<string | null>(null);
  /** Logical video inputs ("Camera 1") with this box's binding, in order. */
  inputs = $state.raw<VideoInputEntry[]>([]);
  inputsLoaded = $state(false);
  private wanted = false;

  /** Follows the connection: the list from `sourcesChanged`, read again after a reconnect once
   *  it was wanted; the inputs again after a change of them. Returns the unsubscribe function. */
  attach(): () => void {
    const conn = session.conn;
    if (!conn) return () => {};
    this.list = [];
    this.loaded = false;
    this.error = null;
    this.inputs = [];
    this.inputsLoaded = false;
    const offs = [
      conn.onSourcesChanged((s) => {
        this.list = s;
        this.loaded = true;
      }),
      conn.onSynced(() => {
        if (this.wanted) void this.load();
      }),
      changes.on("documentChanged", (e) => {
        if (this.wanted && (e.kind === "videoInput" || e.kind === "inputBinding")) void this.loadInputs();
      }),
    ];
    return () => offs.forEach((o) => o());
  }

  /** Reads the lists once (the server starts looking for sources on the first request). */
  ensure(): void {
    if (this.wanted) return;
    this.wanted = true;
    void this.load();
  }

  async load(): Promise<void> {
    const conn = session.conn;
    if (!conn) return;
    void this.loadInputs();
    try {
      const r = await conn.rest.liveSources();
      if (session.conn !== conn) return;
      this.list = r.sources;
      this.loaded = true;
      this.error = null;
    } catch (e) {
      this.error = errorText(e);
    }
  }

  /** Reads the logical inputs (an older server without them: none). */
  async loadInputs(): Promise<void> {
    const conn = session.conn;
    if (!conn) return;
    try {
      const r = await conn.rest.videoInputs();
      if (session.conn !== conn) return;
      this.inputs = r;
    } catch {
      if (session.conn === conn) this.inputs = [];
    }
    this.inputsLoaded = true;
  }

  /** The entry of `name`, if listed or received. */
  find(name: string | null | undefined): LiveSource | null {
    return name ? (this.list.find((s) => s.name === name) ?? null) : null;
  }

  /** The input `id`, if known. */
  input(id: string | null | undefined): VideoInputEntry | null {
    return id ? (this.inputs.find((v) => v.id === id) ?? null) : null;
  }

  /** Adds an input (named by the server when `name` is empty), bound to `source` if given. */
  async createInput(name: string, source: string | null): Promise<VideoInputEntry | null> {
    return this.guard("Adding the input", async (conn) => {
      const e = await conn.rest.createVideoInput({ ...(name ? { name } : {}), ...(source ? { source: bindingFor(source) } : {}) });
      await this.loadInputs();
      return e;
    });
  }

  /** Renames and/or rebinds an input (`source`: a source name or address, null = unbind). */
  async updateInput(v: VideoInputEntry, change: { name?: string; source?: string | null }): Promise<void> {
    await this.guard("Changing the input", async (conn) => {
      const { revision, source, ...doc } = v;
      const next = "source" in change ? (change.source ? bindingFor(change.source) : null) : (source ?? null);
      await conn.rest.updateVideoInput({ ...doc, name: change.name ?? doc.name, source: next }, revision);
      await this.loadInputs();
    });
  }

  async deleteInput(v: VideoInputEntry): Promise<void> {
    await this.guard("Deleting the input", async (conn) => {
      await conn.rest.deleteVideoInput(v.id, v.revision);
      await this.loadInputs();
    });
  }

  private async guard<T>(what: string, fn: (conn: NonNullable<typeof session.conn>) => Promise<T>): Promise<T | null> {
    const conn = session.conn;
    if (!conn) return null;
    try {
      return await fn(conn);
    } catch (e) {
      conn.pushNotice("error", `${what} failed: ${errorText(e)}`);
      await this.loadInputs();
      return null;
    }
  }
}

export const sourcesState = new SourcesState();
