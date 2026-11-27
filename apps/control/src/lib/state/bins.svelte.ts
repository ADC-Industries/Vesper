// The Library's media bins, bin playlists and global groups (G17, Svelte 5 runes): the bin or
// media playlist shown, and the actions (create, rename, reorder, delete; add and remove media;
// groups' name, colour and order). The bin tree and the global groups themselves stay where the
// rest of Oriel reads them (`mediaState.mediaBins`, `session.globalGroups`); they are read again
// after each write here (the `documentChanged` that follows does the same for other Oriels).

import type { CollectionMedia, GlobalGroup, MediaBinDetail, PlaylistDetail } from "$protocol";
import type { Connection } from "../server";
import { ApiRequestError, type RestClient } from "../server/rest";
import { addToBinOps, addToPlaylistOps, binLabel, binMove, groupMove, mediaBinRows, nameOk, type BinRow } from "../pages/library/bins";
import { mediaKind } from "../pages/library/logic";
import { errorText } from "./errors";
import { changes } from "./events";
import { libraryPage } from "./library.svelte";
import { mediaState } from "./media.svelte";
import { session } from "./session.svelte";
import { shell } from "./shell.svelte";
import { sitePage } from "./site.svelte";

/** What a drop or "Add to" puts media into. */
export type BinTarget = { kind: "bin"; id: string } | { kind: "playlist"; id: string };

class BinsPage {
  /** The bin shown (view `bin`). */
  bin = $state.raw<MediaBinDetail | null>(null);
  /** The media playlist shown (view `mediaPlaylist`). */
  playlist = $state.raw<PlaylistDetail | null>(null);
  busy = $state(false);

  rows = $derived<BinRow[]>(mediaBinRows(mediaState.mediaBins));

  /** Follows changes of the bin or playlist shown (while the Library page is open). Changes
   * made while the page was closed, or while the socket was down, are read again on attach and
   * on a reconnect (D09-2). */
  attach(): () => void {
    void this.load();
    const offs = [
      changes.on("documentChanged", (e) => {
        const v = libraryPage.view;
        if (e.kind === "mediaBin" && v.kind === "bin" && e.id === v.id) void this.load();
        if (e.kind === "playlistNode" && v.kind === "mediaPlaylist" && e.id === v.id) void this.load();
      }),
      session.conn?.onSynced((reconnect) => reconnect && void this.load()) ?? (() => {}),
    ];
    return () => offs.forEach((f) => f());
  }

  /** (Re)loads the bin or media playlist the Library shows. */
  async load() {
    const conn = session.conn;
    const v = libraryPage.view;
    if (!conn) return;
    try {
      if (v.kind === "bin") {
        const b = await conn.rest.mediaBin(v.id);
        if (libraryPage.view === v) this.bin = b;
      } else if (v.kind === "mediaPlaylist") {
        const p = await conn.rest.playlist(v.id);
        if (libraryPage.view === v) this.playlist = p;
      }
    } catch (e) {
      if (e instanceof ApiRequestError && e.status === 404) {
        // Deleted (here or elsewhere): back to the media.
        libraryPage.go({ kind: "media", media: "image" });
        return;
      }
      conn.pushNotice("warning", `Could not load it: ${errorText(e)}`, "bins:load");
    }
  }

  open(kind: "bin" | "mediaPlaylist", id: string) {
    this.bin = null;
    this.playlist = null;
    libraryPage.go(kind === "bin" ? { kind: "bin", id } : { kind: "mediaPlaylist", id });
    void this.load();
  }

  private async op<T>(what: string, fn: (rest: RestClient) => Promise<T>): Promise<T | undefined> {
    const conn = session.conn;
    if (!conn) return undefined;
    this.busy = true;
    try {
      return await fn(conn.rest);
    } catch (e) {
      this.fail(conn, e, what);
      return undefined;
    } finally {
      this.busy = false;
    }
  }

