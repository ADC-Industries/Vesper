// Site ▸ Updates (G10): the site's update channel and every machine's Vesper Launcher, read
// while the tab is shown (every 5 s: Launchers report every 30 s and an update takes a while,
// so a poll is enough; nothing is pushed over the WebSocket). Admin actions (upload, delete,
// Update, Roll back) go through `sitePage.admin`, so a 403 asks for the PIN or a sign-in.

import type { LauncherAction, LauncherEntry, UpdatePackage } from "$protocol";
import { session } from "./session.svelte";
import { sitePage } from "./site.svelte";
import { errorText } from "./errors";
import { UpdatesClient } from "../server/updates";

export const UPDATES_POLL_MS = 5000;

class UpdatesState {
  launchers = $state.raw<LauncherEntry[]>([]);
  packages = $state.raw<UpdatePackage[]>([]);
  problem = $state<string | null>(null);
  loaded = $state(false);
  uploading = $state(false);
  private timer: ReturnType<typeof setInterval> | null = null;

  client(): UpdatesClient | null {
    return session.conn ? new UpdatesClient(session.conn.rest) : null;
  }

  /** The tab opened: load now and every few seconds until it closes. */
  open() {
    this.close();
    void this.load();
    this.timer = setInterval(() => void this.load(), UPDATES_POLL_MS);
  }

  close() {
    if (this.timer) clearInterval(this.timer);
    this.timer = null;
  }

  async load() {
    const c = this.client();
    if (!c) return;
    const conn = session.conn;
    try {
      const [l, p] = await Promise.all([c.launchers(), c.packages()]);
      if (session.conn !== conn) return;
      this.launchers = l;
      this.packages = p;
      this.problem = null;
    } catch (e) {
      if (session.conn === conn) this.problem = errorText(e);
    } finally {
      this.loaded = true;
    }
  }

  /** Queues Update / Roll back for a machine's Launcher (it picks it up with its next report). */
  async queue(machineId: string, action: LauncherAction): Promise<boolean> {
    const c = this.client();
    if (!c) return false;
    const what = action.type === "update" ? `Updating ${action.product}` : `Rolling back ${action.product}`;
    const entry = await sitePage.admin(what, () => c.queue(machineId, action));
    if (!entry) return false;
    this.launchers = this.launchers.map((e) => (e.report.machineId === machineId ? entry : e));
    return true;
  }

  /** Uploads a signed package (its file and its `.minisig` signature file). */
  async upload(file: File, signature: File): Promise<UpdatePackage | null> {
    const c = this.client();
    if (!c) return null;
    this.uploading = true;
    try {
      const text = await signature.text();
      const auth = { token: () => session.siteKey(), clientId: session.editor?.clientId ?? null };
      const p = await sitePage.admin("Uploading the package", () => c.upload(file, file.name, text, auth));
      if (p) await this.load();
      return p ?? null;
    } finally {
      this.uploading = false;
    }
  }

  async remove(id: string): Promise<boolean> {
    const c = this.client();
    if (!c) return false;
    const ok = (await sitePage.admin("Deleting the package", async () => (await c.deletePackage(id), true))) === true;
    if (ok) await this.load();
    return ok;
  }
}

export const updatesPage = new UpdatesState();
