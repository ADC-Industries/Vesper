// A show box's own, dormant collection and its upload to the site's Codex (G15): the offer on
// the Library's connection card, the progress (polled every second while copying; the server
// sends `codexChanged {status}` when it starts and ends), then "Show it here".

import type { CollectionSummary, DormantUpload } from "$protocol";
import { CodexClient } from "../server/codex";
import { session } from "./session.svelte";
import { sitePage, siteState } from "./site.svelte";
import { libraryPage } from "./library.svelte";

class DormantState {
  upload = $state.raw<DormantUpload | null>(null);
  busy = $state(false);
  private timer: ReturnType<typeof setTimeout> | null = null;
  private seq = 0;

  private get codex(): CodexClient | null {
    const conn = session.conn;
    return conn ? new CodexClient(conn.rest) : null;
  }

  /** Reads the offer / progress again (nothing on a box without a dormant collection). */
  async load() {
    const codex = this.codex;
    const seq = ++this.seq;
    if (!codex || !siteState.codex?.dormantCollection) {
      this.upload = null;
      this.stopPolling();
      return;
    }
    try {
      const u = await codex.dormantUpload();
      if (seq !== this.seq) return;
      this.upload = u;
      if (u.state === "copying") this.poll();
      else this.stopPolling();
    } catch {
      if (seq === this.seq) this.upload = null;
    }
  }

  private poll() {
    if (this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.load();
    }, 1000);
  }

  private stopPolling() {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }

  /** Starts (or resumes) the upload (admins). */
  async start() {
    const codex = this.codex;
    if (!codex) return;
    this.busy = true;
    try {
      const u = await sitePage.admin("Uploading this box's collection", () => codex.startDormantUpload());
      if (u) {
        this.upload = u;
        if (u.state === "copying") this.poll();
      }
    } finally {
      this.busy = false;
    }
  }

  async cancel() {
    const codex = this.codex;
    if (!codex) return;
    await sitePage.admin("Cancelling the upload", () => codex.cancelDormantUpload());
    void this.load();
  }

  /** Shows the uploaded collection on this box (its assignment). */
  async showHere() {
    const u = this.upload;
    if (!u?.collection) return;
    const summary = { collection: { id: u.collection, name: u.collectionName ?? u.name }, active: false } as unknown as CollectionSummary;
    await libraryPage.switchCollection(summary);
  }

  reset() {
    this.seq++;
    this.stopPolling();
    this.upload = null;
  }
}

export const dormantState = new DormantState();
