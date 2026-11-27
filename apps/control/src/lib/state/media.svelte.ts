// Media state (Svelte 5 runes): the media bins and the bin the Show page's media bin shows, and
// the audio panel (its selected bin or audio playlist, what that holds, the known lengths of audio
// assets) with its actions. The core session fetches the bin tree with its first lists
// (`setBins`) and tells this module when the playlist tree changed (`playlistsChanged`); bin
// changes are refetched here (`attach`).

import type { MediaBinDetail, MediaBinNode, PlaylistDetail, PlaylistItem, PlaylistOp } from "$protocol";
import type { Connection } from "../server";
import { ApiRequestError } from "../server/rest";
import { uuid } from "../edit/ids";
import { audioPlaylists, type AudioSource } from "../pages/show/audio";
import { errorText } from "./errors";
import { changes } from "./events";
import { creds } from "./prefs";
import { session } from "./session.svelte";
import { shell } from "./shell.svelte";
import { showView } from "./show.svelte";

class MediaState {
  // The bottom bin's Media tab
  mediaBins = $state.raw<MediaBinNode[]>([]);
  mediaBinId = $state<string | null>(null);
  mediaBin = $state.raw<MediaBinDetail | null>(null);
  mediaBinLoading = $state(false);

  /** The bin's Audio tab: its selected audio bin or audio playlist, and what it holds. */
  audioSource = $state.raw<AudioSource | null>(null);
  audioBin = $state.raw<MediaBinDetail | null>(null);
  audioPlaylist = $state.raw<PlaylistDetail | null>(null);
  /** Known lengths of audio assets (seconds), for playlist items. */
  audioDurations = $state.raw<ReadonlyMap<string, number | null>>(new Map());

  /** Called once per connection (session.begin): follows bin and asset changes. */
  attach(): () => void {
    return changes.on("documentChanged", (e) => {
      if (e.kind === "mediaBin" || e.kind === "mediaAsset") void session.refetch(e.kind, (conn) => this.refetchBins(conn));
    });
  }

  /** Forgets the audio panel's selection (on disconnect). */
  reset() {
    this.audioSource = null;
    this.audioBin = null;
    this.audioPlaylist = null;
    this.audioDurations = new Map();
  }

  /** Takes the bin tree of the first lists: keeps (or picks) the selected bin and loads it. */
  setBins(bins: MediaBinNode[]) {
    this.mediaBins = bins;
    const all = flattenBins(bins).filter((b) => b.browser !== "audio");
    if (!this.mediaBinId || !all.some((b) => b.id === this.mediaBinId)) {
      const saved = creds.pref("mediaBin");
      this.mediaBinId = all.find((b) => b.id === saved)?.id ?? all[0]?.id ?? null;
      for (const b of all) if (b.id === this.mediaBinId) for (const f of folderKeys(b.path)) showView.expandedBinFolders[f] = true;
    }
    void this.loadMediaBin();
  }

  /** The playlist tree changed (`id`: the playlist that did, if one): audio playlists may have. */
  playlistsChanged(id?: string) {
    this.pickAudioSource();
    if (id && this.audioSource?.kind === "playlist" && this.audioSource.id === id) void this.loadAudio();
  }

  private async refetchBins(conn: Connection) {
    const bins = await conn.rest.mediaBins();
    if (session.conn !== conn) return;
    this.setBins(bins);
    this.pickAudioSource();
    if (this.audioSource?.kind === "bin") void this.loadAudio();
  }

  // ---------------------------------------------------------------- audio panel

  /** Keeps the audio panel's selection valid (the saved one, else the first bin or playlist). */
  pickAudioSource() {
    const bins = flattenBins(this.mediaBins).filter((b) => b.browser === "audio" && b.id);
    const pls = audioPlaylists(session.playlistTree).map((p) => p.id);
    const valid = (s: AudioSource | null) =>
      !!s && (s.kind === "bin" ? bins.some((b) => b.id === s.id) : pls.includes(s.id));
    if (valid(this.audioSource)) return;
    let next: AudioSource | null = null;
    const saved = creds.pref("audioSource");
    const [kind, id] = saved?.split(":") ?? [];
    const fromPref = kind === "bin" || kind === "playlist" ? ({ kind, id: id ?? "" } as AudioSource) : null;
    if (valid(fromPref)) next = fromPref;
    else if (bins[0]?.id) next = { kind: "bin", id: bins[0].id };
    else if (pls[0]) next = { kind: "playlist", id: pls[0] };
    this.audioSource = next;
    void this.loadAudio();
  }

  async selectAudioSource(s: AudioSource) {
    this.audioSource = s;
    creds.setPref("audioSource", `${s.kind}:${s.id}`);
    await this.loadAudio();
  }

