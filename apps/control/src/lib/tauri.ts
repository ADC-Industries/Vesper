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
