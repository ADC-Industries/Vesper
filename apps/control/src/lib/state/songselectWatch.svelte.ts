// The SongSelect Downloads watcher (Settings ▸ CCLI; desktop app; per computer, off by
// default): while it is on and Oriel is connected, the Downloads folder is listed every few
// seconds (edit/downloadWatch.ts picks the new, finished files) and each new file is read by
// the desktop shell, which recognises a SongSelect song by its content. A song found shows a
// notice, "New SongSelect song: <title>", that opens the SongSelect import preview with the
// file (duplicates handled there). Nothing is ever imported by itself.

import { DownloadWatch } from "../edit/downloadWatch";
import { toBase64 } from "../edit/songselect";
import { downloadedSong, isTauri, listDownloads, type DownloadedSong } from "../tauri";
import { session } from "./session.svelte";
import { shell } from "./shell.svelte";

const KEY = "oriel.songselectWatch";
const EVERY_MS = 3000;

function stored(): boolean {
  try {
    return typeof localStorage !== "undefined" && localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

class SongSelectWatch {
  /** The switch (this computer). */
  enabled = $state(isTauri() && stored());
  /** Songs found and not yet answered, oldest first. */
  found = $state.raw<DownloadedSong[]>([]);
  /** Why the folder can't be watched, if it can't. */
  problem = $state<string | null>(null);
  readonly available = isTauri();
  private watch: DownloadWatch | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private busy = false;

  /** Starts watching if the switch is on (once, at start-up). */
  start() {
    if (this.enabled) this.run();
  }

  set(on: boolean) {
    this.enabled = on && this.available;
    try {
      localStorage.setItem(KEY, this.enabled ? "1" : "0");
    } catch {
      /* kept for this session only */
    }
    if (this.enabled) this.run();
    else this.stop();
  }

  private run() {
    if (this.timer) return;
    this.watch = new DownloadWatch();
    const tick = async () => {
      this.timer = null;
      if (!this.enabled) return;
      await this.poll();
      if (this.enabled) this.timer = setTimeout(() => void tick(), EVERY_MS);
    };
    void tick();
  }

  private stop() {
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.watch = null;
    this.found = [];
    this.problem = null;
  }

  private async poll() {
    if (this.busy || !this.watch || !session.conn) return;
    this.busy = true;
    try {
      const fresh = this.watch.step(await listDownloads(), Date.now());
      this.problem = null;
      for (const f of fresh) {
        const song = await downloadedSong(f.name).catch(() => null);
        if (song && !this.found.some((s) => s.name === song.name)) this.found = [...this.found, song];
      }
    } catch (e) {
      this.problem = e instanceof Error ? e.message : String(e);
    } finally {
      this.busy = false;
    }
  }

  /** The notice's Import: the SongSelect import preview with the file. */
  import(song: DownloadedSong) {
    this.dismiss(song);
    shell.songSelect = { files: [{ name: song.name, data: toBase64(new Uint8Array(song.data)) }] };
  }

  dismiss(song: DownloadedSong) {
    this.found = this.found.filter((s) => s !== song);
  }
}

export const songSelectWatch = new SongSelectWatch();
