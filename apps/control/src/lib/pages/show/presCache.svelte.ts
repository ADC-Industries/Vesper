// The documents behind the Show page's sections: each presentation is fetched once, when its
// section first comes near the viewport, and its thumbnails are prefetched at the size the grid
// asks for. The session keeps it for the whole connection (not only while the Show page is
// mounted): a `documentChanged` for a cached presentation refetches it, and after a reconnect or
// dropped events every cached (or failed) one is fetched again (`refreshAll`).

import { SvelteMap } from "svelte/reactivity";
import type { Presentation } from "$protocol";
import { flattenSlides, type GridSlide } from "../../model";
import type { Connection } from "../../server";
import { cuesBySlide, thumbVersions, type MediaCue } from "./sections";
import { changes } from "../../state/events";

export interface LoadedPresentation {
  presentation: Presentation;
  slides: GridSlide[];
  cues: Map<string, MediaCue[]>;
  /** Each slide's thumbnail version (`thumbVersions`). */
  versions: Map<string, string>;
}

export function loaded(p: Presentation): LoadedPresentation {
  return { presentation: p, slides: flattenSlides(p), cues: cuesBySlide(p), versions: thumbVersions(p) };
}

class PresentationCache {
  docs = new SvelteMap<string, LoadedPresentation>();
  /** Presentations that could not be loaded (deleted, or the request failed). */
  failed = new SvelteMap<string, true>();
  private inflight = new Map<string, Promise<void>>();
  private token = new Map<string, object>();
  private prefetched = new Set<string>();
  private conn: Connection | null = null;

  /** Forgets everything when the connection changes. */
  use(conn: Connection | null) {
    if (conn === this.conn) return;
    this.conn = conn;
    this.docs.clear();
    this.failed.clear();
    this.inflight.clear();
    this.token.clear();
    this.prefetched.clear();
  }

  /** For the whole connection (`session.begin`): follows `documentChanged`. Returns the
   *  cleanup. */
  attach(conn: Connection): () => void {
    this.use(conn);
    return changes.on("documentChanged", (e) => {
      if (e.kind === "presentation") void this.refresh(e.id);
    });
  }

  /** Loads a presentation unless it is cached (or loading). */
  load(id: string): Promise<void> {
    if (this.docs.has(id)) return Promise.resolve();
    return this.fetch(id);
  }

  /** Fetches it again (after a change), if it was loaded. */
  refresh(id: string): Promise<void> {
    if (!this.docs.has(id) && !this.failed.has(id)) return Promise.resolve();
    return this.fetch(id, true);
  }

  /** Fetches every cached presentation again, and those that failed (a reconnect, or change
   *  events the server dropped: any of them may have changed meanwhile). */
  refreshAll(): Promise<void> {
    const ids = new Set([...this.docs.keys(), ...this.failed.keys()]);
    return Promise.all([...ids].map((id) => this.fetch(id, true))).then(() => {});
  }

  /** Takes a document fetched elsewhere (the open presentation). */
  put(p: Presentation) {
    const cur = this.docs.get(p.id);
    if (cur?.presentation === p) return;
    this.docs.set(p.id, loaded(p));
    this.failed.delete(p.id);
  }

  private fetch(id: string, again = false): Promise<void> {
    const conn = this.conn;
    if (!conn) return Promise.resolve();
    const busy = this.inflight.get(id);
    if (busy && !again) return busy;
    const token = {};
    this.token.set(id, token);
    const p: Promise<void> = (async () => {
      try {
        const doc = await conn.rest.presentation(id);
        // A later request for it (a newer edit) answers for it, even when this one ends last.
        if (this.conn !== conn || this.token.get(id) !== token) return;
        this.docs.set(id, loaded(doc));
        this.failed.delete(id);
      } catch {
        if (this.conn !== conn || this.token.get(id) !== token) return;
        if (!this.docs.has(id)) this.failed.set(id, true);
      } finally {
        if (this.token.get(id) === token) {
          this.inflight.delete(id);
          this.token.delete(id);
        }
      }
    })();
    this.inflight.set(id, p);
    return p;
  }

  /** Warms the server's thumbnail cache for one presentation at one width (once each). */
  prefetch(id: string, width: number) {
    const conn = this.conn;
    const key = `${id}@${width}`;
    if (!conn || this.prefetched.has(key)) return;
    this.prefetched.add(key);
    conn.command({ type: "prefetchThumbnails", presentation: id, width }).catch(() => {
      // Offline: the thumbnails load on demand later; try again next time.
      this.prefetched.delete(key);
    });
  }
}

export const presCache = new PresentationCache();
