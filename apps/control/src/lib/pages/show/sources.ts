// The bin's Video Inputs tab (G7): how a live network source is named and described. OMT
// sources (docs/OMT-PLAN.md) are named `omt:` + their OMT name or address.
import type { InputSource, LiveSource, LiveSourceStatus, VideoInputEntry } from "$protocol";

/** An OMT source's name (`omt:HOST (name)`, `omt:host:port`, `omt://host:port`). */
export function isOmt(full: string): boolean {
  return full.trim().startsWith("omt:");
}

/** The name without the `omt:` prefix (an `omt://` address stays as is). */
function bare(full: string): string {
  return isOmt(full) && !full.trim().startsWith("omt://") ? full.trim().slice(4).trim() : full;
}

/** `HOST (name)` → `name` (an address or plain name stays as is; `omt:` is dropped). */
export function sourceName(full: string): string {
  full = bare(full);
  const i = full.indexOf(" (");
  return i > 0 && full.endsWith(")") ? full.slice(i + 2, -1) : full;
}

/** `HOST (name)` → `HOST` (null for an address). */
export function sourceHost(full: string): string | null {
  full = bare(full);
  const i = full.indexOf(" (");
  return i > 0 && full.endsWith(")") ? full.slice(0, i) : null;
}

const WORDS: Record<LiveSourceStatus, string> = {
  idle: "Ready",
  connecting: "Connecting…",
  live: "Receiving",
  lost: "Lost · retrying",
  unsupported: "Not supported yet",
};

/** One line about how a source is received: "Receiving · 1920×1080 · 29.97 fps". */
export function statusLine(s: Pick<LiveSource, "status" | "width" | "height" | "frameRate" | "listed">): string {
  const parts = [WORDS[s.status] ?? s.status];
  if (s.status === "live" && s.width && s.height) parts.push(`${s.width}×${s.height}`);
  if (s.status === "live" && s.frameRate) parts.push(`${Math.round(s.frameRate * 100) / 100} fps`);
  if (!s.listed && s.status === "idle") parts[0] = "Not on the network";
  return parts.join(" · ");
}

/** Sources to show: the live one first, then by name; filtered by name or host. */
export function visibleSources(list: LiveSource[], live: string | null, filter: string): LiveSource[] {
  const f = filter.trim().toLowerCase();
  return list
    .filter((s) => !f || s.name.toLowerCase().includes(f))
    .slice()
    .sort((a, b) => Number(b.name === live) - Number(a.name === live) || sourceName(a.name).localeCompare(sourceName(b.name)));
}

/** A typed address (`ip`, `ip:port`, `host:port`, `omt:host:port` for OMT) or a full source
 *  name; null when empty. */
export function manualSource(text: string): string | null {
  const t = text.trim();
  return t ? t : null;
}

/** The network source a logical input is bound to on this box, as the source list names it
 *  (IDN: its name; OMT: `omt:` + its name); null otherwise. */
export function inputSource(v: Pick<VideoInputEntry, "source">): string | null {
  const s = v.source;
  if (s?.type === "idn") return s.sourceName;
  if (s?.type === "omt") return s.name.startsWith("omt://") ? s.name : `omt:${s.name}`;
  return null;
}

/** The binding for a source named as the source list names it (`omt:` → an OMT source). */
export function bindingFor(source: string): InputSource {
  return isOmt(source) ? { type: "omt", name: bare(source) } : { type: "idn", sourceName: source };
}

/** A short transport tag for a source: "OMT" for OMT sources, null for IDN ones. */
export function transportTag(s: Pick<LiveSource, "name" | "omt">): string | null {
  return s.omt || isOmt(s.name) ? "OMT" : null;
}

/** One line about a logical input: its source and how it is received, or why it can't show. */
export function inputLine(v: Pick<VideoInputEntry, "source">, sources: LiveSource[]): string {
  const s = v.source;
  if (!s || s.type === "unknown") return "Not bound here";
  const name = inputSource(v);
  if (!name) return "Capture device · not supported yet";
  const label = `${isOmt(name) ? "OMT · " : ""}${sourceName(name)}`;
  const listed = sources.find((x) => x.name === name);
  return listed ? `${label} · ${statusLine(listed)}` : `${label} · Not on the network`;
}

/** A status dot for a logical input: its bound source's status, "off" when unbound. */
export function inputStatus(v: Pick<VideoInputEntry, "source">, sources: LiveSource[]): LiveSourceStatus | "unbound" {
  const name = inputSource(v);
  if (!name) return "unbound";
  return sources.find((x) => x.name === name)?.status ?? "idle";
}

/** The fill picker's choices: the logical inputs first (portable shows follow each box's
 *  binding), then the network sources; the value is `input:<id>` or the source name. A current
 *  value that is neither listed is kept as its own choice. */
export function liveFillChoices(
  inputs: Pick<VideoInputEntry, "id" | "name">[],
  sources: Pick<LiveSource, "name">[],
  current: { input?: string | null; source?: string | null },
): { group: "inputs" | "sources"; value: string; label: string }[] {
  const out: { group: "inputs" | "sources"; value: string; label: string }[] = inputs.map((v) => ({ group: "inputs", value: `input:${v.id}`, label: v.name || "Unnamed input" }));
  if (current.input && !inputs.some((v) => v.id === current.input)) out.unshift({ group: "inputs", value: `input:${current.input}`, label: "Missing input" });
  if (!current.input && current.source && !sources.some((s) => s.name === current.source)) out.push({ group: "sources", value: current.source, label: current.source });
  for (const s of sources) out.push({ group: "sources", value: s.name, label: isOmt(s.name) ? `OMT: ${bare(s.name)}` : s.name });
  return out;
}

/** The fill picker's value for a live fill. */
export function liveFillValue(k: { input?: string | null; source?: string | null }): string {
  return k.input ? `input:${k.input}` : (k.source ?? "");
}
