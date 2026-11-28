// Site ▸ Updates (G10): the site's update channel and every machine's Vesper Launcher, read
// while the tab is shown (every 5 s: Launchers report every 30 s and an update takes a while,
// so a poll is enough; nothing is pushed over the WebSocket). Admin actions (upload, delete,
// Update, Roll back, Add app, the GitHub check's settings) go through `sitePage.admin`, so a 403
// asks for the PIN or a sign-in.

import type { CatalogueProduct, GithubReleases, GithubSettings, LauncherAction, LauncherEntry, UpdatePackage } from "$protocol";
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
  /** The server's check of Vesper's releases on GitHub (`null`: an older server without it). */
  github = $state.raw<GithubReleases | null>(null);
  /** The machine whose "Add app" list is open, and what it can add (`null`: loading). */
  addFor = $state<string | null>(null);
  catalogue = $state.raw<CatalogueProduct[] | null>(null);
  catalogueProblem = $state<string | null>(null);
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
      const [l, p, g] = await Promise.all([c.launchers(), c.packages(), c.github().catch(() => null)]);
      if (session.conn !== conn) return;
      this.launchers = l;
      this.packages = p;
      this.github = g;
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
    const what = action.type === "update" ? `Updating ${action.product}` : action.type === "install" ? `Adding ${action.product}` : `Rolling back ${action.product}`;
    const entry = await sitePage.admin(what, () => c.queue(machineId, action));
    if (!entry) return false;
    this.launchers = this.launchers.map((e) => (e.report.machineId === machineId ? entry : e));
    return true;
  }

  /** Opens (or closes, when open) a machine's "Add app" list. */
  async openAdd(machineId: string) {
    if (this.addFor === machineId) {
      this.addFor = null;
      return;
    }
    const c = this.client();
    if (!c) return;
    this.addFor = machineId;
    this.catalogue = null;
    this.catalogueProblem = null;
    try {
      const cat = await c.catalogue(machineId);
      if (this.addFor === machineId) this.catalogue = cat.products;
    } catch (e) {
      if (this.addFor === machineId) this.catalogueProblem = errorText(e);
    }
  }

  /** "Add app": queues the install on the machine's Launcher. */
  async add(machineId: string, product: string, version: string): Promise<boolean> {
    const ok = await this.queue(machineId, { type: "install", product, version });
    if (ok) this.addFor = null;
    return ok;
  }

  /** Changes how the server checks GitHub (admins). */
  async setGithub(s: GithubSettings): Promise<boolean> {
    const c = this.client();
    if (!c) return false;
    const g = await sitePage.admin("Changing the GitHub check", () => c.setGithub(s));
    if (g) this.github = g;
    return !!g;
  }

  /** Checks GitHub now (admins). */
  async checkGithub(): Promise<void> {
    const c = this.client();
    if (!c) return;
    const g = await sitePage.admin("Checking GitHub", () => c.checkGithub());
    if (g) this.github = { ...g, checking: true };
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
