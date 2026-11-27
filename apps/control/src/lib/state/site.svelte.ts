// The site's state (Svelte 5 runes): what this box can do (its capabilities) and its part in
// the site's Codex (`GET /codex`: role, active collection, sync), refetched on `codexChanged`.
// The connection pill shows it.
//
// `sitePage` is the Site page's state (round 11): the site's status, devices, join requests,
// Oriel pairings, the site code, events, keys, users, audit and membership (refetched on
// `siteChanged`), and the Codex's protection: settings backups and restores, nightly exports and
// imports (refetched on `codexChanged`). On a show box the Codex's routes answer
// `409 codexElsewhere`: they go through a side connection to the Codex box (plan 2.5). Admin
// actions refused with 403 ask for the site's PIN (or a sign-in) and run again.

import type {
  BackupSetSummary,
  CodexBox,
  CodexExportStatus,
  CodexStatus,
  CodexStorage,
  JoinCodeInfo,
  JoinRequestInfo,
  PairingRequest,
  QueuedRestore,
  ScheduledExport,
  SiteApiKey,
  SiteAuditEntry,
  SiteDevice,
  SiteEvent,
  SiteMembership,
  SiteStatus,
  SiteUser,
} from "$protocol";
import { changes } from "./events";
import { session } from "./session.svelte";
import { shell } from "./shell.svelte";
import { creds } from "./prefs";
import { errorText } from "./errors";
import { ApiRequestError, type RestClient } from "../server/rest";
import { SiteClient } from "../server/site";
import { CodexBackupsClient } from "../server/codexBackups";
import { CodexClient } from "../server/codex";
import { CodexBoxError, CodexBoxLink, boxName } from "../server/codexBox";
import { RECENT_EVENTS, withSiteEvent, type SiteTab } from "../pages/site/logic";
import { PREFS_SCHEMA, PrefsWatcher, collectPrefs, mergePrefs, prefsFromBackup, type Prefs } from "../pages/settings/prefsBackup";
import type { OrielPreferencesBackup } from "$protocol";

class SiteState {
  codex = $state.raw<CodexStatus | null>(null);

  /** True when the server announces `cap` in `GET /server`'s capabilities. */
  has(cap: string): boolean {
    return !!session.server?.capabilities.includes(cap);
  }

  /** Refetches the Codex status (quietly: an older server without `/codex` leaves it null). */
  async loadCodex(): Promise<void> {
    const conn = session.conn;
    if (!conn || !this.has("codex")) {
      this.codex = null;
      return;
    }
    try {
      const c = await conn.rest.codex();
      if (session.conn === conn) this.codex = c;
    } catch {
      /* shown as unknown */
    }
  }

  /** Called once per connection (session.begin): loads, and follows `codexChanged` / `siteChanged`. */
  attach(): () => void {
    this.codex = null;
    void this.loadCodex();
    sitePage.reset();
    const offCodex = changes.on("codexChanged", (e) => {
      if (e.parts.includes("status") || e.parts.includes("collections")) void this.loadCodex();
      sitePage.codexChanged(e.parts);
    });
    const offSite = changes.on("siteChanged", (e) => sitePage.siteChanged(e.parts));
    const offEvent = changes.on("siteEvent", (e) => sitePage.siteEvent(e.event));
    // Events sent while the socket was down are missed: read "Recently" again after a reconnect.
    const offSynced = session.conn?.onSynced((reconnect) => reconnect && sitePage.reconnected());
    const offPrefs = orielPrefs.start();
    return () => {
      offCodex();
      offSite();
      offEvent();
      offSynced?.();
      offPrefs();
    };
  }
}

export const siteState = new SiteState();

/** A question the Site page asks in its own dialog (the PIN, or a sign-in). */
export interface SiteAsk {
  kind: "unlock" | "signIn";
  error: string | null;
}
export interface SiteAnswer {
  pin?: string;
  user?: string;
  password?: string;
}