  /** (Re)loads the audio panel's bin or playlist. */
  async loadAudio() {
    const conn = session.conn;
    const src = this.audioSource;
    if (!conn || !src) {
      this.audioBin = null;
      this.audioPlaylist = null;
      return;
    }
    const same = () => this.audioSource === src;
    try {
      if (src.kind === "bin") {
        const bin = await conn.rest.mediaBin(src.id);
        if (!same()) return;
        this.audioBin = bin;
        this.audioPlaylist = null;
        const d = new Map(this.audioDurations);
        for (const it of bin.items) if (it.duration != null || !d.has(it.asset)) d.set(it.asset, it.duration);
        this.audioDurations = d;
      } else {
        const pl = await conn.rest.playlist(src.id);
        if (!same()) return;
        this.audioPlaylist = pl;
        this.audioBin = null;
        void this.fetchDurations(pl);
      }
    } catch (e) {
      if (!same()) return;
      this.audioBin = null;
      this.audioPlaylist = null;
      conn.pushNotice("error", `Could not load the audio list: ${errorText(e)}`);
    }
  }

  /** Looks up the lengths of a playlist's media items that no loaded bin told us. */
  private async fetchDurations(pl: PlaylistDetail) {
    const conn = session.conn;
    if (!conn) return;
    const missing = [
      ...new Set(pl.items.flatMap((it) => (it.kind.type === "media" && !this.audioDurations.has(it.kind.asset) ? [it.kind.asset] : []))),
    ];
    if (!missing.length) return;
    const found = await Promise.allSettled(missing.map((a) => conn.rest.media(a)));
    if (session.conn !== conn) return;
    const d = new Map(this.audioDurations);
    found.forEach((r, i) => d.set(missing[i]!, r.status === "fulfilled" ? (r.value.asset.probe?.duration ?? null) : null));
    this.audioDurations = d;
  }

  /** Asks for a name and creates an audio playlist (top level), then selects it. */
  async newAudioPlaylist() {
    const name = await shell.askText("New Audio Playlist", "Audio Playlist", "Create");
    if (!name?.trim()) return;
    const r = await session.edit("New audio playlist", (e) => e.createPlaylist(name.trim(), false, null, "audio"));
    if (!r) return;
    session.playlistTree = (await session.conn?.rest.playlists()) ?? session.playlistTree;
    await this.selectAudioSource({ kind: "playlist", id: r.id });
  }

  async renameAudioPlaylist(id: string, from: string) {
    const to = await shell.askText("Rename Audio Playlist", from, "Rename");
    if (to !== null) await session.rename({ kind: "playlist", id }, from, to);
  }

  /** Appends audio files to a playlist (they play on the audio layer). */
  async addAudioToPlaylist(playlistId: string, media: { asset: string; name: string }[]) {
    if (!media.length) return;
    const items =
      playlistId === this.audioPlaylist?.id ? this.audioPlaylist.items : (await session.conn?.rest.playlist(playlistId))?.items;
    let after = items?.[items.length - 1]?.id ?? null;
    const ops: PlaylistOp[] = media.map((m) => {
      const item: PlaylistItem = { id: uuid(), name: m.name, kind: { type: "media", asset: m.asset, layer: "audio" } };
      const op: PlaylistOp = { type: "addItem", item, ...(after ? { after } : {}) };
      after = item.id;
      return op;
    });
    await session.edit("Add to playlist", (e) => e.editPlaylist(playlistId, "Add to Playlist", ops));
  }

  async removeAudioItem(playlistId: string, itemId: string) {
    await session.edit("Remove", (e) => e.editPlaylist(playlistId, "Remove from Playlist", [{ type: "removeItem", item: itemId }]));
  }

  // ---------------------------------------------------------------- media bin

  async selectMediaBin(id: string) {
    this.mediaBinId = id;
    creds.setPref("mediaBin", id);
    await this.loadMediaBin();
  }

  private async loadMediaBin() {
    const conn = session.conn;
    const id = this.mediaBinId;
    if (!conn || !id) {
      this.mediaBin = null;
      return;
    }
    this.mediaBinLoading = true;
    try {
      const bin = await conn.rest.mediaBin(id);
      if (this.mediaBinId === id) this.mediaBin = bin;
    } catch (e) {
      if (this.mediaBinId !== id) return;
      this.mediaBin = null;
      if (e instanceof ApiRequestError && e.status === 404) {
        // Gone: deleted, or the box shows another collection now. Pick one from the tree as it is now.
        const bins = await conn.rest.mediaBins().catch(() => null);
        if (!bins || session.conn !== conn || this.mediaBinId !== id) return;
        if (flattenBins(bins).some((b) => b.id === id)) this.mediaBins = bins;
        else this.setBins(bins);
        return;
      }
      conn.pushNotice("error", `Could not load the media bin: ${errorText(e)}`);
    } finally {
      this.mediaBinLoading = false;
    }
  }
}

export const mediaState = new MediaState();

/** Every bin of a media bin tree, depth first. */
export function flattenBins(nodes: MediaBinNode[]): MediaBinNode[] {
  const out: MediaBinNode[] = [];
  for (const n of nodes) {
    if (n.kind === "folder") out.push(...flattenBins(n.children));
    else out.push(n);
  }
  return out;
}

/** Keys of a folder path and its ancestors (for expanding the tree to a bin). */
export function folderKeys(path: string[]): string[] {
  return path.map((_, i) => path.slice(0, i + 1).join("/"));
}
