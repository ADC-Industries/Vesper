// Live captions in Oriel (docs/CAPTIONS-PLAN.md): the status line of the Show page's captions
// strip and Settings ▸ Captions, and the source / model choices.
import type { CaptionModel, CaptionSource, CaptionsConfig, CaptionsState } from "$protocol";

/** One line for the strip: what the engine is doing. */
export function captionStatus(c: CaptionsState | null): { cls: "off" | "ok" | "busy" | "bad"; text: string } {
  if (!c || !c.enabled) return { cls: "off", text: "Off" };
  switch (c.status) {
    case "running":
      return { cls: "ok", text: c.model ? `Listening · ${c.model}${c.load != null ? ` · load ${Math.round(c.load * 100)} %` : ""}` : (c.detail ?? "On") };
    case "downloading":
      return { cls: "busy", text: `${c.detail ? capital(c.detail) : "Downloading the model"}${c.progress != null ? ` · ${c.progress} %` : ""}` };
    case "starting":
      return { cls: "busy", text: c.detail ? capital(c.detail) : "Starting" };
    case "error":
      return { cls: "bad", text: c.detail ? `Stopped: ${c.detail}` : "Stopped" };
    default:
      return { cls: "off", text: "Off" };
  }
}

/** A model download for a progress bar: `fraction` 0–1 (null: size unknown, the bar runs
 * without a measure) and how far it is in megabytes (the percentage is in the status line);
 * null when nothing is downloading. Servers before IDN v0.3.3 send only `progress` (no
 * megabytes). */
export function downloadBar(c: CaptionsState | null): { fraction: number | null; text: string } | null {
  if (!c || !c.enabled || c.status !== "downloading") return null;
  const size = c.downloadSize ?? null;
  const done = c.downloaded ?? null;
  const fraction = size && done != null ? Math.min(1, done / size) : c.progress != null ? c.progress / 100 : null;
  let text = "";
  if (done != null && size) text = `${megabytes(done, size)} of ${megabytes(size, size)}`;
  else if (done != null) text = `${megabytes(done, done)} so far`;
  return { fraction, text };
}

/** Bytes in MB, or in GB with one decimal when `scale` (the size) is a gigabyte or more. */
function megabytes(bytes: number, scale: number): string {
  return scale >= 1e9 ? `${(bytes / 1e9).toFixed(1)} GB` : `${Math.round(bytes / 1e6)} MB`;
}

function capital(s: string): string {
  return s ? s[0]!.toUpperCase() + s.slice(1) : s;
}

/** The newest line on screen (the draft while one is being corrected). */
export function lastLine(c: CaptionsState | null): string {
  const lines = c?.lines ?? [];
  return lines.length ? lines[lines.length - 1]!.text : "";
}

/** The source as the settings' select value: "program", "external" or "live:NAME". */
export function sourceValue(s: CaptionSource | undefined): string {
  if (!s || s.type === "program") return "program";
  if (s.type === "external") return "external";
  return `live:${s.name}`;
}

export function sourceOf(value: string): CaptionSource {
  if (value === "external") return { type: "external" };
  if (value.startsWith("live:")) return { type: "liveSource", name: value.slice(5) };
  return { type: "program" };
}

/** The model choices: auto first, then each model with its size and what it suits; English
 * only models are left out for another language. */
export function modelOptions(models: CaptionModel[], language: string): { value: string; label: string }[] {
  const english = (language || "en") === "en";
  return [
    { value: "auto", label: "Automatic (measured on the server)" },
    ...models
      .filter((m) => english || !m.name.endsWith(".en"))
      .map((m) => ({
        value: m.name,
        label: `${m.name} · ${m.sizeMb >= 1000 ? `${(m.sizeMb / 1000).toFixed(1)} GB` : `${m.sizeMb} MB`} · ${m.note}${m.downloaded ? " · on the server" : ""}`,
      })),
  ];
}

/** The configuration with every field the settings form binds filled in (the server leaves
 * defaults out), so a form just loaded is not "changed". */
export function withDefaults(c: CaptionsConfig): CaptionsConfig {
  return {
    ...c,
    enabled: c.enabled ?? false,
    source: c.source ?? { type: "program" },
    language: c.language ?? "en",
    model: c.model ?? "auto",
    words: c.words ?? "",
    keepMusic: c.keepMusic ?? false,
    maxLines: c.maxLines ?? 2,
    style: c.style ?? {},
    audience: c.audience ?? true,
    stage: c.stage ?? false,
    network: c.network ?? false,
  };
}
