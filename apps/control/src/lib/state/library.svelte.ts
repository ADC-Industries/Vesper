// The library's state (Svelte 5 runes): the libraries (shelves) of this box's collection, the
// selected one and its presentations, and the search text (the Show page's left column); and the
// Library page's own state (`libraryPage`): its view of the tree, the active collection's songs,
// media, themes, props and trash, the search, the collection switch and the side connection to
// the Codex box; and round 4's managing and editing: uploads, missing media and relinking, the
// storage line, the all-collections view with the collection trash, and a song in Edit.

import type { LibrarySummary, PresentationSummary } from "$protocol";
import type { CodexBox, CollectionMedia, CollectionSummary, PlaylistTreeNode, PropEntry, SearchHit, ThemeSummary, TrashItem } from "$protocol";
import type { CodexStorage, Color, DeletedCollection, MediaMatchReport, PlaylistItemRef, RelinkTarget } from "$protocol";
import { CodexClient, codexFailure, forbiddenText, putUploadChunk } from "../server/codex";
import { ApiRequestError, type RestClient } from "../server/rest";
import { EditState } from "../edit/editState.svelte";
import { UploadQueue, type Hasher, type UploadRequest, type UploadTransport, type UploadView } from "../pages/library/upload";
import { hashBlob } from "../pages/library/sha256";
import { CodexBoxError, CodexBoxLink, boxName } from "../server/codexBox";
import type { SortState } from "../ui";
import { changes } from "./events";
import { shell } from "./shell.svelte";
import { sitePage, siteState } from "./site.svelte";
import { liveSummary, matchReportLines, relinkReportLines } from "../pages/library/logic";
import { errorText } from "./errors";
import { creds } from "./prefs";
import { session } from "./session.svelte";
import { showView } from "./show.svelte";

class LibraryState {
  libraries = $state.raw<LibrarySummary[]>([]);
  libraryId = $state<string | null>(null);
  libraryItems = $state.raw<PresentationSummary[]>([]);
  libraryLoading = $state(false);
  search = $state("");
  /** Bumped to move the focus to the Library page's search field (Ctrl+F). */
  searchFocus = $state(0);

  async selectLibrary(id: string) {
    showView.listSource = "library";
    this.libraryId = id;
    creds.setPref("library", id);
    await this.loadLibrary();
  }

  /** Takes a fresh list of libraries and keeps (or picks) the selected one. */
  setLibraries(libs: LibrarySummary[], pickSaved: boolean) {
    this.libraries = libs;
    if (!this.libraryId || !libs.some((l) => l.id === this.libraryId)) {
      const saved = pickSaved ? creds.pref("library") : null;
      this.libraryId = libs.find((l) => l.id === saved)?.id ?? libs[0]?.id ?? null;
    }
  }

  async loadLibrary() {
    const conn = session.conn;
    const id = this.libraryId;
    if (!conn || !id) {
      this.libraryItems = [];
      return;
    }
    this.libraryLoading = true;
    try {
      const items = await conn.rest.libraryPresentations(id);
      if (this.libraryId === id) this.libraryItems = items;
    } catch (e) {
      if (this.libraryId !== id) return;
      if (e instanceof ApiRequestError && e.status === 404) {
        // Gone: deleted, or the box shows another collection now (its documents' change events
        // come before the library list's). Pick one from the list as it is now.
        this.libraryItems = [];
        const libs = await conn.rest.libraries().catch(() => null);
        if (!libs || session.conn !== conn || this.libraryId !== id) return;
        this.setLibraries(libs, false);
        if (this.libraryId !== id) void this.loadLibrary();
        return;
      }
      conn.pushNotice("error", `Could not load the library: ${errorText(e)}`);
    } finally {
      this.libraryLoading = false;
    }
  }

  reset() {
    this.libraryItems = [];
  }
}

export const libraryState = new LibraryState();

// ---------------------------------------------------------------- the Library page (Codex)

/** Where in the Library tree the operator is. */
export type LibraryView =
  | { kind: "songs"; shelf: string | null }
  | { kind: "media"; media: "image" | "video" }
  /** A media bin, or a media playlist (bin playlists), and the global groups (G17). */
  | { kind: "bin"; id: string }
  | { kind: "mediaPlaylist"; id: string }
  | { kind: "groups" }
  | { kind: "themes" }
  | { kind: "props" }
  | { kind: "trash" }
  /** Every collection of the Codex and the collection trash ("Manage collections"). */
  | { kind: "collections" };

/** The search scope: this collection, or every collection of the Codex. */
export type SearchScope = "collection" | "all";

