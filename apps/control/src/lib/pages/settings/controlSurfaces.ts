// Settings ▸ Network & IDN ▸ Control surfaces: what the Remote API compatibility card says.
import type { RemoteCompatStatus } from "$protocol";
import type { Status } from "../../ui";

/** The status line of the Remote API compatibility layer. */
export function remoteCompatLine(s: RemoteCompatStatus | null): { status: Status; text: string } {
  if (!s) return { status: "off", text: "Reading…" };
  if (!s.enabled) return { status: "off", text: "Off" };
  if (s.listening) return { status: "ok", text: `Listening on port ${s.listeningPort ?? s.port}` };
  return { status: "error", text: s.problem ? `Not listening: ${s.problem}` : "Not listening" };
}

/** A port typed by the operator (1–65535), else null. */
export function parsePort(text: string): number | null {
  const t = text.trim();
  if (!/^\d{1,5}$/.test(t)) return null;
  const n = Number(t);
  return n >= 1 && n <= 65535 ? n : null;
}
