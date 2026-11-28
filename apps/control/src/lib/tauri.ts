// Tauri-only features. Everything here degrades to "not available" in a plain browser.

export interface DiscoveredServer {
  id: string;
  name: string;
  version: string;
  protocolVersion: number;
  baseUrl: string | null;
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** mDNS discovery through the shell's `discover_servers` command (empty in a browser). */
export async function discoverServers(timeoutMs = 1500): Promise<DiscoveredServer[]> {
  if (!isTauri()) return [];
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<DiscoveredServer[]>("discover_servers", { timeoutMs });
}

/** The middle of Oriel's window on the desktop in physical pixels (the server's display
 * positions are physical), from the desktop shell; null in a browser or where the window
 * system doesn't say (Wayland: no position, reported as the origin). */
export async function windowCentre(): Promise<{ x: number; y: number } | null> {
  if (!isTauri()) return null;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    const w = getCurrentWindow();
    const [p, s] = await Promise.all([w.outerPosition(), w.outerSize()]);
    if (p.x === 0 && p.y === 0) return null;
    return { x: p.x + s.width / 2, y: p.y + s.height / 2 };
  } catch {
    return null;
  }
}

/** A file in the user's Downloads folder (the shell's `downloads_list`). */
export interface DownloadFile {
  name: string;
  size: number;
  modifiedMs: number;
}

/** A download the shell recognised as a SongSelect song by its content. */
export interface DownloadedSong {
  name: string;
  /** The song's title, if the file has one. */
  title: string | null;
  /** The file's bytes. */
  data: number[];
}

/** The files in the user's Downloads folder (empty in a browser). */
export async function listDownloads(): Promise<DownloadFile[]> {
  if (!isTauri()) return [];
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<DownloadFile[]>("downloads_list");
}

/** A file of the Downloads folder if it is a SongSelect song (null otherwise, or in a browser). */
export async function downloadedSong(name: string): Promise<DownloadedSong | null> {
  if (!isTauri()) return null;
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<DownloadedSong | null>("downloads_song", { name });
}