/** Playlists (not folders) of the tree, in order, with their folder path for the picker. */
export function flatPlaylists(tree: PlaylistTreeNode[], path: string[] = []): { id: string; name: string; path: string[] }[] {
  const out: { id: string; name: string; path: string[] }[] = [];
  for (const n of tree) {
    if (n.kind === "playlist") out.push({ id: n.id, name: n.name, path });
    if (n.children?.length) out.push(...flatPlaylists(n.children, [...path, n.name]));
  }
  return out;
}

class LibraryPageState {
  view = $state<LibraryView>({ kind: "songs", shelf: null });
  /** The collections card's list is open. */
  collectionsOpen = $state(false);
  collections = $state.raw<CollectionSummary[]>([]);
  /** Every presentation of the active collection (the songs table filters it by shelf). */
  songs = $state.raw<PresentationSummary[]>([]);
  media = $state.raw<CollectionMedia[]>([]);
  themes = $state.raw<ThemeSummary[]>([]);
  /** Bumped when the themes are read again (their template pictures are asked for again). */
  themesVersion = $state(0);
  props = $state.raw<PropEntry[]>([]);
  trash = $state.raw<TrashItem[]>([]);
  loading = $state(false);
  /** The collection the lists above belong to. */
  loadedCollection = $state<string | null>(null);
  /** The filter / search text and its scope. */
  query = $state("");
  scope = $state<SearchScope>("collection");
  /** The server's search answer for `query` (null: none yet, or no query). */
  hits = $state.raw<SearchHit[] | null>(null);
  searchError = $state<string | null>(null);
  sort = $state<SortState | null>({ key: "title", dir: "asc" });
  /** The selected item's ID in the current view. */
  selected = $state<string | null>(null);
  /** The playlist "Add to" uses (the page toolbar's picker). */
  target = $state<string | null>(null);
  /** The Codex box used for management requests (the "Managed on *name*" note). */
  managedOn = $state.raw<CodexBox | null>(null);
  busy = $state(false);

  // ---- round 4
  /** Media: every file, or only the missing ones. */
  mediaFilter = $state<"all" | "missing">("all");
  storage = $state.raw<CodexStorage | null>(null);
  deletedCollections = $state.raw<DeletedCollection[]>([]);
  uploads = $state.raw<UploadView[]>([]);
  /** The manual media bin new uploads go into (null: none). */
  importBin = $state<string | null>(null);
  /** The missing asset the relink dialog is open for. */
  relinkFor = $state.raw<CollectionMedia | null>(null);
  /** The last "Find missing files" / folder match answer, shown until closed. */
  relinkReport = $state.raw<{ title: string; lines: string[] } | null>(null);
  /** The song's View | Edit switch. */
  mode = $state<"view" | "edit">("view");
  /** The editor's state while a song is in Edit (the slide editor's components share it). */
  editState = $state.raw<EditState | null>(null);
  /** The where column, folded into an icon rail while editing, is open on hover. */
  railOpen = $state(false);
  /** What Show had open before a song was opened in Edit (given back afterwards). */
  private openBefore: { id: string | null; playlistItem: PlaylistItemRef | null; selP: string | null; selI: string | null } | null = null;
  private queue: UploadQueue | null = null;

  private link: CodexBoxLink | null = null;
  private linkFor: unknown = null;
  private searchSeq = 0;

  /** The Codex client of the main connection. */
  private get codex(): CodexClient | null {
    const conn = session.conn;
    return conn ? new CodexClient(conn.rest) : null;
  }

