// Site ▸ Displays (Vesper Mosaic, docs/MOSAIC-PLAN.md): the site's display boxes, their screens
// and the channels (`GET /mosaic`), read while the tab is shown: again on `siteChanged`
// (`mosaic`) and every few seconds for the thumbnails. Also this server's own displays and the
// outputs driving them (read-only: they are Vesper's, Configure Screens), and the network's
// sources for the feed picker. Changes go through `sitePage.admin`, so a 403 asks for the PIN or
// a sign-in.

import type { DisplayInfo, MosaicFeed, MosaicScreenSettings, MosaicServiceMode, MosaicState, OutputEntry, PutMosaicWall, ScreenEntry } from "$protocol";
import { changes } from "./events";
import { session } from "./session.svelte";
import { sitePage } from "./site.svelte";
import { errorText } from "./errors";
import { MosaicClient } from "../server/mosaic";
import { SignageClient } from "../server/signage";
import { setSignageSources, signageSourceNames } from "../pages/site/signage";

export const MOSAIC_POLL_MS = 5000;

class MosaicPageState {
  state = $state.raw<MosaicState | null>(null);
  sources = $state.raw<string[]>([]);
  /** This server's own displays and what drives them (`GET /displays`). */
  displays = $state.raw<DisplayInfo[]>([]);
  outputs = $state.raw<OutputEntry[]>([]);
  screens = $state.raw<ScreenEntry[]>([]);
  problem = $state<string | null>(null);
  loaded = $state(false);
  /** Bumped on every read: the thumbnails are fetched again. */
  tick = $state(0);
  private timer: ReturnType<typeof setInterval> | null = null;
  private off: (() => void) | null = null;

  client(): MosaicClient | null {
    return session.conn ? new MosaicClient(session.conn.rest) : null;
  }

  /** The tab opened: load now, on `siteChanged` (`mosaic`) and every few seconds. */
  open() {
    this.close();
    void this.load();
    void this.loadOwn();
    this.timer = setInterval(() => void this.load(), MOSAIC_POLL_MS);
    this.off = changes.on("siteChanged", (e) => {
      if (e.parts.includes("mosaic")) void this.load();
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
      const signage = session.server?.capabilities.includes("signage") ? new SignageClient(conn!.rest).state().catch(() => null) : null;
      const [s, src, sg] = await Promise.all([c.state(), conn!.rest.liveSources().catch(() => null), signage]);
      if (session.conn !== conn) return;
      if (sg) setSignageSources(sg.channels);
      this.state = s;
      // Signage channels' sources are listed even before the network lists them (Site ▸ Signage).
      if (src) this.sources = [...new Set([...src.sources.map((x) => x.name), ...signageSourceNames()])];
      this.problem = null;
      this.tick++;
    } catch (e) {
      if (session.conn === conn) this.problem = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  /** This server's own displays, outputs and screens (read-only here; a server without
   * `GET /displays` shows none). */
  async loadOwn() {
    const conn = session.conn;
    if (!conn) return;
    try {
      const [d, o, s] = await Promise.all([conn.rest.displays(), conn.rest.outputs(), conn.rest.screenEntries()]);
      if (session.conn !== conn) return;
      this.displays = d;
      this.outputs = o;
      this.screens = s;
    } catch {
      /* the section stays empty */
    }
  }

  private async run<T>(what: string, fn: (c: MosaicClient) => Promise<T>): Promise<T | undefined> {
    const c = this.client();
    if (!c) return undefined;
    const r = await sitePage.admin(what, () => fn(c));
    void this.load();
    return r;
  }

  setFeed = (machine: string, n: number, feed: MosaicFeed) => this.run("Changing the display's feed", (c) => c.setFeed(machine, n, feed));
  setSettings = (machine: string, n: number, s: MosaicScreenSettings) => this.run("Changing the display's settings", (c) => c.setSettings(machine, n, s));
  identify = (machine: string, n: number) => this.run("Identifying the display", (c) => c.identify(machine, n));
  testPattern = (machine: string, n: number, on: boolean) => this.run("The test pattern", (c) => c.testPattern(machine, n, { on }));
  identifyAll = () => this.run("Identifying every display", (c) => c.identifyAll());
  testPatternAll = (on: boolean) => this.run("The test pattern on every display", (c) => c.testPatternAll({ on }));
  setAddress = (machine: string, address: string) => this.run("Setting the box's address", (c) => c.putBox(machine, { address }));
  createChannel = (name: string, source: string) => this.run("Creating the channel", (c) => c.createChannel({ name, source }));
  renameChannel = (id: string, name: string) => this.run("Renaming the channel", (c) => c.putChannel(id, { name }));
  setChannelSource = (id: string, source: string) => this.run("Changing the channel's source", (c) => c.putChannel(id, { source }));
  deleteChannel = (id: string) => this.run("Deleting the channel", (c) => c.deleteChannel(id));
  createWall = (w: PutMosaicWall) => this.run("Creating the wall", (c) => c.createWall(w));
  putWall = (id: string, w: PutMosaicWall) => this.run("Changing the wall", (c) => c.putWall(id, w));
  setWallFeed = (id: string, feed: MosaicFeed) => this.run("Changing the wall's feed", (c) => c.setWallFeed(id, feed));
  identifyWall = (id: string) => this.run("Identifying the wall", (c) => c.identifyWall(id));
  deleteWall = (id: string) => this.run("Deleting the wall", (c) => c.deleteWall(id));
  setService = (machine: string, mode: MosaicServiceMode) =>
    this.run(mode === "off" ? "Switching the display service off" : "Using the machine as a display box", (c) => c.setService(machine, mode));
  removeLogo = () => this.run("Removing the logo", (c) => c.deleteLogo());
  setLogo = (file: File) =>
    this.run("Setting the site's logo", (c) => c.putLogo(file, { token: () => session.siteKey(), clientId: session.editor?.clientId ?? null }));

  logoUrl(): string | null {
    const h = this.state?.logo?.hash;
    return h ? (this.client()?.logoUrl(h) ?? null) : null;
  }

  /** The newest picture of a source (the wall's preview, cut by its tiles). */
  sourceThumbnail(source: string): string | null {
    return source && session.conn ? session.conn.rest.sourceThumbnailUrl(source, 640, this.tick) : null;
  }

  thumbnail(machine: string, n: number): string | null {
    return this.client()?.thumbnailUrl(machine, n, 320, this.tick) ?? null;
  }
}

export const mosaicPage = new MosaicPageState();