class SitePageState {
  tab = $state<SiteTab>("devices");
  status = $state.raw<SiteStatus | null>(null);
  devices = $state.raw<SiteDevice[]>([]);
  joinRequests = $state.raw<JoinRequestInfo[]>([]);
  pairings = $state.raw<PairingRequest[]>([]);
  joinCode = $state.raw<JoinCodeInfo | null>(null);
  /** When `joinCode` was read (its `expiresInSeconds` counts from then). */
  joinCodeAt = $state(0);
  events = $state.raw<SiteEvent[]>([]);
  apiKeys = $state.raw<SiteApiKey[]>([]);
  users = $state.raw<SiteUser[]>([]);
  audit = $state.raw<SiteAuditEntry[]>([]);
  membership = $state.raw<SiteMembership | null>(null);
  backups = $state.raw<BackupSetSummary[]>([]);
  restores = $state.raw<QueuedRestore[]>([]);
  exportStatus = $state.raw<CodexExportStatus | null>(null);
  scheduled = $state.raw<ScheduledExport[]>([]);
  storage = $state.raw<CodexStorage | null>(null);
  /** Why a list couldn't be read (by list), shown in its place. */
  problems = $state<Record<string, string>>({});
  /** The Codex box the Codex's routes go to (a show box's side connection), or null. */
  managedOn = $state.raw<CodexBox | null>(null);
  /** The PIN or sign-in question, while open. */
  ask = $state.raw<SiteAsk | null>(null);
  private answer: ((v: SiteAnswer | null) => void) | null = null;
  private link: CodexBoxLink | null = null;
  private linkFor: unknown = null;
  private shown = false;

  /** The site client of the main connection. */
  client(): SiteClient | null {
    const c = session.conn;
    return c ? new SiteClient(c.rest) : null;
  }

  /** The site key and client ID, for requests that aren't JSON. */
  auth() {
    return { token: () => session.siteKey(this.managedOn?.serverId), clientId: session.editor?.clientId ?? null };
  }

  reset() {
    this.status = null;
    this.devices = [];
    this.joinRequests = [];
    this.pairings = [];
    this.joinCode = null;
    this.events = [];
    this.apiKeys = [];
    this.users = [];
    this.audit = [];
    this.membership = null;
    this.backups = [];
    this.restores = [];
    this.exportStatus = null;
    this.scheduled = [];
    this.storage = null;
    this.problems = {};
    this.link?.reset();
    this.link = null;
    this.managedOn = null;
    if (this.shown) void this.loadAll();
  }

  /** The page opened: load everything it shows. */
  open() {
    this.shown = true;
    void this.loadAll();
  }
  close() {
    this.shown = false;
  }

  async loadAll() {
    await Promise.all([
      this.loadStatus(),
      this.loadDevices(),
      this.loadJoin(),
      this.loadJoinCode(),
      this.loadEvents(),
      this.loadSecurity(),
      this.loadBackups(),
      this.loadExport(),
      this.loadStorage(),
    ]);
  }

  /** Reads one list; a failure is kept (shown in its place), never thrown. */
  private async read<T>(key: string, p: Promise<T> | undefined, set: (v: T) => void): Promise<void> {
    if (!p) return;
    const conn = session.conn;
    try {
      const v = await p;
      if (session.conn !== conn) return;
      set(v);
      if (this.problems[key]) {
        const next = { ...this.problems };
        delete next[key];
        this.problems = next;
      }
    } catch (e) {
      if (session.conn !== conn) return;
      this.problems = { ...this.problems, [key]: errorText(e) };
    }
  }

  loadStatus = () =>
    this.read("status", this.client()?.status(), (v) => {
      this.status = v;
      this.membership = v.membership;
    });
  loadDevices = () => this.read("devices", this.client()?.devices(), (v) => (this.devices = v));
  loadJoin = () =>
    Promise.all([
      this.read("joinRequests", this.client()?.joinRequests(), (v) => (this.joinRequests = v)),
      this.read("pairings", this.client()?.pairings(), (v) => (this.pairings = v)),
    ]);
  async loadJoinCode() {
    const s = this.client();
    if (!s) return;
    try {
      const c = await s.joinCode();
      this.joinCode = c;
      this.joinCodeAt = Date.now();
    } catch (e) {
      if (e instanceof ApiRequestError && e.status === 404) this.joinCode = null;
    }
  }
  loadEvents = () => this.read("events", this.client()?.events(RECENT_EVENTS), (v) => (this.events = v));
  loadSecurity = () =>
    Promise.all([
      this.read("apiKeys", this.client()?.apiKeys(), (v) => (this.apiKeys = v)),
      this.read("users", this.client()?.users(), (v) => (this.users = v)),
      this.read("audit", this.client()?.audit(100), (v) => (this.audit = v)),
    ]);
  loadBackups = () =>
    Promise.all([
      this.read("backups", this.codexBoxQuiet((c) => c.sets()), (v) => (this.backups = v)),
      this.read("restores", this.codexBoxQuiet((c) => c.restores()), (v) => (this.restores = v)),
    ]);
  loadExport = () =>
    Promise.all([
      this.read("export", this.codexBoxQuiet((c) => c.exportStatus()), (v) => (this.exportStatus = v)),
      this.read("scheduled", this.codexBoxQuiet((c) => c.scheduled()), (v) => (this.scheduled = v)),
    ]);
  loadStorage = () => this.read("storage", session.conn ? new CodexClient(session.conn.rest).storage() : undefined, (v) => (this.storage = v));

