// The SongSelect Downloads watcher's choice of files (Settings ▸ CCLI, desktop app, off by
// default): from each listing of the Downloads folder, which files to read. The desktop shell
// lists the folder and reads a file; whether it is a SongSelect song is decided by its
// content there (model::songselect::recognise), never by its name.
//
// A file is read once it is new since watching started (the files already there are left
// alone), not a partial download (`.crdownload`, `.part`, …), not empty, and the same size and
// time in two listings in a row and untouched for `settleMs` (still growing: wait). Each
// version of a file (name, size, time) is read once.

/** A file in the Downloads folder (the shell's `downloads_list`). */
export interface DownloadEntry {
  name: string;
  size: number;
  modifiedMs: number;
}

/** Browsers' names for a download in progress (Chrome and Edge, Firefox, Safari, Opera), and
 * temporary or hidden files. */
const PARTIAL = /\.(crdownload|part|partial|download|opdownload|tmp)$/i;

export function isPartial(name: string): boolean {
  return PARTIAL.test(name) || name.startsWith(".") || name.startsWith("~$");
}

const signature = (e: DownloadEntry) => `${e.size}:${e.modifiedMs}`;

export class DownloadWatch {
  private baseline: Map<string, string> | null = null;
  private last = new Map<string, string>();
  private done = new Set<string>();

  constructor(private readonly settleMs = 2000) {}

  /** One listing taken at `now` (ms): the files to read now. The first listing only notes
   * what is already there. */
  step(list: DownloadEntry[], now: number): DownloadEntry[] {
    const current = new Map(list.map((e) => [e.name, signature(e)]));
    if (!this.baseline) {
      this.baseline = current;
      this.last = current;
      return [];
    }
    const out: DownloadEntry[] = [];
    for (const e of list) {
      const sig = signature(e);
      if (isPartial(e.name) || e.size <= 0) continue;
      if (this.baseline.get(e.name) === sig) continue;
      if (this.last.get(e.name) !== sig) continue;
      if (now - e.modifiedMs < this.settleMs) continue;
      const key = `${e.name}\u0000${sig}`;
      if (this.done.has(key)) continue;
      this.done.add(key);
      out.push(e);
    }
    this.last = current;
    return out;
  }
}
