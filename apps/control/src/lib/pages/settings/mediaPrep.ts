// Settings ▸ Media's choices for media preparation (docs/MEDIA-PREP-PLAN.md, "Settings"): the
// options with what they mean on this box (the quality presets' bit rates at 1080p30, the
// encoders this box has), and the explicit bit rate typed in Mb/s. Plain TypeScript, unit tested.

import type { HouseVideoCodec, MediaPrepStatus, PrepEncoder, PrepMaxSize, PrepPriority, PrepQuality, PrepWhileLive } from "$protocol";

export interface Choice<V extends string> {
  value: V;
  label: string;
  disabled?: boolean;
}

/** IDN's converter's own bit rate for 1080p30 H.264, Mb/s; the presets are multiples of it. */
const IDN_1080P30 = 7.5;
/** The presets' factors over IDN's own (as the server's `plan::quality_factor`). */
export const QUALITY_FACTOR: Record<PrepQuality, number> = { standard: 1.6, high: 8 / 3, maximum: 4 };

/** A preset's bit rate for 1080p30, Mb/s (HEVC 0.6 of H.264's), rounded to a whole one. */
export function presetMbps(q: PrepQuality, codec: HouseVideoCodec): number {
  const h264 = IDN_1080P30 * QUALITY_FACTOR[q];
  return Math.round(codec === "hevc" ? h264 * 0.6 : h264);
}

export function qualityChoices(codec: HouseVideoCodec): Choice<PrepQuality>[] {
  const name: Record<PrepQuality, string> = { standard: "Standard", high: "High", maximum: "Maximum" };
  return (["standard", "high", "maximum"] as const).map((q) => ({
    value: q,
    label: `${name[q]} (${presetMbps(q, codec)} Mb/s at 1080p30)`,
  }));
}

export const CODEC_CHOICES: Choice<HouseVideoCodec>[] = [
  { value: "h264", label: "H.264 (every box decodes it)" },
  { value: "hevc", label: "HEVC (smaller; needs HEVC decoding everywhere)" },
];

export const SIZE_CHOICES: Choice<PrepMaxSize>[] = [
  { value: "keep", label: "Keep its size (larger than 4K: 4K)" },
  { value: "1080p", label: "At most 1080p" },
  { value: "2160p", label: "At most 4K" },
];

export const PRIORITY_CHOICES: Choice<PrepPriority>[] = [
  { value: "low", label: "Low" },
  { value: "normal", label: "Normal" },
];

export const LIVE_CHOICES: Choice<PrepWhileLive>[] = [
  { value: "lowPriority", label: "Go on at the lowest priority" },
  { value: "wait", label: "Wait until nothing is live" },
];

/** A codec's encoders on this box, as the encoder choices take them. */
export type Encoders = Pick<MediaPrepStatus, "gpuEncoder" | "cpuEncoder">;

/** This box's encoders for `codec` (the one being edited, saved or not); a server that lists
 *  only the saved codec's gives those. */
export function encodersFor(st: Pick<MediaPrepStatus, "gpuEncoder" | "cpuEncoder" | "codecEncoders">, codec: HouseVideoCodec): Encoders {
  const c = st.codecEncoders?.[codec];
  const e: Encoders = {};
  const gpu = c ? c.gpu : st.gpuEncoder;
  const cpu = c ? c.cpu : st.cpuEncoder;
  if (gpu) e.gpuEncoder = gpu;
  if (cpu) e.cpuEncoder = cpu;
  return e;
}

/** The encoder choices, with what this box has (a choice it has nothing for is marked). */
export function encoderChoices(st: Encoders): Choice<PrepEncoder>[] {
  const gpu = st.gpuEncoder ?? null;
  const cpu = st.cpuEncoder ? st.cpuEncoder.replace(/ \(CPU\)$/, "") : null;
  return [
    { value: "auto", label: `Auto (${gpu ? "GPU" : cpu ? "CPU" : "none here"})` },
    { value: "gpu", label: gpu ? `GPU: ${gpu}` : "GPU (none on this box)" },
    { value: "cpu", label: cpu ? `CPU: ${cpu}` : "CPU (none on this box for this codec)" },
  ];
}

/** What this box has, for the line next to the encoder choice. */
export function encodersLine(st: Encoders): string {
  const gpu = st.gpuEncoder ? `GPU ${st.gpuEncoder}` : "no GPU encoder";
  const cpu = st.cpuEncoder ? `CPU ${st.cpuEncoder.replace(/ \(CPU\)$/, "")}` : "no CPU encoder";
  return `This box: ${gpu} · ${cpu}`;
}

/** A problem with the chosen encoder on this box (the videos then aren't converted), or null. */
export function encoderProblem(choice: PrepEncoder, st: Encoders): string | null {
  if (choice === "gpu" && !st.gpuEncoder) return "This box has no GPU encoder for this codec: videos would be analysed, not converted.";
  if (choice === "cpu" && !st.cpuEncoder) return "This box has no CPU encoder for this codec: videos would be analysed, not converted.";
  if (!st.gpuEncoder && !st.cpuEncoder) return "This box has no encoder for this codec: videos are analysed, not converted.";
  return null;
}

/** The explicit bit rate typed in Mb/s, as the settings' kbit/s (1–60 Mb/s); `undefined`: by
 *  the quality preset (empty); `null`: not a number. */
export function parseMbps(text: string): number | undefined | null {
  const t = text.trim().replace(",", ".");
  if (!t) return undefined;
  const v = Number(t);
  if (!Number.isFinite(v) || v <= 0) return null;
  return Math.round(Math.min(60, Math.max(1, v)) * 1000);
}

/** kbit/s shown as Mb/s in the field ("" for none), exactly as saved (7250 → "7.25"): read back
 *  by `parseMbps`, it gives the same kbit/s, so a saved form isn't changed by showing it. */
export function formatMbps(kbps: number | undefined): string {
  if (kbps === undefined) return "";
  return String(Math.round(kbps) / 1000);
}

/** Two settings objects alike (whatever the order of their keys; a key absent or undefined is
 *  the same). */
export function sameSettings(a: object, b: object): boolean {
  const canon = (o: object) =>
    JSON.stringify(
      Object.entries(o)
        .filter(([, v]) => v !== undefined)
        .sort(([x], [y]) => (x < y ? -1 : x > y ? 1 : 0)),
    );
  return canon(a) === canon(b);
}