  siteChanged(parts: string[]) {
    if (!this.shown) return;
    if (parts.includes("status") || parts.includes("membership") || parts.includes("session")) void this.loadStatus();
    if (parts.includes("devices")) void this.loadDevices();
    if (parts.includes("joinRequests") || parts.includes("pairings")) void this.loadJoin();
    if (parts.includes("joinCodes")) void this.loadJoinCode();
    if (parts.includes("apiKeys") || parts.includes("users")) void this.loadSecurity();
  }

  /** A `siteEvent`: "Recently" gets it at once (no refetch). */
  siteEvent(e: SiteEvent) {
    if (this.shown) this.events = withSiteEvent(this.events, e);
  }

  /** The socket came back: what happened meanwhile wasn't sent, so read the lists again. */
  reconnected() {
    if (this.shown) void this.loadAll();
  }

  codexChanged(parts: string[]) {
    if (!this.shown) return;
    if (parts.includes("backups") || parts.includes("restores")) void this.loadBackups();
    if (parts.includes("collections") || parts.includes("status")) void this.loadStorage();
    if (parts.includes("exports")) void this.loadExport();
  }

  // ------------------------------------------------------------ the side connection

  private sideLink(): CodexBoxLink {
    if (this.link && this.linkFor === session.conn) return this.link;
    this.link?.reset();
    const link = new CodexBoxLink({
      token: (box) => session.siteKey(box?.serverId),
      clientId: () => session.editor?.clientId ?? null,
      prompts: {
        ask: (box) => shell.confirm("Codex box", `Backups and exports are kept on ${boxName(box, siteState.codex?.sync?.name)}. Use it for this?`, `Use ${boxName(box, siteState.codex?.sync?.name)}`, false),
        unpaired: async (box) => {
          const name = boxName(box, siteState.codex?.sync?.name);
          const go = await shell.confirm(
            "Codex box",
            `Backups and exports are kept on ${name}, and this computer isn't paired with it (Oriel reaches this box as the box itself). Pair it with ${name} now? Oriel comes back to this box afterwards.`,
            `Pair with ${name}`,
            false,
          );
          if (go) await session.pairWithCodexBox(box);
        },
        unreachable: async (box) => {
          const where = box.addresses.length ? box.addresses.map((a) => a.replace(/^https?:\/\//, "")).join(", ") : "no address known yet";
          const go = await shell.confirm("Codex box unreachable", `${boxName(box, siteState.codex?.sync?.name)} didn't answer (${where}). Connect Oriel to it instead? The show on this box keeps playing.`, "Connect to it", false);
          if (!go) return;
          const address = await shell.askText(`Connect to ${boxName(box, siteState.codex?.sync?.name)}`, box.addresses[0] ?? "", "Connect", "Address");
          if (address) await session.connectFromMain(address, { codexBox: box.serverId });
        },
      },
    });
    link.subscribe((b) => (this.managedOn = b));
    this.link = link;
    this.linkFor = session.conn;
    return link;
  }

  /** Runs a Codex protection request here, or on the Codex box (a show box asks first). */
  codexBox<T>(fn: (c: CodexBackupsClient, rest: RestClient) => Promise<T>): Promise<T> {
    const conn = session.conn;
    if (!conn) return Promise.reject(new Error("Not connected"));
    return this.sideLink().run(conn.rest, (rest) => fn(new CodexBackupsClient(rest), rest));
  }

  /** Without asking: on a show box only once the operator agreed to use the Codex box. */
  codexBoxQuiet<T>(fn: (c: CodexBackupsClient) => Promise<T>): Promise<T> | undefined {
    const conn = session.conn;
    if (!conn) return undefined;
    if (this.link && this.linkFor === conn && this.managedOn) return this.link.run(conn.rest, (rest) => fn(new CodexBackupsClient(rest)));
    return fn(new CodexBackupsClient(conn.rest)).catch((e) => {
      if (e instanceof ApiRequestError && e.code === "codexElsewhere") throw new Error(`Kept on ${boxName(e.codex, siteState.codex?.sync?.name)}`);
      throw e;
    });
  }

  /** True when a list failed because it is kept on the Codex box (a show box). */
  elsewhere(key: string): boolean {
    return (this.problems[key] ?? "").startsWith("Kept on ");
  }

  /** The operator asked to use the Codex box for this page (a show box). */
  async useCodexBox() {
    try {
      await this.codexBox((c) => c.sets());
      await Promise.all([this.loadBackups(), this.loadExport()]);
    } catch (e) {
      if (!(e instanceof CodexBoxError && e.reason === "declined")) this.fail(e, "Reaching the Codex box");
    }
  }

  // ------------------------------------------------------------ admin actions

  /**
   * Runs an admin action; on 403 asks for the site's PIN (access `pin`) or a sign-in
   * (`users`) and runs it again. Failures become one notice. Returns the result, or
   * `undefined` when it didn't happen.
   */
  async admin<T>(what: string, fn: () => Promise<T>): Promise<T | undefined> {
    for (let attempt = 0; attempt < 3; attempt++) {
      try {
        return await fn();
      } catch (e) {
        if (e instanceof ApiRequestError && e.status === 403 && attempt < 2 && (await this.unlockFor(e))) continue;
        if (e instanceof CodexBoxError && e.reason === "declined") return undefined;
        this.fail(e, what);
        return undefined;
      }
    }
    return undefined;
  }

  /** After a 403: asks for the PIN or a sign-in, as the site's access level needs. */
  async unlockFor(e?: ApiRequestError): Promise<boolean> {
    const site = this.client();
    if (!site) return false;
    let access = this.status?.you.access;
    try {
      access = (await site.session()).access;
    } catch {
      /* keep what we knew */
    }
    if (access !== "pin" && access !== "users") {
      if (e) session.conn?.pushNotice("warning", e.message, "site:forbidden");
      return false;
    }
    let error: string | null = null;
    for (;;) {
      const a = await this.question({ kind: access === "pin" ? "unlock" : "signIn", error });
      if (!a) return false;
      try {
        if (access === "pin") await site.unlock(a.pin ?? "");
        else await site.signIn({ user: a.user ?? "", password: a.password ?? "" });
        void this.loadStatus();
        return true;
      } catch (err) {
        error = err instanceof ApiRequestError && err.status === 401 ? (access === "pin" ? "Wrong PIN" : "Wrong user name or password") : errorText(err);
      }
    }
  }

  private question(q: SiteAsk): Promise<SiteAnswer | null> {
    this.answer?.(null);
    this.ask = q;
    return new Promise((resolve) => (this.answer = resolve));
  }

  /** The PIN / sign-in dialog's answer (null: cancelled). */
  reply(v: SiteAnswer | null) {
    const r = this.answer;
    this.answer = null;
    this.ask = null;
    r?.(v);
  }

  async lock() {
    const s = this.client();
    if (!s) return;
    try {
      if (this.status?.you.access === "users") await s.signOut();
      else await s.lock();
    } catch (e) {
      this.fail(e, "Locking");
    }
    void this.loadStatus();
  }

  fail(e: unknown, what: string) {
    const forbidden = e instanceof ApiRequestError && e.status === 403;
    const msg = e instanceof CodexBoxError ? `${what} isn't possible here: ${e.message}.` : forbidden ? `${what} is for the site's admins: ${errorText(e)}` : `${what} failed: ${errorText(e)}`;
    session.conn?.pushNotice(forbidden ? "warning" : "error", msg, `site:${what}`);
  }

  /** Runs a site admin action and refetches what it changed. */
  async act(what: string, fn: (s: SiteClient) => Promise<unknown>, after: () => Promise<unknown> = () => Promise.resolve()): Promise<boolean> {
    const s = this.client();
    if (!s) return false;
    const ok = (await this.admin(what, async () => {
      await fn(s);
      return true;
    })) === true;
    await after();
    return ok;
  }

  async newCode() {
    const s = this.client();
    if (!s) return;
    const c = await this.admin("Making a site code", () => s.newJoinCode());
    if (c) {
      this.joinCode = c;
      this.joinCodeAt = Date.now();
      void this.loadEvents();
    }
  }
}

export const sitePage = new SitePageState();

/**
 * Oriel's preferences backup (plan 4.10): saved to the Codex (`PUT /codex/backups/oriel`) a
 * few seconds after they change, and "Restore my Oriel settings" (this computer's own backup,
 * or another Oriel computer's set). On a show box they go to the Codex box once the operator
 * agreed to use it (never asked from the background).
 */
class OrielPrefsState {
  /** This computer's newest backup, once read (null: none, or not read). */
  backup = $state.raw<OrielPreferencesBackup | null>(null);
  /** Why the last save failed, or null. */
  problem = $state<string | null>(null);
  lastSaved = $state<string | null>(null);
  /** A show box: the backup is kept on the Codex box and the operator hasn't agreed to use it
   *  yet. Saving waits (no retries in the background) until they do (`useCodexBox`). */
  elsewhere = $state(false);
  private watcher: PrefsWatcher | null = null;

  /** Starts watching for this connection (from siteState.attach). */
  start(): () => void {
    this.backup = null;
    this.problem = null;
    this.elsewhere = false;
    if (!siteState.has("codex")) return () => {};
    const w = new PrefsWatcher({
      read: () => collectPrefs((k) => creds.pref(k)),
      save: (p) => this.save(p),
    });
    this.watcher = w;
    const t = setInterval(() => w.check(), 2000);
    w.check();
    void this.load();
    return () => {
      clearInterval(t);
      w.stop();
      if (this.watcher === w) this.watcher = null;
    };
  }

  private async save(p: Prefs): Promise<boolean> {
    const run = sitePage.codexBoxQuiet((c) => c.saveOrielPreferences({ schemaVersion: PREFS_SCHEMA, preferences: p }));
    if (!run) return false;
    try {
      const r = await run;
      this.lastSaved = r.snapshot.takenAt;
      this.problem = null;
      this.elsewhere = false;
      return true;
    } catch (e) {
      this.problem = errorText(e);
      // Kept on the Codex box: retrying here would only be refused again every few seconds.
      this.elsewhere = this.problem.startsWith("Kept on ");
      return this.elsewhere;
    }
  }

  /** The operator agreed to use the Codex box (a show box): save now and read the backup there. */
  async useCodexBox() {
    await sitePage.useCodexBox();
    if (!sitePage.managedOn) return;
    await this.save(collectPrefs((k) => creds.pref(k)));
    await this.load();
  }

  async load() {
    const run = sitePage.codexBoxQuiet((c) => c.orielPreferences());
    if (!run) return;
    try {
      this.backup = await run;
    } catch {
      this.backup = null;
    }
  }

  /** Writes restored preferences and reloads Oriel so every page picks them up. */
  apply(backup: Prefs) {
    const merged = mergePrefs(collectPrefs((k) => creds.pref(k)), backup);
    for (const [k, v] of Object.entries(merged)) creds.setPref(k, v);
    this.watcher?.markSaved(merged);
    location.reload();
  }

  /** "Restore my Oriel settings" from this computer's own backup. */
  async restoreOwn() {
    const b = await sitePage.admin("Restoring Oriel's settings", () => sitePage.codexBox((c) => c.orielPreferences()));
    const p = b ? prefsFromBackup(b) : null;
    if (p) this.apply(p);
    else if (b) sitePage.fail(new Error("the backup was made by a newer Oriel"), "Restoring Oriel's settings");
  }

  /** Takes over another Oriel computer's backup (its newest, or `snapshot`): any paired
   * client may, no admin needed. */
  async restoreFrom(setId: string, snapshot?: string) {
    let b: OrielPreferencesBackup;
    try {
      b = await sitePage.codexBox((c) => c.orielPreferences(snapshot, setId));
    } catch (e) {
      sitePage.fail(e, "Taking over Oriel's settings");
      return;
    }
    const p = prefsFromBackup(b);
    if (p) this.apply(p);
    else sitePage.fail(new Error("that backup was made by a newer Oriel"), "Taking over Oriel's settings");
  }
}

export const orielPrefs = new OrielPrefsState();
