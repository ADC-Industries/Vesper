// The core session (Svelte 5 runes): the connection to the server and pairing, the server's
// info, the first load (`refreshAll`) and the `documentChanged` refetches of the core's own
// documents, the editor and undo, the playlist tree and the open playlist, the open
// presentation, looks, clear groups, global groups and inline renaming. Show state itself lives
// in the Connection's store.
//
// Each area's documents live in their own module next to it in state/, each attached once per
// connection (`begin`) to follow its own `documentChanged` kinds: media (bins, audio panel),
// showDocs (messages, props, themes), timers and stage. View state lives there too: shell (page,
// mode, dialogs), show (the Show page's view), library, screens and site.

import type {
  ClearGroup,
  CodexBox,
  Color,
  Command,
  DocumentKind,
  GlobalGroup,
  PlaylistItem,
  PlaylistItemDetail,
  Look,
  PlaylistDetail,
  PlaylistItemRef,
  PlaylistTreeNode,
  Presentation,
  ServerInfo,
} from "$protocol";
import { PROTOCOL_VERSION } from "$protocol";
import { get } from "svelte/store";
import { DEFAULT_PORT, SERVER_NAME } from "../config";
import {
  ApiRequestError,
  CommandError,
  Connection,
  RestClient,
  awaitPairing,
  isPending,
  normaliseBaseUrl,
  pairingEndedText,
  settleCredential,
  type WaitControl,
} from "../server";
import { isTauri } from "../tauri";
import { boxName } from "../server/codexBox";
import { flattenSlides, type GridSlide } from "../model";
import { LatestLive } from "../live";
import { Editor } from "../edit/editor";
import { uuid } from "../edit/ids";
import { openFailure } from "../edit/open";
import type { UndoState } from "../edit/undo";
import type { DocumentChangedMessage } from "../server";
import { errorText } from "./errors";
import { changes } from "./events";
import { creds } from "./prefs";
import { cleanComputerName, defaultComputerName } from "../computerName";

/** The preference holding the name this computer pairs with. */
const COMPUTER_NAME_PREF = "computerName";
import { flatPlaylists, libraryState } from "./library.svelte";
import { mediaState } from "./media.svelte";
import { screensState } from "./screens.svelte";
import { shell } from "./shell.svelte";
import { showDocsState } from "./showDocs.svelte";
import { siteState } from "./site.svelte";
import { showView } from "./show.svelte";
import { presCache } from "../pages/show/presCache.svelte";
import { stageState } from "./stage.svelte";
import { timersState } from "./timers.svelte";
import { presence } from "./presence.svelte";
import { sourcesState } from "./sources.svelte";

export type View = "connect" | "main";

/** The connect screen's line after the site removed this computer (socket closed with 4003). */
export const REVOKED_TEXT = "This computer was removed from the site. Pair again.";

export interface PairingNeeded {
  baseUrl: string;
  server: ServerInfo;
  /** Waiting for a site admin's approval (no code, or a relayed pairing). */
  waiting?: boolean;
  /** Typed at a server of the site that isn't its server, the code was wrong or expired: the
   *  pairing waits for approval, and the code may be typed again. */
  codeWrong?: boolean;
  /** Pairing only for the site key (an Oriel on a show box itself, for the Codex box's
   *  uploads and collections): afterwards (or on Back) Oriel connects to this address again. */
  returnTo?: string;
}

/** What the centre grid shows. */
export interface OpenPresentation {
  presentation: Presentation;
  slides: GridSlide[];
  /** Set when it was opened from a playlist item: triggers then carry playlist context. */
  playlistItem: PlaylistItemRef | null;
  /** A prop or stage layout opened as a one-slide document in the Edit view (prop-document
   * mode). */
  prop?: boolean;
  /** The one-slide document is a stage layout (not a prop). */
  stageLayout?: boolean;
  /** The document is a theme: its template slides (opened like a prop). */
  theme?: boolean;
}

/** What is being renamed inline in the left column. */
export interface Renaming {
  kind: "library" | "presentation" | "playlist" | "item";
  id: string;
}

/** Playlist header colours offered by the header dialog. */
export const HEADER_COLORS: Color[] = [
  { r: 0.85, g: 0.2, b: 0.25 },
  { r: 0.95, g: 0.55, b: 0.1 },
  { r: 0.95, g: 0.8, b: 0.15 },
  { r: 0.25, g: 0.7, b: 0.35 },
  { r: 0.15, g: 0.6, b: 0.75 },
  { r: 0.2, g: 0.42, b: 0.9 },
  { r: 0.6, g: 0.35, b: 0.85 },
  { r: 0.55, g: 0.55, b: 0.55 },
];

/** IDs from the root to the node `id` (inclusive), or [] when it isn't in the tree. */
function findPath(nodes: PlaylistTreeNode[], id: string): string[] {
  for (const n of nodes) {
    if (n.id === id) return [n.id];
    const sub = findPath(n.children, id);
    if (sub.length) return [n.id, ...sub];
  }
  return [];
}

class Session {
  view = $state<View>("connect");
  conn = $state.raw<Connection | null>(null);
  server = $state.raw<ServerInfo | null>(null);
  baseUrl = $state("");