  private fail(conn: Connection, e: unknown, what: string) {
    if (e instanceof ApiRequestError && e.status === 403) {
      void sitePage.unlockFor(e).then((ok) => ok && conn.pushNotice("info", `Unlocked: try ${what.toLowerCase()} again.`, `bins:${what}`));
      return;
    }
    if (e instanceof ApiRequestError && e.status === 503) {
      conn.pushNotice("warning", `${what} needs the site's server, which can't be reached now. The show keeps playing.`, `bins:${what}`);
      return;
    }
    if (e instanceof ApiRequestError && e.status === 409) void this.refresh();
    conn.pushNotice("error", `${what} failed: ${errorText(e)}`, `bins:${what}`);
  }

  /** Reads the bin tree, the groups and the open bin again. */
  async refresh() {
    const conn = session.conn;
    if (!conn) return;
    const [bins, groups] = await Promise.allSettled([conn.rest.mediaBins(), conn.rest.globalGroups()]);
    if (session.conn !== conn) return;
    if (bins.status === "fulfilled") mediaState.setBins(bins.value);
    if (groups.status === "fulfilled") session.globalGroups = groups.value;
    await this.load();
  }

  // ---------------------------------------------------------------- bins

  /** "+ New bin": asks the name; the new bin is opened. */
  async createBin() {
    const v = await shell.askText("New media bin", "", "Create", "Name");
    if (v === null) return;
    if (!nameOk(v)) {
      session.conn?.pushNotice("warning", "A bin needs a name (up to 200 characters).", "bins:name");
      return;
    }
    const r = await this.op("Creating the bin", (rest) => rest.createMediaBin({ name: v.trim() }));
    if (!r) return;
    await this.refresh();
    this.open("bin", r.id);
  }

  async renameBin(row: BinRow) {
    const v = (await shell.askText("Rename media bin", row.name, "Rename", "Name"))?.trim();
    if (!v || v === row.name) return;
    if (!nameOk(v)) return;
    if (await this.op(`Renaming “${row.name}”`, (rest) => rest.updateMediaBin(row.id, { name: v }))) await this.refresh();
  }

  async moveBin(row: BinRow, dir: -1 | 1) {
    const m = binMove(this.rows, row.id, dir);
    if (!m) return;
    if (await this.op(`Moving “${row.name}”`, (rest) => rest.updateMediaBin(row.id, { after: m.after }))) await this.refresh();
  }

  async deleteBin(row: BinRow) {
    const ok = await shell.confirm(
      `Delete “${binLabel(row)}”`,
      `The bin goes to Recently deleted. Its ${row.itemCount === 1 ? "file stays" : "files stay"} in the collection.`,
      "Delete",
      true,
    );
    if (!ok) return;
    if (!(await this.op(`Deleting “${row.name}”`, (rest) => rest.deleteMediaBin(row.id)))) return;
    libraryPage.go({ kind: "media", media: "image" });
    await Promise.all([this.refresh(), libraryPage.loadTrash()]);
  }

  /** Adds files to a bin or media playlist (from a drop or "Add to"). Files it already holds
   * aren't added twice (bins). */
  async addMedia(target: BinTarget, files: Pick<CollectionMedia, "asset">[]) {
    const conn = session.conn;
    if (!conn || !files.length) return;
    const list = files.map((m) => ({ asset: m.asset.id, kind: mediaKind(m.asset) }));
    if (target.kind === "bin") {
      const row = this.rows.find((r) => r.id === target.id);
      const done = await this.op(`Adding to “${row?.name ?? "the bin"}”`, async (rest) => {
        const bin = await rest.mediaBin(target.id);
        const ops = addToBinOps(bin.items, list);
        if (!ops.length) return "none" as const;
        await rest.editMediaBin(target.id, { ops });
        return ops.length;
      });
      if (done === "none") conn.pushNotice("info", `Already in “${row?.name ?? "the bin"}”.`, "bins:add");
      else if (done) conn.pushNotice("info", `Added ${done === 1 ? "1 file" : `${done} files`} to “${row?.name ?? "the bin"}”.`, "bins:add");
    } else {
      const done = await this.op("Adding to the playlist", async (rest) => {
        const pl = await rest.playlist(target.id);
        await rest.editPlaylist(target.id, { ops: addToPlaylistOps(pl.items, list) });
        return pl.name;
      });
      if (done) conn.pushNotice("info", `Added to “${done}”.`, "bins:add");
    }
    await this.refresh();
  }

