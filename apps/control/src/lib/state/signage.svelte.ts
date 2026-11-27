// Site ▸ Signage (docs/SIGNAGE-PLAN.md): the site's signage channels, playlists, feeds, settings
// and the override (`GET /signage`), read while the tab is shown: again on `siteChanged`
// (`signage`) and every few seconds (now/next, the thumbnails). Also the library's presentations
// and media for the playlist editor's pickers. Changes go through `sitePage.admin`, so a 403 asks
// for the PIN or a sign-in. The channels' sources also label Mosaic's source picker.

import type { MediaBinNode, PresentationSummary, PutSignageChannel, SignageFeed, SignageOverride, SignagePlaylist, SignageSettings, SignageState } from "$protocol";
import { changes } from "./events";
import { session } from "./session.svelte";
import { sitePage } from "./site.svelte";
import { errorText } from "./errors";
import { SignageClient } from "../server/signage";
import { setSignageSources } from "../pages/site/signage";

export const SIGNAGE_POLL_MS = 4000;

/** An image or video the playlist editor offers. */
export interface SignageMedia {
  asset: string;
  name: string;
  kind: "image" | "video";
}

class SignagePageState {
  state = $state.raw<SignageState | null>(null);
  presentations = $state.raw<PresentationSummary[]>([]);
  /** The media bins' images and videos (each asset once). */
  media = $state.raw<SignageMedia[]>([]);
  problem = $state<string | null>(null);
  loaded = $state(false);
  /** Bumped on every read: the thumbnails are fetched again. */
  tick = $state(0);
  private timer: ReturnType<typeof setInterval> | null = null;
  private off: (() => void) | null = null;

  client(): SignageClient | null {
    return session.conn ? new SignageClient(session.conn.rest) : null;
  }

  open() {
    this.close();
    void this.load();
    void this.loadLibrary();
    this.timer = setInterval(() => void this.load(), SIGNAGE_POLL_MS);
    this.off = changes.on("siteChanged", (e) => {
      if (e.parts.includes("signage")) void this.load();
    });
  }

  close() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
    this.off?.();
    this.off = null;
  }

  async load() {
    const c = this.client();
    if (!c) return;
    const conn = session.conn;
    try {
      const s = await c.state();
      if (session.conn !== conn) return;
      this.state = s;
      setSignageSources(s.channels);
      this.problem = null;
      this.tick++;
    } catch (e) {
      if (session.conn === conn) this.problem = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  /** The presentations and media the playlist editor offers (quietly empty when unreadable). */
  async loadLibrary() {
    const conn = session.conn;
    if (!conn) return;
    const p = await conn.rest.presentations().catch(() => null);
    if (session.conn !== conn) return;
    if (p) this.presentations = p;
    const bins: string[] = [];
    const walk = (n: MediaBinNode[]) => n.forEach((b) => (b.id && b.kind !== "folder" && bins.push(b.id), walk(b.children ?? [])));
    walk((await conn.rest.mediaBins().catch(() => null)) ?? []);
    const seen = new Map<string, SignageMedia>();
    for (const id of bins) {
      const d = await conn.rest.mediaBin(id).catch(() => null);
      for (const it of d?.items ?? []) {
        if ((it.kind === "image" || it.kind === "video") && !seen.has(it.asset)) seen.set(it.asset, { asset: it.asset, name: it.name, kind: it.kind });
      }
    }
    if (session.conn === conn) this.media = [...seen.values()].sort((a, b) => a.name.localeCompare(b.name));
  }

  private async run<T>(what: string, fn: (c: SignageClient) => Promise<T>): Promise<T | undefined> {
    const c = this.client();
    if (!c) return undefined;
    const r = await sitePage.admin(what, () => fn(c));
    void this.load();
    return r;
  }

  createChannel = (name: string) => this.run("Creating the channel", (c) => c.createChannel({ name }));
  putChannel = (id: string, body: PutSignageChannel) => this.run("Changing the channel", (c) => c.putChannel(id, body));
  deleteChannel = (id: string) => this.run("Deleting the channel", (c) => c.deleteChannel(id));
  createPlaylist = (name: string) => this.run("Creating the playlist", (c) => c.createPlaylist({ id: "", name, items: [] }));
  putPlaylist = (p: SignagePlaylist) => this.run("Saving the playlist", (c) => c.putPlaylist(p.id, p));
  deletePlaylist = (id: string) => this.run("Deleting the playlist", (c) => c.deletePlaylist(id));
  saveFeed = (f: SignageFeed) => this.run("Saving the feed", (c) => (f.id ? c.putFeed(f.id, f) : c.createFeed(f)));
  deleteFeed = (id: string) => this.run("Deleting the feed", (c) => c.deleteFeed(id));
  previewFeed = (f: SignageFeed) => this.run("Reading the feed", (c) => c.previewFeed(f));
  putSettings = (s: SignageSettings) => this.run("Saving the signage settings", (c) => c.putSettings(s));
  places = (q: string) => this.run("Finding the place", (c) => c.places(q));
  pushOverride = (o: SignageOverride) => this.run("Showing the override", (c) => c.pushOverride(o));
  clearOverride = () => this.run("Clearing the override", (c) => c.clearOverride());

  /** The newest picture of a channel's source. */
  thumbnail(source: string): string | null {
    return source && session.conn ? session.conn.rest.sourceThumbnailUrl(source, 480, this.tick) : null;
  }
}

export const signagePage = new SignagePageState();