  /** The side connection for this main connection (made on first use). */
  private sideLink(): CodexBoxLink {
    if (this.link && this.linkFor === session.conn) return this.link;
    this.link?.reset();
    const link = new CodexBoxLink({
      token: (box) => session.siteKey(box?.serverId),
      clientId: () => session.editor?.clientId ?? null,
      prompts: {
        ask: (box) => shell.confirm("Codex box", `Uploads and collections are managed on ${boxName(box)}. Use it for this?`, `Use ${boxName(box)}`, false),
        unpaired: async (box) => {
          const go = await shell.confirm(
            "Codex box",
            `Uploads and collections are managed on ${boxName(box)}, and this computer isn't paired with it (Oriel reaches this box as the box itself). Pair it with ${boxName(box)} now? Oriel comes back to this box afterwards.`,
            `Pair with ${boxName(box)}`,
            false,
          );
          if (go) await session.pairWithCodexBox(box);
        },
        unreachable: async (box) => {
          const where = box.addresses.length ? box.addresses.map((a) => a.replace(/^https?:\/\//, "")).join(", ") : "no address known yet";
          const go = await shell.confirm(
            "Codex box unreachable",
            `${boxName(box)} didn't answer (${where}). Connect Oriel to it instead? The show on this box keeps playing.`,
            "Connect to it",
            false,
          );
          if (!go) return;
          const address = await shell.askText(`Connect to ${boxName(box)}`, box.addresses[0] ?? "", "Connect", "Address");
          if (address) await session.connectFromMain(address, { codexBox: box.serverId });
        },
      },
    });
    link.subscribe((b) => (this.managedOn = b));
    this.link = link;
    this.linkFor = session.conn;
    return link;
  }

  /** Called by the page while it is shown: loads, and follows the change events. */
  attach(): () => void {
    void this.load();
    const offs = [
      changes.on("codexChanged", (e) => {
        if (e.parts.includes("collections")) void this.loadCollections().then(() => this.loadDeletedCollections());
        else if (e.parts.includes("collectionTrash")) void this.loadDeletedCollections();
        if (e.parts.includes("trash")) void this.loadTrash();
        if (e.parts.includes("media") && (!e.collection || e.collection === this.loadedCollection)) void this.loadMedia();
        if (e.parts.includes("status")) void siteState.loadCodex().then(() => this.followCollection());
      }),
      changes.on("documentChanged", (e) => {
        if (e.msg.collection && this.loadedCollection && e.msg.collection !== this.loadedCollection) return;
        if (e.kind === "presentation" || e.kind === "library") void session.refetch(e.kind, () => this.loadSongs());
        else if (e.kind === "theme") void session.refetch(e.kind, () => this.loadThemes());
        else if (e.kind === "prop") void session.refetch(e.kind, () => this.loadProps());
        else if (e.kind === "mediaAsset") void session.refetch(e.kind, () => this.loadMedia());
      }),
      // Events sent while the socket was down (or dropped for a slow client) are missed: read
      // the page's lists again.
      session.conn?.onSynced((reconnect) => reconnect && void this.load()) ?? (() => {}),
    ];
    return () => offs.forEach((f) => f());
  }

  /** Reloads everything when the box shows another collection than the lists. */
  followCollection() {
    const c = siteState.codex?.collection ?? null;
    // Another Oriel switched the box's collection: a song in Edit belongs to the old one.
    if (c && c !== this.loadedCollection) void this.endEdit().then(() => this.load());
  }

  async load() {
    const conn = session.conn;
    if (!conn) return;
    this.loading = true;
    if (!siteState.codex) await siteState.loadCodex();
    this.loadedCollection = siteState.codex?.collection ?? null;
    await Promise.allSettled([
      this.loadCollections(),
      this.loadSongs(),
      this.loadMedia(),
      this.loadThemes(),
      this.loadProps(),
      this.loadTrash(),
      this.loadStorage(),
      this.loadDeletedCollections(),
    ]);
    if (session.conn === conn) this.loading = false;
    if (!this.target || !flatPlaylists(session.playlistTree).some((p) => p.id === this.target)) {
      this.target = session.playlistId ?? flatPlaylists(session.playlistTree)[0]?.id ?? null;
    }
    if (this.query.trim()) void this.search();
  }

  private async quiet<T>(what: string, p: Promise<T> | undefined): Promise<T | undefined> {
    const conn = session.conn;
    if (!p) return undefined;
    try {
      const v = await p;
      return session.conn === conn ? v : undefined;
    } catch (e) {
      if (session.conn === conn) conn?.pushNotice("warning", `Could not load ${what}: ${errorText(e)}`, `library:${what}`);
      return undefined;
    }
  }

  async loadCollections() {
    const v = await this.quiet("the collections", this.codex?.collections());
    if (v) this.collections = v;
  }
  async loadSongs() {
    const v = await this.quiet("the songs", session.conn?.rest.presentations());
    if (v) this.songs = v;
  }
  async loadMedia() {
    const c = this.loadedCollection;
    if (!c) return;
    const v = await this.quiet("the media", this.codex?.media(c));
    if (v && this.loadedCollection === c) this.media = v;
  }
  async loadThemes() {
    const v = await this.quiet("the themes", session.conn?.rest.themes());
    if (v) {
      this.themes = v;
      this.themesVersion++;
    }
  }
  async loadProps() {
    const v = await this.quiet("the props", session.conn?.rest.props());
    if (v) this.props = v;
  }
  async loadTrash() {
    const v = await this.quiet("Recently deleted", this.codex?.trash());
    if (v) this.trash = v;
  }

  async loadStorage() {
    const v = await this.quiet("the storage figures", this.codex?.storage());
    if (v) this.storage = v;
  }
  async loadDeletedCollections() {
    const v = await this.quiet("the collection trash", this.codex?.collectionTrash());
    if (v) this.deletedCollections = v;
  }

  go(view: LibraryView) {
    if (this.mode === "edit") void this.endEdit();
    this.view = view;
    this.selected = null;
    this.hits = null;
    this.railOpen = false;
  }

  /** Selects an item (leaving Edit when it was another song). */
  select(id: string | null) {
    if (this.mode === "edit" && id !== this.selected) void this.endEdit();
    this.selected = id;
  }

  /** Runs the Codex search for `query` (debounced by the caller); clears it when empty. */
  async search() {
    const q = this.query.trim();
    const my = ++this.searchSeq;
    this.searchError = null;
    if (!q) {
      this.hits = null;
      return;
    }
    const kinds = this.view.kind === "themes" ? ["theme" as const] : this.view.kind === "props" ? ["prop" as const] : this.view.kind === "media" ? ["mediaAsset" as const] : ["presentation" as const];
    try {
      const r = await this.codex?.search(q, { kinds, limit: 200, ...(this.scope === "all" ? { collection: "all" } : {}) });
      if (my === this.searchSeq && r) this.hits = r.hits;
    } catch (e) {
      if (my === this.searchSeq) {
        this.hits = [];
        this.searchError = errorText(e);
      }
    }
  }

  /** Shows another collection on this box, asking first while something is live. */
  async switchCollection(c: CollectionSummary) {
    const codex = this.codex;
    if (!codex || c.active) {
      this.collectionsOpen = false;
      return;
    }
    const name = c.collection.name ?? "the collection";
    // A song in Edit: done first (its typing saved to the collection it belongs to; the page
    // leaves the half-edit state, and Show's presentation comes back).
    await this.endEdit();
    this.busy = true;
    try {
      await session.collectionSwitch(async () => {
        try {
          await codex.assign(c.collection.id);
        } catch (e) {
          const f = codexFailure(e);
          if (f.kind !== "liveShow") throw e;
          const what = f.live?.length ? f.live.join(", ") : (liveSummary(session.showState) ?? f.message);
          const ok = await shell.confirm(`Switch to ${name}`, `Something is live on this box (${what}). Switch anyway?`, "Switch anyway", false);
          if (!ok) return;
          await codex.assign(c.collection.id, true);
        }
        this.collectionsOpen = false;
        this.view = { kind: "songs", shelf: null };
        this.selected = null;
        await siteState.loadCodex();
        await Promise.all([session.refreshAll(true), this.load()]);
      });
    } catch (e) {
      this.fail(e, `Switching to ${name}`);
    } finally {
      this.busy = false;
    }
  }

  /** Restores a deleted document into its collection. */
  async restore(item: TrashItem) {
    const codex = this.codex;
    if (!codex) return;
    try {
      await codex.restoreTrash(item.id);
      session.conn?.pushNotice("info", `“${item.name}” is back.`, `restore:${item.id}`);
      await Promise.all([this.loadTrash(), this.loadSongs(), this.loadProps()]);
    } catch (e) {
      this.fail(e, `Restoring “${item.name}”`);
    }
  }

  /** Empties Recently deleted for good (admins; on a show box through the Codex box). */
  async emptyTrash() {
    const conn = session.conn;
    if (!conn) return;
    const n = this.trash.length;
    if (!(await shell.confirm("Empty Recently deleted", `Delete ${n === 1 ? "this item" : `these ${n} items`} for good? This can't be undone.`, "Delete for good", true))) return;
    try {
      const r = await this.sideLink().run(conn.rest, (rest) => new CodexClient(rest).emptyTrash());
      conn.pushNotice("info", `${r.purged === 1 ? "1 item" : `${r.purged} items`} deleted for good.`, "trash:purge");
      await this.loadTrash();
    } catch (e) {
      if (e instanceof CodexBoxError && e.reason !== "unknown") return;
      this.fail(e, "Emptying Recently deleted");
    }
  }

  /** Adds a presentation to the toolbar's playlist. */
  async addToPlaylist(p: { id: string; name: string }) {
    const target = flatPlaylists(session.playlistTree).find((x) => x.id === this.target);
    if (!target) {
      session.conn?.pushNotice("info", "Pick a playlist in “Add to” first.", "library:add");
      return;
    }
    await session.addToPlaylist(target.id, p);
    session.conn?.pushNotice("info", `Added “${p.name}” to ${target.name}.`, "library:add");
  }

  // ---------------------------------------------------------------- uploads

  /** The site key for the chunk uploads (the same on every server of the site). */
  private auth() {
    return { token: () => session.siteKey(this.managedOn?.serverId), clientId: session.editor?.clientId ?? null };
  }

  /** Runs `fn` where uploads go (this box, or on a show box the Codex box, asked once) with the
   *  site key for raw-body requests there (the PowerPoint import, `state/slidesImport`). */
  onCodexBox<T>(fn: (rest: RestClient, auth: { token: () => string | null; clientId: string | null }) => Promise<T>): Promise<T> {
    const conn = session.conn;
    if (!conn) return Promise.reject(new Error("Not connected"));
    return this.sideLink().run(conn.rest, (rest) => fn(rest, this.auth()));
  }

  /** One upload's transport: created on the main server or (a show box) on the Codex box. */
  private uploadTransport(): UploadTransport {
    let used: RestClient | null = null;
    const on = () => {
      if (!used) throw new Error("The upload hasn't started");
      return new CodexClient(used);
    };
    return {
      create: async (req) => {
        const conn = session.conn;
        if (!conn) throw new Error("Not connected");
        return this.sideLink().run(conn.rest, async (rest) => {
          const info = await new CodexClient(rest).createUpload(req);
          used = rest;
          return info;
        });
      },
      status: (id) => on().uploadInfo(id),
      put: (id, chunk, start, size, signal) => putUploadChunk(on().rest, this.auth(), id, chunk, start, size, signal),
      finish: (id, sha256) => on().finishUpload(id, sha256),
      cancel: (id) => on().cancelUpload(id),
    };
  }

  private uploadQueue(): UploadQueue {
    if (this.queue) return this.queue;
    const q = new UploadQueue({
      transport: () => this.uploadTransport(),
      hasher: workerHasher,
      onDone: (v) => {
        if (v.result?.relinked) session.conn?.pushNotice("info", `“${v.name}” is linked again.`, `upload:${v.key}`);
        void this.loadMedia();
        void this.loadStorage();
      },
    });
    q.subscribe((l) => (this.uploads = l));
    this.queue = q;
    return q;
  }

  /** Uploads files into the active collection (and the chosen bin). */
  importFiles(files: { file: File; relativePath?: string; replaces?: string }[]) {
    const collection = this.loadedCollection;
    if (!collection || !files.length) return;
    const reqs: UploadRequest[] = files.map((f) => ({
      file: f.file,
      name: f.file.name,
      collection,
      ...(this.importBin ? { bin: this.importBin } : {}),
      ...(f.relativePath ? { relativePath: f.relativePath } : {}),
      ...(f.replaces ? { replaces: f.replaces } : {}),
    }));
    this.uploadQueue().add(reqs);
  }
  cancelUpload(key: number) {
    void this.queue?.cancel(key);
  }
  retryUpload(key: number) {
    this.queue?.retry(key);
  }
  clearUploads() {
    this.queue?.clearFinished();
  }
  /** The connection is back: waiting uploads go on at once. */
  resumeUploads() {
    this.queue?.resumeAll();
  }

  // ---------------------------------------------------------------- missing media

  /** Relinks a missing asset to the file of another item of the media library. */
  async relinkTo(m: CollectionMedia, target: RelinkTarget, label: string) {
    const conn = session.conn;
    const c = this.loadedCollection;
    if (!conn || !c) return;
    try {
      await this.sideLink().run(conn.rest, (rest) => new CodexClient(rest).relink(c, m.asset.id, target));
      conn.pushNotice("info", `“${m.asset.name}” now uses ${label}.`, `relink:${m.asset.id}`);
      this.relinkFor = null;
      await Promise.all([this.loadMedia(), this.loadStorage()]);
    } catch (e) {
      if (e instanceof CodexBoxError && e.reason !== "unknown") return;
      this.fail(e, `Relinking “${m.asset.name}”`);
    }
  }

  /** "Find missing files": the server looks for every missing file (or `assets`). */
  async findMissing(assets?: string[]) {
    const conn = session.conn;
    const c = this.loadedCollection;
    if (!conn || !c) return;
    this.busy = true;
    try {
      const r = await this.sideLink().run(conn.rest, (rest) => new CodexClient(rest).relinkSearch(c, assets ? { assets } : {}));
      this.relinkReport = { title: "Find missing files", lines: relinkReportLines(r) };
      await Promise.all([this.loadMedia(), this.loadStorage()]);
    } catch (e) {
      if (e instanceof CodexBoxError && e.reason !== "unknown") return;
      this.fail(e, "Finding missing files");
    } finally {
      this.busy = false;
    }
  }

  /**
   * "Match a folder on this computer": asks the server which of the folder's files are the
   * missing ones, then (after a yes) uploads each match with `replaces`, which relinks it.
   */
  async matchFolder(files: { file: File; relativePath?: string }[]) {
    const conn = session.conn;
    const c = this.loadedCollection;
    if (!conn || !c || !files.length) return;
    const missing = this.media.filter((m) => m.availability === "missing").map((m) => m.asset.id);
    this.busy = true;
    let r: MediaMatchReport;
    try {
      r = await this.sideLink().run(conn.rest, (rest) =>
        new CodexClient(rest).matchFolder(c, {
          files: files.map((f) => ({ name: f.file.name, size: f.file.size, ...(f.relativePath ? { relativePath: f.relativePath } : {}) })),
          ...(missing.length ? { assets: missing } : {}),
        }),
      );
    } catch (e) {
      if (!(e instanceof CodexBoxError && e.reason !== "unknown")) this.fail(e, "Matching the folder");
      return;
    } finally {
      this.busy = false;
    }
    const byKey = new Map(files.map((f) => [`${f.relativePath ?? f.file.name}|${f.file.size}`, f]));
    const uploads = r.matches
      .map((mt) => ({ mt, f: byKey.get(`${mt.file.relativePath ?? mt.file.name}|${mt.file.size}`) }))
      .filter((x): x is { mt: (typeof r.matches)[number]; f: { file: File; relativePath?: string } } => !!x.f);
    const lines = matchReportLines(r);
    if (!uploads.length) {
      this.relinkReport = { title: "Match a folder", lines };
      return;
    }
    const ok = await shell.confirm(
      "Match a folder",
      `${lines.join(" ")} Upload ${uploads.length === 1 ? "this file" : `these ${uploads.length} files`} to link ${uploads.length === 1 ? "it" : "them"} again?`,
      uploads.length === 1 ? "Upload 1 file" : `Upload ${uploads.length} files`,
      false,
    );
    if (!ok) return;
    this.importFiles(uploads.map(({ mt, f }) => ({ file: f.file, ...(f.relativePath ? { relativePath: f.relativePath } : {}), replaces: mt.asset })));
  }

  // ---------------------------------------------------------------- collections (admins)

  /** Runs a collection admin action (through the Codex box on a show box), then reloads. */
  private async admin<T>(what: string, fn: (codex: CodexClient) => Promise<T>): Promise<T | undefined> {
    const conn = session.conn;
    if (!conn) return undefined;
    this.busy = true;
    try {
      const v = await this.sideLink().run(conn.rest, (rest) => fn(new CodexClient(rest)));
      await Promise.all([this.loadCollections(), this.loadDeletedCollections(), this.loadStorage()]);
      return v;
    } catch (e) {
      if (!(e instanceof CodexBoxError && e.reason !== "unknown")) this.fail(e, what);
      return undefined;
    } finally {
      this.busy = false;
    }
  }

  async createCollection() {
    const name = (await shell.askText("New collection", "", "Create", "Name"))?.trim();
    if (!name) return;
    const c = await this.admin(`Creating “${name}”`, (x) => x.createCollection({ name }));
    if (c) this.selected = c.collection.id;
  }

  private collectionEdits: Promise<unknown> = Promise.resolve();

  /** Changes a collection's name, description or colour; false when it failed. One after the
   * other, each with the revision the one before left (a rename, then a colour clicked at once,
   * would otherwise send the same, stale, revision twice). */
  updateCollection(c: CollectionSummary, patch: { name?: string; description?: string | null; color?: Color | null }, what: string): Promise<boolean> {
    const job = this.collectionEdits.then(async () => {
      const now = this.collections.find((x) => x.collection.id === c.collection.id) ?? c;
      return (await this.admin(what, (x) => x.updateCollection(c.collection.id, patch, now.revision))) !== undefined;
    });
    this.collectionEdits = job.catch(() => {});
    return job;
  }

  async deleteCollection(c: CollectionSummary) {
    const name = c.collection.name ?? "this collection";
    if (c.active) {
      session.conn?.pushNotice("info", `${name} is shown on this box. Show another collection first, then delete it.`, "collection:delete");
      return;
    }
    const ok = await shell.confirm(
      `Delete ${name}`,
      `${name} goes to the collection trash with its songs, media links and playlists. It stays there until an admin restores it or deletes it for good.`,
      "Move to collection trash",
      true,
    );
    if (!ok) return;
    const done = await this.admin(`Deleting ${name}`, (x) => x.deleteCollection(c.collection.id, c.revision).then(() => true));
    if (done) this.selected = `trash:${c.collection.id}`;
  }

  async restoreCollection(d: DeletedCollection) {
    const name = d.collection.name ?? "the collection";
    const c = await this.admin(`Restoring ${name}`, (x) => x.restoreCollection(d.collection.id));
    if (c) {
      session.conn?.pushNotice("info", `${name} is back.`, `collection:${d.collection.id}`);
      this.selected = c.collection.id;
    }
  }

  /** Deletes a collection for good, after two confirmations, the second naming it. */
  async purgeCollection(d: DeletedCollection) {
    const name = d.collection.name ?? "Untitled";
    const first = await shell.confirm(
      "Delete for good",
      `Delete ${name} for good? Its documents, its change log and its deleted items go. This can't be undone.`,
      "Continue",
      true,
    );
    if (!first) return;
    const second = await shell.confirm(
      `Delete “${name}” for good`,
      `Last check: “${name}” and its ${d.counts.presentations === 1 ? "1 song" : `${d.counts.presentations} songs`} will be gone for good. Media files stay in the media library.`,
      `Delete “${name}” for good`,
      true,
    );
    if (!second) return;
    const done = await this.admin(`Deleting ${name} for good`, (x) => x.purgeCollection(d.collection.id, d.revision).then(() => true));
    if (done) {
      session.conn?.pushNotice("info", `${name} was deleted for good.`, `collection:${d.collection.id}`);
      this.selected = null;
    }
  }

  // ---------------------------------------------------------------- themes (linked)

  /** Runs a theme or song change; failures become a notice. */
  private async themeOp<T>(what: string, fn: (rest: RestClient) => Promise<T>): Promise<T | undefined> {
    const conn = session.conn;
    if (!conn) return undefined;
    this.busy = true;
    try {
      return await fn(conn.rest);
    } catch (e) {
      this.fail(e, what);
      return undefined;
    } finally {
      this.busy = false;
    }
  }

  /** "+ New theme": three starter template slides; the new theme is selected. */
  async createTheme() {
    const v = await shell.askText("New theme", "", "Create", "Name");
    if (v === null) return;
    const name = v.trim();
    const r = await this.themeOp("Creating the theme", (rest) => rest.createTheme(name || undefined));
    if (!r) return;
    await this.loadThemes();
    this.selected = r.id;
  }

  async duplicateTheme(t: ThemeSummary) {
    const r = await this.themeOp(`Duplicating “${t.name}”`, (rest) => rest.duplicateTheme(t.id));
    if (!r) return;
    await this.loadThemes();
    this.selected = r.id;
  }

  async renameTheme(t: ThemeSummary) {
    const name = (await shell.askText("Rename theme", t.name, "Rename", "Name"))?.trim();
    if (!name || name === t.name) return;
    if (await this.themeOp(`Renaming “${t.name}”`, (rest) => rest.renameTheme(t.id, name))) await this.loadThemes();
  }

  /** Deletes a theme. One songs follow needs `reassign` (they follow that theme) or `unlink`
   * (they keep their look); the item panel asks which. */
  async deleteTheme(t: ThemeSummary, opts: { reassign?: string; unlink?: boolean } = {}) {
    if (!t.usedBy) {
      const ok = await shell.confirm(`Delete “${t.name}”`, `“${t.name}” goes to Recently deleted. No song follows it.`, "Delete", true);
      if (!ok) return false;
    }
    const done = await this.themeOp(`Deleting “${t.name}”`, (rest) => rest.deleteTheme(t.id, opts));
    if (!done) return false;
    if (this.selected === t.id) this.selected = opts.reassign ?? null;
    await Promise.all([this.loadThemes(), this.loadSongs(), this.loadTrash()]);
    return true;
  }

  /** Opens a theme's template slides in the slide editor (like a prop). */
  async openTheme(id: string) {
    const conn = session.conn;
    if (!conn) return;
    conn.rest.themeDocuments.add(id);
    try {
      const p = await conn.rest.presentation(id);
      session.showOneSlide(p, false, true);
    } catch (e) {
      conn.pushNotice("error", `Could not open the theme: ${errorText(e)}`);
    }
  }

  /** The song follows `theme` from now on (every slide; text kept). */
  async applyTheme(songId: string, theme: string) {
    const r = await this.themeOp("Applying the theme", (rest) => rest.editPresentation(songId, { ops: [{ type: "applyTheme", theme }] }));
    if (r) await this.loadSongs();
    return !!r;
  }

  /** "Link to theme, keeping the look": the song's unlinked slides follow the theme that fits
   * each best, every difference kept as an override (nothing on screen changes). Answers how
   * many slides now follow, or null when it failed. */
  async linkKeepingLook(songId: string): Promise<number | null> {
    const r = await this.themeOp("Linking to the theme", (rest) => rest.editPresentation(songId, { ops: [{ type: "linkTheme" }] }));
    if (!r) return null;
    await this.loadSongs();
    return r.applied.filter((o) => o.type === "replaceSlide").length;
  }

  /** "Reset to theme" for every slide of the song that follows its theme. */
  async resetSongToTheme(songId: string, slides: string[]) {
    if (!slides.length) return false;
    const r = await this.themeOp("Resetting to the theme", (rest) =>
      rest.editPresentation(songId, { ops: slides.map((slide) => ({ type: "resetToTheme" as const, slide })) }),
    );
    if (r) await this.loadSongs();
    return !!r;
  }

  // ---------------------------------------------------------------- a song in Edit

  /** Opens the song in the slide editor's components; what Show had open comes back after. */
  async startEdit(songId: string) {
    if (this.mode === "edit" && session.open?.presentation.id === songId) return;
    if (!this.openBefore) {
      this.openBefore = {
        id: session.open && !session.open.prop ? session.open.presentation.id : null,
        playlistItem: session.open?.playlistItem ?? null,
        selP: showView.selectedPresentationId,
        selI: showView.selectedItemId,
      };
    }
    this.editState = new EditState();
    this.mode = "edit";
    shell.pageEditing = true;
    this.railOpen = false;
    await session.openPresentation(songId);
  }

  async endEdit() {
    if (this.mode !== "edit") return;
    await this.editState?.finishText?.();
    // Typing kept from a lost edit lock is reported with its text, never dropped (D09-1).
    this.editState?.reportLocked();
    this.mode = "view";
    this.editState = null;
    shell.pageEditing = false;
    const before = this.openBefore;
    this.openBefore = null;
    if (!before) return;
    // (The same song too: Edit opened it without its playlist item, which comes back here, so
    // triggers carry the playlist again.)
    if (before.id) await session.openPresentation(before.id, before.playlistItem, true);
    else session.open = null;
    showView.selectedPresentationId = before.selP;
    showView.selectedItemId = before.selI;
  }

  private fail(e: unknown, what: string) {
    const f = codexFailure(e);
    if (f.kind === "forbidden" && e instanceof ApiRequestError) {
      // The site's unlock (PIN) or sign-in, then the operator tries again.
      void sitePage.unlockFor(e).then((ok) => ok && session.conn?.pushNotice("info", `Unlocked: try ${what.toLowerCase()} again.`, `library:${what}`));
      return;
    }
    const msg =
      f.kind === "forbidden"
        ? forbiddenText(what)
        : f.kind === "elsewhere" || e instanceof CodexBoxError
          ? `${what} isn't possible here: ${e instanceof CodexBoxError ? e.message : f.message}.`
          : f.kind === "unavailable"
            ? `${what} needs the site's server, which can't be reached now. The show keeps playing.`
            : `${what} failed: ${f.message}`;
    session.conn?.pushNotice(f.kind === "forbidden" || f.kind === "unavailable" ? "warning" : "error", msg, `library:${what}`);
  }

  reset() {
    this.collections = [];
    this.songs = [];
    this.media = [];
    this.themes = [];
    this.props = [];
    this.trash = [];
    this.loadedCollection = null;
    this.hits = null;
    this.selected = null;
    this.storage = null;
    this.deletedCollections = [];
    this.relinkFor = null;
    this.relinkReport = null;
    this.mode = "view";
    this.editState = null;
    shell.pageEditing = false;
    this.openBefore = null;
    this.link?.reset();
    this.link = null;
  }
}

// The SHA-256 of uploads, in a Web Worker (on the main thread where workers aren't available).
let worker: Worker | null = null;
let workerSeq = 0;
const jobs = new Map<number, { done: (sha: string) => void; fail: (e: Error) => void; progress: (n: number) => void }>();

export const workerHasher: Hasher = (file, onProgress) => {
  if (typeof Worker === "undefined") return hashBlob(file, onProgress);
  if (!worker) {
    try {
      worker = new Worker(new URL("../pages/library/sha256.worker.ts", import.meta.url), { type: "module" });
    } catch {
      return hashBlob(file, onProgress);
    }
    worker.onmessage = (e: MessageEvent<{ id: number; done?: number; sha256?: string; error?: string }>) => {
      const j = jobs.get(e.data.id);
      if (!j) return;
      if (e.data.sha256) {
        jobs.delete(e.data.id);
        j.done(e.data.sha256);
      } else if (e.data.error) {
        jobs.delete(e.data.id);
        j.fail(new Error(e.data.error));
      } else if (e.data.done !== undefined) j.progress(e.data.done);
    };
  }
  const id = ++workerSeq;
  return new Promise<string>((done, fail) => {
    jobs.set(id, { done, fail, progress: onProgress });
    worker!.postMessage({ id, file });
  });
};

export const libraryPage = new LibraryPageState();