  // Connect screen
  connecting = $state(false);
  connectError = $state<string | null>(null);
  pairing = $state.raw<PairingNeeded | null>(null);
  /** The site removed this computer (its key was revoked): the connect screen says so. */
  revoked = $state(false);

  // Playlists
  playlistTree = $state.raw<PlaylistTreeNode[]>([]);
  playlistId = $state<string | null>(null);
  playlist = $state.raw<PlaylistDetail | null>(null);

  // Centre
  open = $state.raw<OpenPresentation | null>(null);
  openLoading = $state(false);

  // Right column
  looks = $state.raw<Look[]>([]);
  clearGroups = $state.raw<ClearGroup[]>([]);
  /** What was open before a prop was opened for editing (restored when editing ends). */
  private propBefore: OpenPresentation | null = null;

  // Editing
  editor = $state.raw<Editor | null>(null);
  undoState = $state<UndoState>({ undoLabel: null, redoLabel: null, busy: false });
  globalGroups = $state.raw<GlobalGroup[]>([]);
  renaming = $state<Renaming | null>(null);
  /** Set by the Reflow editor: sends waiting typing (undo waits for it). */
  reflowFlush: (() => Promise<void>) | null = null;

  private unsubs: (() => void)[] = [];
  private openSeq = 0;
  private reopenSeq = 0;

  /** Where to connect first: the last server, else this page's own origin (web build). */
  defaultAddress(): string {
    const last = creds.lastServer();
    if (last) return last.baseUrl;
    if (typeof location !== "undefined" && /^https?:$/.test(location.protocol)) return location.origin;
    return `127.0.0.1:${DEFAULT_PORT}`;
  }

  /** Tries the default address once at startup; stays on the connect screen if it fails. */
  async autoConnect(): Promise<void> {
    await this.connect(this.defaultAddress(), true);
  }

