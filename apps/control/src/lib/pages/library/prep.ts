// Media preparation in the Library (docs/MEDIA-PREP-PLAN.md): what a media file's preparation
// says (state, loudness, gain, why it was converted), the badge on its tile, and the trim bar's
// arithmetic. Plain TypeScript, unit tested.

import type { MediaAsset, MediaPrepJob } from "$protocol";

export type PrepTone = "ok" | "busy" | "warn" | "muted";

export interface PrepSummary {
  /** One line: "Converted to the house format", "Converting… 42 %". */
  label: string;
  tone: PrepTone;
  /** Key/value lines for the card. */
  facts: { k: string; v: string }[];
  /** Why it was converted. */
  reasons: string[];
  notes: string[];
  error: string | null;
  /** It plays a prepared copy (the original can be used again). */
  converted: boolean;
}

/** "−23.0 LUFS". */
export function formatLufs(v: number): string {
  return `${v < 0 ? "−" : ""}${Math.abs(v).toFixed(1)} LUFS`;
}

/** "+7.0 dB", "−3.5 dB", "0 dB". */
export function formatGain(v: number): string {
  if (Math.abs(v) < 0.05) return "0 dB";
  return `${v > 0 ? "+" : "−"}${Math.abs(v).toFixed(1)} dB`;
}

/** "1:02.5": minutes, seconds and a tenth. */
export function formatTime(s: number): string {
  const t = Math.max(0, Math.round(s * 10) / 10);
  const m = Math.floor(t / 60);
  const rest = t - m * 60;
  return `${m}:${rest < 10 ? "0" : ""}${rest.toFixed(1)}`;
}

/** Parses "1:02.5", "62.5" or "62" into seconds (null: not a time). */
export function parseTime(text: string): number | null {
  const t = text.trim();
  const m = /^(?:(\d+):)?(\d+(?:\.\d+)?)$/.exec(t);
  if (!m) return null;
  const v = (m[1] ? Number(m[1]) * 60 : 0) + Number(m[2]);
  return Number.isFinite(v) ? v : null;
}

const STEP: Record<string, string> = {
  queued: "Waiting to be prepared",
  analysing: "Analysing…",
  converting: "Converting…",
  waitingForLive: "Converting after the show (nothing runs while live)",
  done: "Prepared",
  failed: "Preparation failed",
};

/** What a media file's preparation says, with its running job if any. */
export function prepSummary(asset: Pick<MediaAsset, "kind" | "prep">, job: MediaPrepJob | null): PrepSummary {
  const p = asset.prep ?? null;
  const facts: { k: string; v: string }[] = [];
  if (p?.loudness != null) facts.push({ k: "Loudness", v: formatLufs(p.loudness) });
  if (p?.gainDb != null) facts.push({ k: "Level at playback", v: formatGain(p.gainDb) });
  if (p?.peak != null) facts.push({ k: "Peak", v: `${p.peak < 0 ? "−" : ""}${Math.abs(p.peak).toFixed(1)} dBFS` });
  if (p?.posterAt != null) facts.push({ k: "Poster frame", v: formatTime(p.posterAt) });
  if (p?.original?.format) facts.push({ k: "Plays", v: p.original.format });
  if (p?.original?.probe?.codec) facts.push({ k: "Original", v: originalLine(p.original.probe) });
  const converted = !!p?.original;
  const base = { facts, reasons: p?.original?.reasons ?? [], notes: p?.notes ?? [], converted };
  if (job && job.step !== "done") {
    const pct = job.step === "converting" ? ` ${Math.round(job.progress * 100)} %` : "";
    return {
      ...base,
      label: `${STEP[job.step] ?? job.step}${pct}`,
      tone: job.step === "failed" ? "warn" : "busy",
      error: job.step === "failed" ? (job.error ?? p?.error ?? "unknown") : null,
    };
  }
  if (p?.error) return { ...base, label: STEP.failed!, tone: "warn", error: p.error };
  if (converted) return { ...base, label: "Converted to the house format", tone: "ok", error: null };
  if (p?.analysed) return { ...base, label: "Ready: plays as it is", tone: "ok", error: null };
  return { ...base, label: "Not prepared yet", tone: "muted", error: null };
}

function originalLine(probe: NonNullable<NonNullable<MediaAsset["prep"]>["original"]>["probe"]): string {
  if (!probe) return "";
  const size = probe.width ? `${probe.width}×${probe.height}` : "";
  const fps = probe.frameRate ? `${Math.round((probe.frameRate.num / probe.frameRate.den) * 100) / 100} fps` : "";
  return [probe.codec?.toUpperCase(), size, fps].filter(Boolean).join(" ");
}

/** The badge on a media tile while a job runs or after a failure (null: none). */
export function prepBadge(job: MediaPrepJob | null, asset: Pick<MediaAsset, "prep">): { text: string; warn: boolean } | null {
  if (job) {
    if (job.step === "converting") return { text: `PREPARING ${Math.round(job.progress * 100)}%`, warn: false };
    if (job.step === "queued" || job.step === "analysing") return { text: "PREPARING", warn: false };
    if (job.step === "waitingForLive") return { text: "AFTER THE SHOW", warn: false };
    if (job.step === "failed") return { text: "PREP FAILED", warn: true };
  }
  if (asset.prep?.error) return { text: "PREP FAILED", warn: true };
  return null;
}

/** The trim as fractions of the clip (0–1) for the bar. */
export function trimSpan(duration: number, trimIn: number | null | undefined, trimOut: number | null | undefined): { a: number; b: number } {
  if (!(duration > 0)) return { a: 0, b: 1 };
  const a = Math.min(1, Math.max(0, (trimIn ?? 0) / duration));
  const b = Math.min(1, Math.max(a, (trimOut ?? duration) / duration));
  return { a, b };
}

/** Why a trim can't be saved (null: it can). */
export function trimProblem(duration: number | null, trimIn: number | null, trimOut: number | null): string | null {
  if (trimIn != null && trimIn < 0) return "The in point can't be before the start.";
  if (trimOut != null && trimOut < 0) return "The out point can't be before the start.";
  if (duration != null && trimIn != null && trimIn > duration) return "The in point is after the end.";
  if (duration != null && trimOut != null && trimOut > duration + 0.05) return "The out point is after the end.";
  if (trimIn != null && trimOut != null && trimOut <= trimIn) return "The out point must come after the in point.";
  return null;
}