  /** Removes an item from the bin or media playlist shown (its file stays). */
  async removeItem(itemId: string) {
    const conn = session.conn;
    const v = libraryPage.view;
    if (!conn) return;
    if (v.kind === "bin") {
      await this.op("Removing from the bin", (rest) => rest.editMediaBin(v.id, { ops: [{ type: "removeItem", item: itemId }] }));
    } else if (v.kind === "mediaPlaylist") {
      await this.op("Removing from the playlist", (rest) => rest.editPlaylist(v.id, { ops: [{ type: "removeItem", item: itemId }] }));
    }
    await this.refresh();
  }

  /** "+ New media playlist". */
  async createMediaPlaylist() {
    const v = await shell.askText("New media playlist", "", "Create", "Name");
    if (v === null || !nameOk(v)) return;
    const r = await this.op("Creating the playlist", (rest) => rest.createPlaylist({ name: v.trim(), purpose: "media" }));
    if (!r) return;
    await session.refetch("playlistNode", async (conn) => {
      session.playlistTree = await conn.rest.playlists();
    });
    this.open("mediaPlaylist", r.id);
  }

  async renameMediaPlaylist(p: { id: string; name: string }) {
    const v = (await shell.askText("Rename media playlist", p.name, "Rename", "Name"))?.trim();
    if (!v || v === p.name || !nameOk(v)) return;
    if (!(await this.op(`Renaming “${p.name}”`, (rest) => rest.updatePlaylist(p.id, { name: v })))) return;
    await session.refetch("playlistNode", async (conn) => {
      session.playlistTree = await conn.rest.playlists();
    });
    await this.load();
  }

  async deleteMediaPlaylist(p: { id: string; name: string }) {
    const ok = await shell.confirm(`Delete “${p.name}”`, "The playlist goes to Recently deleted. Its files stay in the collection.", "Delete", true);
    if (!ok) return;
    if (!(await this.op(`Deleting “${p.name}”`, (rest) => rest.deletePlaylist(p.id)))) return;
    libraryPage.go({ kind: "media", media: "image" });
    await session.refetch("playlistNode", async (conn) => {
      session.playlistTree = await conn.rest.playlists();
    });
    await libraryPage.loadTrash();
  }

  // ---------------------------------------------------------------- global groups

  async createGroup() {
    const v = await shell.askText("New group", "", "Create", "Name");
    if (v === null || !nameOk(v)) return;
    if (await this.op("Creating the group", (rest) => rest.createGlobalGroup({ name: v.trim() }))) await this.refresh();
  }

  async renameGroup(g: GlobalGroup) {
    const v = (await shell.askText("Rename group", g.name ?? "", "Rename", "Name"))?.trim();
    if (!v || v === g.name || !nameOk(v)) return;
    if (await this.op(`Renaming “${g.name}”`, (rest) => rest.updateGlobalGroup(g.id, { name: v }))) await this.refresh();
  }

  async recolorGroup(g: GlobalGroup, color: { r: number; g: number; b: number } | null) {
    if (await this.op(`Recolouring “${g.name}”`, (rest) => rest.updateGlobalGroup(g.id, { color }))) await this.refresh();
  }

  async moveGroup(g: GlobalGroup, dir: -1 | 1) {
    const m = groupMove(session.globalGroups, g.id, dir);
    if (!m) return;
    if (await this.op(`Moving “${g.name}”`, (rest) => rest.updateGlobalGroup(g.id, { after: m.after }))) await this.refresh();
  }

  async deleteGroup(g: GlobalGroup) {
    const ok = await shell.confirm(`Delete “${g.name}”`, "The group goes to Recently deleted. Songs keep their sections as they are.", "Delete", true);
    if (!ok) return;
    if (await this.op(`Deleting “${g.name}”`, (rest) => rest.deleteGlobalGroup(g.id))) await Promise.all([this.refresh(), libraryPage.loadTrash()]);
  }
}

export const binsPage = new BinsPage();