  /**
   * Connects to a server (the connect screen). `site`: the site's Codex box and this client's
   * site key, tried there when this computer has no key saved for it (a site key works on
   * every server of the site).
   */
  async connect(address: string, quiet = false, site?: { serverId: string; token: string | null }): Promise<void> {
    this.connectError = null;
    this.pairing = null;
    let baseUrl: string;
    try {
      baseUrl = normaliseBaseUrl(address, DEFAULT_PORT);
    } catch (e) {
      this.connectError = errorText(e);
      return;
    }
    this.connecting = true;
    try {
      const anon = new RestClient({ baseUrl, token: () => null, timeoutMs: 4000 });
      const first = await anon.serverInfo();
      if (first.protocolVersion !== PROTOCOL_VERSION) {
        throw new Error(
          `This server speaks protocol version ${first.protocolVersion}; this app needs version ${PROTOCOL_VERSION}. Update the app or the server.`,
        );
      }
      const saved = creds.token(first.id) ?? (site?.token && first.id === site.serverId ? site.token : null);
      const shown = saved ? await new RestClient({ baseUrl, token: () => saved }).serverInfo() : first;
      if (shown.auth === "unpaired") {
        if (saved) creds.setToken(shown.id, null);
        this.pairing = { baseUrl, server: shown };
        return;
      }
      // Paired before site keys: swap the old credential for this client's site key, once.
      const { info, token } = await settleCredential(baseUrl, shown, saved, creds, { browser: !isTauri() });
      this.begin(baseUrl, info, info.auth === "local" ? null : token);
    } catch (e) {
      const host = baseUrl.replace(/^https?:\/\//, "");
      const msg =
        e instanceof ApiRequestError && (e.code === "network" || e.code === "timeout")
          ? `Could not reach a server at ${host}. Check that the server is running and the address is right.`
          : e instanceof ApiRequestError && e.status === 404
            ? `${host} answered, but it is not a ${SERVER_NAME} server.`
            : errorText(e);
      if (!quiet || !(e instanceof ApiRequestError)) this.connectError = msg;
    } finally {
      this.connecting = false;
    }
  }

  /**
   * Connects Oriel to another server from the main view (a Codex box prompt's "Connect to
   * it"). The current connection stays until the new one is made: a failure becomes a notice
   * there. When the server needs pairing, Oriel goes to the connect screen for it. `codexBox`:
   * the site's Codex box (server ID), which takes this client's site key; `returnTo`: pair only
   * for the key, then connect to this address again.
   */
  async connectFromMain(address: string, opts: { codexBox?: string; returnTo?: string } = {}): Promise<void> {
    const from = this.conn;
    const site = opts.codexBox ? { serverId: opts.codexBox, token: this.siteKey(opts.codexBox) } : undefined;
    await this.connect(address, false, site);
    if (this.conn !== from) return;
    const pairing = this.pairing;
    if (pairing) {
      this.disconnect();
      this.pairing = { ...pairing, ...(opts.returnTo ? { returnTo: opts.returnTo } : {}) };
      return;
    }
    const err = this.connectError;
    this.connectError = null;
    if (err) from?.pushNotice("error", err);
  }

  /**
   * An Oriel on a show box itself (no key: the box trusts its own machine) pairs with the
   * site's Codex box for the site key its uploads and collection admin need there, then comes
   * back to this box.
   */
  async pairWithCodexBox(box: CodexBox): Promise<void> {
    const name = boxName(box);
    const address = await shell.askText(`Pair with ${name}`, box.addresses[0] ?? "", "Connect", "Address");
    if (!address) return;
    await this.connectFromMain(address, { codexBox: box.serverId, returnTo: this.baseUrl });
  }

  /**
   * This client's site key: the main server's, else the one it got at the site's Codex box
   * (`codexBox`, else the one this box follows). An Oriel on a show box itself connects
   * without a key.
   */
  siteKey(codexBox?: string | null): string | null {
    const own = this.server ? creds.token(this.server.id) : null;
    if (own) return own;
    const box = codexBox ?? siteState.codex?.sync?.serverId ?? null;
    return box ? creds.token(box) : null;
  }

  /** The pairing waiting for approval, if any (cancelled when the user goes back). */
  private pairingWait: WaitControl | null = null;
  /** Its id at the server, to take it back when the user goes back or types the code again. */
  private pendingPairing: { rest: RestClient; id: string } | null = null;

  /** Takes the waiting pairing back at the server (best effort: a failure is ignored; it
   *  ends unanswered there anyway). */
  private withdrawPairing() {
    const p = this.pendingPairing;
    this.pendingPairing = null;
    if (p) void p.rest.withdrawPairing(p.id).catch(() => {});
  }

  /** The name this computer pairs with (the operator's, else the platform and browser). */
  computerName(): string {
    return cleanComputerName(creds.pref(COMPUTER_NAME_PREF) ?? "") || defaultComputerName(globalThis.navigator?.userAgent ?? "", isTauri());
  }

  /** Remembers the name this computer pairs with (empty: the default again). */
  setComputerName(name: string): void {
    creds.setPref(COMPUTER_NAME_PREF, cleanComputerName(name));
  }

  /** Pairs with the code shown on the server, or (`null`) asks a site admin's approval. */
  async pair(code: string | null): Promise<void> {
    const p = this.pairing;
    if (!p) return;
    this.connectError = null;
    this.connecting = true;
    if (this.pairingWait) this.pairingWait.cancelled = true;
    const ctl: WaitControl = { cancelled: false };
    this.pairingWait = ctl;
    try {
      const rest = new RestClient({ baseUrl: p.baseUrl, token: () => null });
      const first = await rest.pair(code?.trim() || null, this.computerName(), !isTauri());
      let res = first;
      if (isPending(first)) {
        this.pendingPairing = { rest, id: first.pairingId };
        this.pairing = { ...p, waiting: true, codeWrong: !!first.codeWrong };
        this.connecting = false;
        const got = await awaitPairing(rest, first, ctl);
        if (!got || ctl.cancelled) return;
        if (this.pendingPairing?.id === first.pairingId) this.pendingPairing = null;
        res = got;
      }
      if (isPending(res)) return;
      // The key is made: keep it even if the operator went back meanwhile (the next connect
      // to this server uses it), but go on to the server only if they are still here.
      creds.setToken(p.server.id, res.token);
      if (ctl.cancelled) return;
      if (p.returnTo) {
        // Paired for the key only: back to the box Oriel came from.
        this.pairing = null;
        await this.connect(p.returnTo);
        return;
      }
      const info = await new RestClient({ baseUrl: p.baseUrl, token: () => res.token }).serverInfo();
      if (ctl.cancelled) return;
      this.pairing = null;
      this.begin(p.baseUrl, info, res.token);
    } catch (e) {
      if (ctl.cancelled) return;
      // (Answered: refused, or ended unanswered: nothing to take back.)
      if (this.pairingWait === ctl) this.pendingPairing = null;
      if (this.pairing?.waiting) this.pairing = { ...this.pairing, waiting: false, codeWrong: false };
      this.connectError =
        e instanceof ApiRequestError && e.code === "unauthorized"
          ? "That code is not right. Check the code shown on the server and try again."
          : (pairingEndedText(e) ?? errorText(e));
    } finally {
      if (this.pairingWait === ctl) this.pairingWait = null;
      this.connecting = false;
    }
  }

  /** Stops waiting for approval and goes back to typing the code (after a wrong one). */
  retryCode() {
    if (this.pairingWait) this.pairingWait.cancelled = true;
    this.pairingWait = null;
    this.withdrawPairing();
    if (this.pairing) this.pairing = { ...this.pairing, waiting: false, codeWrong: false };
    this.connectError = null;
  }

  cancelPairing() {
    const back = this.pairing?.returnTo;
    this.revoked = false;
    if (this.pairingWait) this.pairingWait.cancelled = true;
    this.pairingWait = null;
    this.withdrawPairing();
    this.pairing = null;
    this.connectError = null;
    // Pairing only for the key: back to the box Oriel came from.
    if (back) void this.connect(back);
  }

  disconnect() {
    for (const u of this.unsubs.splice(0)) u();
    this.conn?.stop();
    this.conn = null;
    presCache.use(null);
    this.editor = null;
    shell.reset();
    this.view = "connect";
    this.open = null;
    this.playlist = null;
    mediaState.reset();
  }

  /** Forget this server's token and go back to the connect screen. */
  forgetServer() {
    if (this.server) creds.setToken(this.server.id, null);
    creds.setLastServer(null);
    this.disconnect();
  }

  private begin(baseUrl: string, info: ServerInfo, token: string | null) {
    this.disconnect();
    this.revoked = false;
    this.baseUrl = baseUrl;
    this.server = info;
    creds.setLastServer({ id: info.id, name: info.name, baseUrl });
    const conn = new Connection({ baseUrl, token });
    this.conn = conn;
    const editor = new Editor(conn.rest);
    this.editor = editor;
    // The slide or announcement triggered last (live moves on into announcement items too).
    const latest = new LatestLive();
    this.unsubs.push(
      // The Show page's documents: kept current for the whole connection.
      presCache.attach(conn),
      editor.undo.subscribe((s) => (this.undoState = s)),
      // A reconnect, or events the server dropped for this client (`resync`): read again.
      conn.onSynced((reconnect) => {
        if (reconnect) void this.refreshAll(true);
      }),
      conn.onDocumentChanged((kind, id, msg) => changes.emit({ type: "documentChanged", kind, id, msg })),
      conn.onCodexChanged((m) => changes.emit({ type: "codexChanged", parts: m.parts, collection: m.collection })),
      conn.onSiteChanged((m) => changes.emit({ type: "siteChanged", parts: m.parts })),
      conn.onSiteEvent((m) => changes.emit({ type: "siteEvent", event: m.event })),
      changes.on("documentChanged", (e) => void this.documentChanged(e.kind, e.id, e.msg)),
      siteState.attach(),
      mediaState.attach(),
      showDocsState.attach(),
      timersState.attach(),
      stageState.attach(),
      presence.attach(),
      sourcesState.attach(),
      conn.status.subscribe((s) => {
        // The site removed this computer (its key revoked; the socket closed with 4003).
        if (s.phase === "revoked") queueMicrotask(() => this.keyRevoked(conn));
        // A token revoked while we were offline: the socket can never come back.
        if (s.phase === "reconnecting" && s.attempt > 0 && s.attempt % 4 === 0) void this.checkAuth();
      }),
      conn.show.subscribe((v) => this.followLive(latest.update(v?.state ?? null))),
    );
    conn.start();
    this.view = "main";
    void this.refreshAll(false);
  }

  /** The key was revoked: forget it and go back to the connect screen to pair again. */
  private keyRevoked(conn: Connection) {
    if (this.conn !== conn) return;
    const server = this.server;
    const baseUrl = this.baseUrl;
    if (server) creds.setToken(server.id, null);
    this.disconnect();
    if (server) this.pairing = { baseUrl, server: { ...server, auth: "unpaired" } };
    this.revoked = true;
  }

  private async checkAuth() {
    const conn = this.conn;
    if (!conn) return;
    try {
      const info = await conn.rest.serverInfo();
      if (info.auth === "unpaired" && this.conn === conn) {
        const baseUrl = this.baseUrl;
        this.disconnect();
        this.pairing = { baseUrl, server: info };
        this.connectError = "This computer is no longer paired with the server. Enter the code shown on the server.";
      }
    } catch {
      /* still offline */
    }
  }

  // ---------------------------------------------------------------- documents

  async refreshAll(reconnect: boolean) {
    const conn = this.conn;
    if (!conn) return;
    const r = conn.rest;
    const results = await Promise.allSettled([
      r.libraries(),
      r.playlists(),
      r.looks(),
      r.clearGroups(),
      r.screens(),
      r.serverInfo(),
      r.mediaBins(),
      r.globalGroups(),
    ]);
    if (this.conn !== conn) return;
    const [libs, pls, looks, cgs, screens, info, bins, groups] = results;
    if (groups.status === "fulfilled") this.globalGroups = groups.value;
    if (bins.status === "fulfilled") mediaState.setBins(bins.value);
    if (libs.status === "fulfilled") {
      libraryState.setLibraries(libs.value, true);
      void libraryState.loadLibrary();
    }
    if (pls.status === "fulfilled") {
      this.playlistTree = pls.value;
      const id = this.playlistId;
      // (After a collection switch the old playlist isn't there: not asked for again.)
      if (id && flatPlaylists(pls.value).some((p) => p.id === id)) void this.selectPlaylist(id, true);
      else if (id) {
        this.playlistId = null;
        this.playlist = null;
      }
    }
    if (bins.status === "fulfilled" || pls.status === "fulfilled") mediaState.pickAudioSource();
    void showDocsState.load();
    void timersState.load();
    void stageState.load(true);
    if (looks.status === "fulfilled") this.looks = looks.value;
    if (cgs.status === "fulfilled") this.clearGroups = cgs.value;
    if (screens.status === "fulfilled") {
      screensState.setScreens(screens.value);
    }
    if (info.status === "fulfilled") this.server = info.value;
    if (reconnect) void siteState.loadCodex();
    if (reconnect) void presCache.refreshAll();
    if (reconnect && this.open) void this.reopen();
    if (!reconnect) void this.revealLive();
    const failed = results.filter((x) => x.status === "rejected").length;
    if (failed && conn.socket.isOnline) conn.pushNotice("warning", `Could not load ${failed} list(s) from the server.`);
  }

  /**
   * Runs a `documentChanged` refetch (the core's, or an area module's) with the connection of
   * now. A failure becomes a notice (one per kind) instead of an unhandled rejection; `fn` drops
   * an answer that comes after a switch of server (`session.conn !== conn`).
   */
  async refetch(kind: DocumentKind, fn: (conn: Connection) => Promise<void>) {
    const conn = this.conn;
    if (!conn) return;
    try {
      await fn(conn);
    } catch (e) {
      // Called with `void`: a failed refetch must not become an unhandled rejection.
      if (this.conn === conn && conn.socket.isOnline)
        conn.pushNotice("warning", `Could not reload a changed ${kind} from the server: ${errorText(e)}`, `refetch:${kind}`);
    }
  }

  private documentChanged(kind: DocumentKind, id: string, msg?: DocumentChangedMessage) {
    return this.refetch(kind, (conn) => this.refetchChanged(conn, kind, id, msg));
  }

  /** Presentations this Oriel is deleting (their removal needs no notice). */
  private deleting = new Set<string>();

  /** Says why the open presentation went away when the removal named no client: the box shows
   *  another collection now, else it was deleted (on a show box, the Codex's deletions come
   *  through its mirror without the client). */
  private async removedNotice(conn: Connection, name: string | undefined, collection: string | null) {
    await siteState.loadCodex();
    if (this.conn !== conn) return;
    const now = siteState.codex?.collection ?? null;
    const what = `“${name ?? "The presentation"}”`;
    if (collection && now && now !== collection) conn.pushNotice("info", `${what} was closed: it isn't in the collection this box shows now.`);
    else conn.pushNotice("info", `${what} was deleted by another operator.`);
  }

  /** While this Oriel switches the box's collection: the old collection's libraries,
   * presentations and playlists aren't asked for again (they are gone); `fn`'s own refresh
   * reads the new ones once. */
  private switchingCollection = false;

  async collectionSwitch<T>(fn: () => Promise<T>): Promise<T> {
    this.switchingCollection = true;
    try {
      return await fn();
    } finally {
      this.switchingCollection = false;
    }
  }

  /** `documentChanged`'s refetch of the core's documents (the area modules refetch their own).
   * An answer that comes after a switch of server is dropped. */
  private async refetchChanged(conn: Connection, kind: DocumentKind, id: string, msg?: DocumentChangedMessage) {
    const fresh = async <T,>(p: Promise<T>): Promise<T | undefined> => {
      const v = await p;
      return this.conn === conn ? v : undefined;
    };
    switch (kind) {
      case "presentation":
        if (msg?.change === "removed") {
          const wasOpen = this.open?.presentation.id === id;
          const name = this.open?.presentation.name;
          this.forgetPresentation(id);
          // An edit names its client. A removal without one is the box showing another
          // collection now, or (on a show box) a deletion the Codex mirror passed on.
          if (wasOpen && !this.deleting.has(id)) {
            if (msg.clientId && msg.clientId !== this.editor?.clientId)
              conn.pushNotice("info", `“${name ?? "The presentation"}” was deleted by another operator.`);
            else if (!msg.clientId) void this.removedNotice(conn, name, msg.collection ?? null);
          }
        } else if (this.open?.presentation.id === id) void this.reopen();
        if (this.switchingCollection) break;
        void libraryState.loadLibrary();
        if (this.playlistId) void this.selectPlaylist(this.playlistId, true);
        break;
      case "library": {
        if (this.switchingCollection) break;
        const libs = await fresh(conn.rest.libraries());
        if (!libs) return;
        libraryState.libraries = libs;
        if (libraryState.libraryId && !libraryState.libraries.some((l) => l.id === libraryState.libraryId)) libraryState.libraryId = libraryState.libraries[0]?.id ?? null;
        void libraryState.loadLibrary();
        break;
      }
      case "playlistNode": {
        if (this.switchingCollection) break;
        const tree = await fresh(conn.rest.playlists());
        if (!tree) return;
        this.playlistTree = tree;
        if (this.playlistId) void this.selectPlaylist(this.playlistId, true);
        mediaState.playlistsChanged(id);
        break;
      }
      case "globalGroup": {
        const groups = await fresh(conn.rest.globalGroups());
        if (groups) this.globalGroups = groups;
        break;
      }
      case "look": {
        const looks = await fresh(conn.rest.looks());
        if (looks) this.looks = looks;
        break;
      }
      case "clearGroup": {
        const cgs = await fresh(conn.rest.clearGroups());
        if (cgs) this.clearGroups = cgs;
        break;
      }
      case "screen": {
        const screens = await fresh(conn.rest.screens());
        if (screens) screensState.setScreens(screens);
        break;
      }
      // timer, message, prop, theme, stageLayout, mediaBin, mediaAsset: their area modules.
    }
  }

  // ---------------------------------------------------------------- props and stage layouts

  /** Shows a prop or stage layout in the Edit view as a one-slide document (prop-document
   * mode); what was open comes back when editing ends (`closeProp`). */
  showOneSlide(p: Presentation, stageLayout = false, theme = false) {
    if (!this.open?.prop) this.propBefore = this.open;
    this.openSeq++;
    this.open = { presentation: p, slides: flattenSlides(p), playlistItem: null, prop: true, ...(stageLayout ? { stageLayout: true } : {}), ...(theme ? { theme: true } : {}) };
    shell.go("edit");
  }

  /** A prop or stage layout changed (after its list was reloaded): the one open in the Edit
   * view is reloaded, or closed when it was removed. */
  oneSlideChanged(id: string, msg?: DocumentChangedMessage) {
    if (!this.open?.prop || this.open.presentation.id !== id) return;
    if (msg?.change === "removed") this.closeProp();
    else void this.reopen();
  }

  /** Ends prop editing: the presentation that was open comes back. */
  closeProp() {
    if (!this.open?.prop) return;
    const before = this.propBefore;
    this.propBefore = null;
    this.open = before;
    if (shell.route === "edit") shell.go("show");
    if (before) void this.reopen();
  }

  // ---------------------------------------------------------------- playlists and presentations

  async selectPlaylist(id: string, keepOpen = false) {
    const conn = this.conn;
    if (!conn) return;
    if (!keepOpen) showView.listSource = "playlist";
    this.playlistId = id;
    try {
      const p = await conn.rest.playlist(id);
      if (this.playlistId === id) this.playlist = p;
    } catch (e) {
      if (e instanceof ApiRequestError && e.code === "notFound") {
        this.playlistId = null;
        this.playlist = null;
      } else conn.pushNotice("error", `Could not load the playlist: ${errorText(e)}`);
    }
  }

  /** Opens a presentation in the grid (does not trigger anything). */
  async openPresentation(id: string, playlistItem: PlaylistItemRef | null = null, quiet = false) {
    const conn = this.conn;
    if (!conn) return;
    const seq = ++this.openSeq;
    if (this.open?.prop) this.propBefore = null;
    showView.selectedPresentationId = playlistItem ? null : id;
    showView.selectedItemId = playlistItem?.item ?? null;
    if (this.open?.presentation.id === id) {
      this.open = { ...this.open, playlistItem };
      return;
    }
    this.openLoading = true;
    try {
      const p = await conn.rest.presentation(id);
      if (seq !== this.openSeq) return;
      this.open = { presentation: p, slides: flattenSlides(p), playlistItem };
    } catch (e) {
      if (seq !== this.openSeq) return;
      const f = openFailure(e, quiet);
      if (f.missing) this.forgetPresentation(id);
      if (f.notice) conn.pushNotice(f.notice.level, f.notice.message);
    } finally {
      if (seq === this.openSeq) this.openLoading = false;
    }
  }

  /** A presentation is gone: drop the selection and the open view that point at it. */
  private forgetPresentation(id: string) {
    if (showView.selectedPresentationId === id) showView.selectedPresentationId = null;
    const item = this.playlist?.items.find((i) => i.id === showView.selectedItemId);
    if (item?.kind.type === "presentation" && item.kind.presentation.id === id) showView.selectedItemId = null;
    if (this.open?.presentation.id === id) {
      this.open = null;
      if (shell.route === "edit" || shell.route === "reflow") shell.go("show");
    }
  }

  /** Refetches the open presentation (after an edit). */
  async reopen() {
    const conn = this.conn;
    const cur = this.open;
    if (!conn || !cur) return;
    // Overlapping refetches (an edit's, then its documentChanged's): the last one asked for
    // wins, even when an earlier one answers after it.
    const seq = ++this.reopenSeq;
    try {
      const p = await conn.rest.presentation(cur.presentation.id);
      if (seq !== this.reopenSeq || this.conn !== conn) return;
      if (this.open?.presentation.id === p.id) this.open = { ...this.open, presentation: p, slides: flattenSlides(p) };
    } catch (e) {
      // Deleted meanwhile: let go of it quietly. Offline: keep what we have.
      if (openFailure(e, true).missing) this.forgetPresentation(cur.presentation.id);
    }
  }

  /**
   * On startup, show what is live: open its presentation and, when it came from a playlist,
   * open that playlist (and the folders around it) in the sidebar.
   */
  private async revealLive() {
    const conn = this.conn;
    if (!conn || this.open) return;
    if (!conn.show.current) await new Promise<void>((r) => setTimeout(r, 300));
    const slide = new LatestLive().update(conn.show.current?.state ?? null);
    if (!slide || this.open || this.conn !== conn) return;
    if (slide.playlist) {
      const path = findPath(this.playlistTree, slide.playlist.playlist);
      for (const id of path.slice(0, -1)) showView.expandedFolders[id] = true;
      if (path.length) {
        showView.listSource = "playlist";
        await this.selectPlaylist(slide.playlist.playlist, true);
      }
    }
    // Quiet: the live slide may belong to a presentation deleted while it was on screen.
    await this.openPresentation(slide.presentation, slide.playlist, true);
  }

  private lastLivePresentation: string | null = null;

  /**
   * When the live slide moves on to another presentation of the playlist the grid shows
   * (next past the last slide), follow it, as operators expect.
   */
  private followLive(slide: { presentation: string; playlist: PlaylistItemRef | null } | null) {
    const prev = this.lastLivePresentation;
    this.lastLivePresentation = slide?.presentation ?? null;
    if (!slide || slide.presentation === prev) return;
    // Not while the operator edits it (Edit, Reflow, a song in Edit on the Library page): the
    // editor follows the open presentation and would jump to the next song mid-typing.
    if (shell.route === "edit" || shell.route === "reflow" || shell.pageEditing) return;
    const open = this.open;
    if (!open || !slide.playlist || open.presentation.id !== prev) return;
    if (open.playlistItem?.playlist !== slide.playlist.playlist) return;
    if (open.presentation.id !== slide.presentation) void this.openPresentation(slide.presentation, slide.playlist);
  }

  // ---------------------------------------------------------------- commands

  /** Sends a command; failures become a toast instead of an exception. */
  async run(c: Command): Promise<boolean> {
    const conn = this.conn;
    if (!conn) return false;
    try {
      await conn.command(c);
      return true;
    } catch (e) {
      if (e instanceof CommandError && e.code === "conflict" && (c.type === "next" || c.type === "prev")) {
        conn.pushNotice("info", c.type === "next" ? "Nothing to go to next. Click a slide first." : "Nothing before this slide.");
      } else if (e instanceof CommandError && e.code === "notImplemented") {
        conn.pushNotice("info", "This server does not support that yet.");
      } else {
        conn.pushNotice("error", errorText(e));
      }
      return false;
    }
  }

  triggerSlide(s: GridSlide) {
    const open = this.open;
    if (!open) return;
    return this.run({
      type: "triggerSlide",
      presentation: open.presentation.id,
      slide: s.id,
      ...(open.playlistItem ? { playlistItem: open.playlistItem } : {}),
    });
  }

  /** Switches audience or stage screens on or off (toolbar toggles, Ctrl+1 / Ctrl+2). */
  toggleScreens(role: "audience" | "stage") {
    const on = this.showState?.screensEnabled?.[role] ?? true;
    return this.run({ type: "setScreensEnabled", role, enabled: !on });
  }

  // ---------------------------------------------------------------- editing

  /** Runs an edit; a failure becomes a toast. */
  async edit<T>(what: string, fn: (e: Editor) => Promise<T>): Promise<T | null> {
    const e = this.editor;
    const conn = this.conn;
    if (!e || !conn) return null;
    try {
      return await fn(e);
    } catch (err) {
      conn.pushNotice("error", `${what} failed: ${errorText(err)}`);
      return null;
    }
  }

  async undo() {
    await this.reflowFlush?.().catch(() => {});
    const conn = this.conn;
    const e = this.editor;
    if (!e || !conn) return;
    try {
      await e.undo.undo();
    } catch (err) {
      conn.pushNotice("error", `Could not undo: ${errorText(err)}`);
    }
  }

  async redo() {
    await this.reflowFlush?.().catch(() => {});
    const conn = this.conn;
    const e = this.editor;
    if (!e || !conn) return;
    try {
      await e.undo.redo();
    } catch (err) {
      conn.pushNotice("error", `Could not redo: ${errorText(err)}`);
    }
  }

  /** File ▸ New Presentation: asks for a name, creates it in the selected library, opens Reflow. */
  async newPresentation() {
    if (!shell.closeDialogs()) return;
    const name = await shell.askText("New Presentation", "Untitled", "Create");
    if (!name) return;
    let library = libraryState.libraryId ?? libraryState.libraries[0]?.id ?? null;
    if (!library) {
      const l = await this.edit("New library", (e) => e.createLibrary("Library"));
      library = l?.id ?? null;
    }
    const r = await this.edit("New presentation", (e) => e.createPresentation(name, library));
    if (!r) return;
    if (library) await libraryState.selectLibrary(library);
    await this.openPresentation(r.id);
    shell.setMode("reflow");
  }

  async newLibrary() {
    shell.closeDialogs();
    const r = await this.edit("New library", (e) => e.createLibrary("New Library"));
    if (!r) return;
    libraryState.libraries = (await this.conn?.rest.libraries()) ?? libraryState.libraries;
    await libraryState.selectLibrary(r.id);
    this.renaming = { kind: "library", id: r.id };
  }

  async newPlaylist(folder = false, parent: string | null = null) {
    shell.closeDialogs();
    const r = await this.edit(folder ? "New folder" : "New playlist", (e) =>
      e.createPlaylist(folder ? "New Folder" : "New Playlist", folder, parent),
    );
    if (!r) return;
    this.playlistTree = (await this.conn?.rest.playlists()) ?? this.playlistTree;
    if (parent) showView.expandedFolders[parent] = true;
    if (!folder) await this.selectPlaylist(r.id);
    this.renaming = { kind: "playlist", id: r.id };
  }

  /** Commits an inline rename. */
  async rename(r: Renaming, from: string, to: string) {
    this.renaming = null;
    const name = to.trim();
    if (!name || name === from) return;
    switch (r.kind) {
      case "library":
        await this.edit("Rename", (e) => e.renameLibrary(r.id, from, name));
        break;
      case "presentation":
        await this.edit("Rename", (e) => e.renamePresentation(r.id, name));
        break;
      case "playlist":
        await this.edit("Rename", (e) => e.renamePlaylist(r.id, from, name));
        break;
      case "item": {
        const pl = this.playlist;
        const it = pl?.items.find((i) => i.id === r.id);
        if (!pl || !it) return;
        await this.edit("Rename", (e) => e.editPlaylist(pl.id, "Rename", [{ type: "setItem", item: it.id, name }]));
        break;
      }
    }
  }

  async deleteLibrary(id: string) {
    const l = libraryState.libraries.find((x) => x.id === id);
    if (!l) return;
    if (l.presentationCount > 0) {
      this.conn?.pushNotice("info", `“${l.name}” still holds ${l.presentationCount} presentation(s). Delete or move them first.`);
      return;
    }
    if (!(await shell.confirm("Delete Library", `Delete the library “${l.name}”?`))) return;
    await this.edit("Delete library", (e) => e.deleteLibrary(id));
  }

  async deletePresentation(id: string, name: string) {
    if (!(await shell.confirm("Delete Presentation", `Delete “${name}”? Playlist items that use it will show it as missing.`))) return;
    this.deleting.add(id);
    const ok = await this.edit("Delete presentation", (e) => e.deletePresentation(id)).finally(() => setTimeout(() => this.deleting.delete(id), 5000));
    if (ok !== null && this.open?.presentation.id === id) {
      this.open = null;
      shell.setMode("show");
    }
  }

  async movePresentation(id: string, from: string | null, to: string) {
    await this.edit("Move", (e) => e.movePresentation(id, from, to));
  }

  async duplicatePresentation(id: string) {
    await this.edit("Duplicate", (e) => e.duplicatePresentation(id));
  }

  async deletePlaylist(id: string, name: string, folder: boolean) {
    const msg = folder ? `Delete the folder “${name}” and everything in it?` : `Delete the playlist “${name}”?`;
    if (!(await shell.confirm(folder ? "Delete Folder" : "Delete Playlist", msg))) return;
    await this.edit("Delete", (e) => e.deletePlaylist(id, folder));
    if (this.playlistId === id) {
      this.playlistId = null;
      this.playlist = null;
    }
  }

  async duplicatePlaylist(id: string) {
    await this.edit("Duplicate", (e) => e.duplicatePlaylist(id));
  }

  /** Adds a presentation to a playlist after `after` (undefined = at the end). */
  async addToPlaylist(playlistId: string, p: { id: string; name: string }, after?: string | null) {
    let at = after;
    if (at === undefined) {
      const items = playlistId === this.playlist?.id ? this.playlist.items : (await this.conn?.rest.playlist(playlistId))?.items;
      at = items?.[items.length - 1]?.id ?? null;
    }
    const item: PlaylistItem = { id: uuid(), kind: { type: "presentation", presentation: { id: p.id, name: p.name } } };
    await this.edit("Add to playlist", (e) =>
      e.editPlaylist(playlistId, "Add to Playlist", [{ type: "addItem", item, ...(at ? { after: at } : {}) }]),
    );
  }

  async addHeader(playlistId: string, after?: string | null) {
    const h = await shell.askHeader("Add Header", "Header");
    if (!h) return;
    let at = after;
    if (at === undefined) at = this.playlist?.items[this.playlist.items.length - 1]?.id ?? null;
    const item: PlaylistItem = { id: uuid(), kind: { type: "header", name: h.name, color: h.color } };
    await this.edit("Add header", (e) => e.editPlaylist(playlistId, "Add Header", [{ type: "addItem", item, ...(at ? { after: at } : {}) }]));
  }

  async editHeader(it: PlaylistItemDetail) {
    const pl = this.playlist;
    if (!pl || it.kind.type !== "header") return;
    const h = await shell.askHeader("Edit Header", it.kind.name ?? "", it.kind.color ?? undefined);
    if (!h) return;
    await this.edit("Edit header", (e) => e.editPlaylist(pl.id, "Edit Header", [{ type: "setItem", item: it.id, name: h.name, color: h.color }]));
  }

  async moveItem(itemId: string, after: string | null) {
    const pl = this.playlist;
    if (!pl || itemId === after) return;
    await this.edit("Move", (e) => e.editPlaylist(pl.id, "Move", [{ type: "moveItem", item: itemId, ...(after ? { after } : {}) }]));
  }

  async removeItem(it: PlaylistItemDetail) {
    const pl = this.playlist;
    if (!pl) return;
    if (!(await shell.confirm("Remove Item", `Remove “${it.displayName}” from “${pl.name}”?`, "Remove"))) return;
    await this.edit("Remove", (e) => e.editPlaylist(pl.id, "Remove from Playlist", [{ type: "removeItem", item: it.id }]));
  }

  async duplicateItem(it: PlaylistItemDetail) {
    const pl = this.playlist;
    if (!pl) return;
    const item: PlaylistItem = { id: uuid(), kind: it.kind, enabled: it.enabled };
    await this.edit("Duplicate", (e) => e.editPlaylist(pl.id, "Duplicate", [{ type: "addItem", after: it.id, item }]));
  }

  /** Shows a playlist item's presentation in its library. */
  async revealInLibrary(presentationId: string) {
    const p = (await this.conn?.rest.presentations())?.find((x) => x.id === presentationId);
    if (!p?.library) {
      this.conn?.pushNotice("info", "That presentation is not in a library.");
      return;
    }
    await libraryState.selectLibrary(p.library);
    showView.selectedPresentationId = p.id;
    void this.openPresentation(p.id);
  }

  /** The show state right now (no subscription). */
  get showState() {
    return this.conn ? get(this.conn.show)?.state ?? null : null;
  }
}

export const session = new Session();
