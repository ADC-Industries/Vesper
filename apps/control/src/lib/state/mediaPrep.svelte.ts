// Media preparation (docs/MEDIA-PREP-PLAN.md): the Codex box's jobs and settings, polled every
// 2 s while a view watches them (the Library, Settings ▸ Media), and the actions on a media file
// (prepare, trim, use the original). On a show box the status answers `409 codexElsewhere`:
// `elsewhere` names the Codex box, and the jobs stay empty (the file's own `prep` still shows).
// What it holds belongs to the connection it came from.

import type { MediaPrepJob, MediaPrepSettings, MediaPrepStatus, SetMediaTrim } from "$protocol";
import { ApiRequestError } from "../server/rest";
import { MediaPrepClient } from "../server/mediaPrep";
import { errorText } from "./errors";
import { session } from "./session.svelte";
import { libraryPage } from "./library.svelte";
import { sitePage, siteState } from "./site.svelte";

const POLL_MS = 2000;

type Conn = typeof session.conn;

class MediaPrepState {
  private st = $state.raw<{ conn: Conn; v: MediaPrepStatus | null; elsewhere: string | null }>({ conn: null, v: null, elsewhere: null });
  busy = $state(false);
  private watchers = 0;
  private timer: ReturnType<typeof setInterval> | null = null;

  /** The status, for the current connection only. */
  get status(): MediaPrepStatus | null {
    return this.st.conn === session.conn ? this.st.v : null;
  }
  /** On a show box: the box preparation runs on. */
  get elsewhere(): string | null {
    return this.st.conn === session.conn ? this.st.elsewhere : null;
  }

  /** The job of an asset, if one is listed. */
  jobFor(asset: string): MediaPrepJob | null {
    return this.status?.jobs.find((j) => j.asset === asset) ?? null;
  }

  private client(): MediaPrepClient | null {
    const conn = session.conn;
    return conn && siteState.has("codex") ? new MediaPrepClient(conn.rest) : null;
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
    if (!c) return;
    const open = (this.status?.jobs ?? []).filter((j) => j.step !== "done" && j.step !== "failed").length;
    try {
      const v = await c.status();
      if (session.conn !== conn) return;
      this.st = { conn, v, elsewhere: null };
      // A job finished: the media list shows what it did.
      const still = v.jobs.filter((j) => j.step !== "done" && j.step !== "failed").length;
      if (still < open) void libraryPage.loadMedia();
    } catch (e) {
      if (session.conn !== conn) return;
      if (e instanceof ApiRequestError && e.code === "codexElsewhere") {
        this.st = { conn, v: null, elsewhere: e.codex?.name ?? "the site's Codex box" };
      }
    }
  }

  private async act(f: (c: MediaPrepClient) => Promise<unknown>): Promise<boolean> {
    const conn = session.conn;
    const c = this.client();
    if (!c || this.busy) return false;
    this.busy = true;
    try {
      await f(c);
      if (session.conn !== conn) return false;
      await Promise.all([this.refresh(), libraryPage.loadMedia()]);
      return true;
    } catch (e) {
      if (session.conn === conn) conn?.pushNotice("error", errorText(e));
      return false;
    } finally {
      this.busy = false;
    }
  }

  private get collection(): string | null {
    return libraryPage.loadedCollection ?? siteState.codex?.collection ?? null;
  }

  prepare = (asset: string, again = false) => {
    const c = this.collection;
    return c ? this.act((x) => x.prepare(c, asset, again)) : Promise.resolve(false);
  };
  trim = (asset: string, t: SetMediaTrim) => {
    const c = this.collection;
    return c ? this.act((x) => x.trim(c, asset, t)) : Promise.resolve(false);
  };
  useOriginal = (asset: string) => {
    const c = this.collection;
    return c ? this.act((x) => x.useOriginal(c, asset)) : Promise.resolve(false);
  };
  /** Saves the settings (admins only: on a locked site this asks for the PIN or a sign-in). */
  saveSettings = async (s: MediaPrepSettings): Promise<boolean> => {
    const conn = session.conn;
    const c = this.client();
    if (!c || this.busy) return false;
    this.busy = true;
    try {
      const v = await sitePage.admin("Saving the media settings", () => c.setSettings(s));
      if (v === undefined || session.conn !== conn) return false;
      const cur = this.status;
      if (cur) this.st = { conn, v: { ...cur, settings: v }, elsewhere: null };
      await this.refresh();
      return true;
    } finally {
      this.busy = false;
    }
  };
}

export const mediaPrep = new MediaPrepState();
