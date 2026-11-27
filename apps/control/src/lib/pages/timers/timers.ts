// Timer helpers for the Timers tab: duration and time-of-day entry and display, labels.

import type { TimePeriod, TimerKind, TimerRunState } from "$protocol";

const two = (n: number) => String(n).padStart(2, "0");

/** `m:ss`, or `h:mm:ss` from an hour on (negative values get a leading `-`). */
export function formatDuration(seconds: number): string {
  const neg = seconds < 0;
  const s = Math.round(Math.abs(seconds));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return `${neg ? "-" : ""}${h ? `${h}:${two(m)}` : m}:${two(sec)}`;
}

/**
 * Parses a typed duration: `h:mm:ss`, `m:ss`, or a bare number of **minutes** (`5` = 5:00,
 * `1.5` = 1:30). Returns seconds, or null when it is not a duration.
 */
export function parseDuration(text: string): number | null {
  const t = text.trim();
  if (!t) return null;
  if (/^\d+(\.\d+)?$/.test(t)) return Math.round(Number(t) * 60);
  const parts = t.split(":");
  if (parts.length < 2 || parts.length > 3 || parts.some((p) => !/^\d+$/.test(p))) return null;
  const n = parts.map(Number);
  const [h, m, s] = n.length === 3 ? (n as [number, number, number]) : [0, n[0]!, n[1]!];
  if (s > 59 || (n.length === 3 && m > 59)) return null;
  return h * 3600 + m * 60 + s;
}

/** The target as entered: 24-hour seconds after midnight plus how it is shown. */
export interface TimeOfDay {
  secondsAfterMidnight: number;
  period: TimePeriod;
}

/** `12:05 PM` (AM / PM periods) or `13:05` (24-hour). Seconds are shown only when set. */
export function formatTimeOfDay(secondsAfterMidnight: number, period: TimePeriod = "h24"): string {
  const s = ((Math.round(secondsAfterMidnight) % 86400) + 86400) % 86400;
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const tail = sec ? `:${two(sec)}` : "";
  if (period === "am" || period === "pm") {
    const h12 = h % 12 === 0 ? 12 : h % 12;
    return `${h12}:${two(m)}${tail} ${h < 12 ? "AM" : "PM"}`;
  }
  return `${two(h)}:${two(m)}${tail}`;
}

/**
 * Parses a typed time of day: `13:05`, `1:05 pm`, `1:05p`, `9am`, `7:30:15 AM`. Without
 * AM / PM it is 24-hour. Returns null when it is not a time.
 */
export function parseTimeOfDay(text: string): TimeOfDay | null {
  const m = /^\s*(\d{1,2})(?::(\d{2}))?(?::(\d{2}))?\s*([ap])?\.?\s*m?\.?\s*$/i.exec(text);
  if (!m) return null;
  let h = Number(m[1]);
  const min = Number(m[2] ?? 0);
  const sec = Number(m[3] ?? 0);
  if (min > 59 || sec > 59) return null;
  const ap = m[4]?.toLowerCase();
  if (ap) {
    if (h < 1 || h > 12) return null;
    if (ap === "a" && h === 12) h = 0;
    if (ap === "p" && h !== 12) h += 12;
  } else if (h > 23) return null;
  return { secondsAfterMidnight: h * 3600 + min * 60 + sec, period: ap === "a" ? "am" : ap === "p" ? "pm" : "h24" };
}

/** What the duration / time field of a timer shows. */
export function timerSetting(kind: TimerKind | undefined): string {
  const k = kind ?? { type: "countdown" };
  switch (k.type) {
    case "countdown":
      return formatDuration(k.seconds ?? 0);
    case "countdownToTime":
      return formatTimeOfDay(k.secondsAfterMidnight ?? 0, k.period ?? "h24");
    case "elapsed":
      return k.end != null ? formatDuration(k.end) : formatDuration(k.start ?? 0);
    default:
      return "";
  }
}

/**
 * Applies a typed setting to a timer kind: a duration for countdowns (and the end of elapsed
 * timers; empty clears it), a time of day for countdowns to a time. Null when it does not parse.
 */
export function applySetting(kind: TimerKind | undefined, text: string): TimerKind | null {
  const k = kind ?? { type: "countdown" };
  switch (k.type) {
    case "countdown": {
      const s = parseDuration(text);
      return s === null ? null : { type: "countdown", seconds: s };
    }
    case "countdownToTime": {
      const t = parseTimeOfDay(text);
      if (!t) return null;
      // Typed without AM / PM: keep the timer's own 12-hour display when it has one.
      const period = t.period === "h24" && (k.period === "am" || k.period === "pm") ? (t.secondsAfterMidnight < 43200 ? "am" : "pm") : t.period;
      return { type: "countdownToTime", secondsAfterMidnight: t.secondsAfterMidnight, period };
    }
    case "elapsed": {
      if (!text.trim()) return { type: "elapsed", ...(k.start != null ? { start: k.start } : {}) };
      const s = parseDuration(text);
      return s === null ? null : { ...k, end: s };
    }
    default:
      return null;
  }
}

export const KIND_LABELS: Record<TimerKind["type"], string> = {
  countdown: "Countdown",
  countdownToTime: "Countdown to Time",
  elapsed: "Elapsed Time",
  unknown: "Unknown",
};

/** A timer counts (start shows as pause). */
export function isRunning(s: TimerRunState | undefined): boolean {
  return s === "running" || s === "overrun";
}

/** "Timer N" with the first free N. */
export function nextTimerName(names: string[]): string {
  const taken = new Set(names.map((n) => n.toLowerCase()));
  for (let i = 1; ; i++) if (!taken.has(`timer ${i}`)) return `Timer ${i}`;
}

// ---- the Timers page ----------------------------------------------------------------------------

/** How a timer's value is coloured: counting (green), stopped or paused (grey), over time (red). */
export type TimerTone = "run" | "idle" | "warn" | "over";

export function timerTone(state: TimerRunState | undefined, seconds: number | undefined, warningSeconds?: number | null): TimerTone {
  if (state === "overrun" || state === "expired" || (seconds ?? 0) < 0) return "over";
  if (warningSeconds != null && seconds != null && seconds > 0 && seconds <= warningSeconds && state !== "stopped") return "warn";
  return isRunning(state) ? "run" : "idle";
}

/** The warning field ("Turn amber with this many minutes left"): minutes as typed (decimals
 *  allowed) → seconds; empty → `null` (no warning); anything else → `undefined` (invalid). */
export function warningFrom(input: string): number | null | undefined {
  const t = input.trim().replace(",", ".");
  if (!t) return null;
  const m = Number(t);
  if (!Number.isFinite(m) || m <= 0 || m > 24 * 60) return undefined;
  return Math.round(m * 60);
}

/** The warning field's text for a stored threshold (seconds → minutes). */
export function warningText(seconds: number | null | undefined): string {
  if (seconds == null) return "";
  const m = seconds / 60;
  return Number.isInteger(m) ? String(m) : String(Math.round(m * 100) / 100);
}

/** The status line under the big readout. */
export function timerStatus(kind: TimerKind | undefined, state: TimerRunState | undefined): string {
  const k = kind ?? { type: "countdown" };
  const st = state ?? "stopped";
  if (k.type === "countdownToTime") {
    if (st === "overrun") return "Target time has passed";
    if (st === "expired") return "Target time reached";
    const at = `Counting down to ${formatTimeOfDay(k.secondsAfterMidnight ?? 0, k.period ?? "h24")}`;
    return st === "running" ? at : st === "paused" ? "Paused" : `Ready · ${at.toLowerCase()}`;
  }
  if (k.type === "elapsed") return st === "running" ? "Counting up" : st === "paused" ? "Paused" : st === "expired" ? "Reached its stop time" : "Ready";
  switch (st) {
    case "running":
      return "Running";
    case "paused":
      return "Paused";
    case "overrun":
      return "Over time";
    case "expired":
      return "Time is up";
    default:
      return "Ready";
  }
}

/** One line describing what a timer does (the settings' footer). */
export function timerSummary(kind: TimerKind | undefined, allowOverrun: boolean): string {
  const k = kind ?? { type: "countdown" };
  switch (k.type) {
    case "countdown":
      return `Counts down from ${formatDuration(k.seconds ?? 300)}${allowOverrun ? ", then counts up in red" : ", then stops at zero"}`;
    case "countdownToTime":
      return `Counts down to ${formatTimeOfDay(k.secondsAfterMidnight ?? 0, k.period ?? "h24")} on the server's clock${allowOverrun ? ", then counts up in red" : ""}`;
    case "elapsed":
      return `Counts up from ${formatDuration(k.start ?? 0)} when started${k.end != null ? `, stops at ${formatDuration(k.end)}` : ""}`;
    default:
      return "A timer this version of Oriel can't edit";
  }
}

/** The kind a timer gets when its type changes in the settings. */
export function defaultKind(type: TimerKind["type"]): TimerKind {
  if (type === "countdownToTime") return { type: "countdownToTime", secondsAfterMidnight: 36000, period: "am" };
  if (type === "elapsed") return { type: "elapsed" };
  return { type: "countdown", seconds: 300 };
}

/** A countdown's duration from separate minute and second fields (minutes may be over 59). */
export function durationFrom(minutes: number, seconds: number): number {
  const m = Math.max(0, Math.min(5999, Math.floor(Number.isFinite(minutes) ? minutes : 0)));
  const s = Math.max(0, Math.min(59, Math.floor(Number.isFinite(seconds) ? seconds : 0)));
  return m * 60 + s;
}

/** `HH:MM` for an `<input type="time">` (seconds dropped). */
export function timeInputValue(secondsAfterMidnight: number): string {
  const s = ((Math.round(secondsAfterMidnight) % 86400) + 86400) % 86400;
  return `${two(Math.floor(s / 3600))}:${two(Math.floor((s % 3600) / 60))}`;
}

/** A countdown-to-time kind from an `<input type="time">` value, keeping a 12-hour display. */
export function kindFromTimeInput(kind: TimerKind | undefined, value: string): TimerKind | null {
  const m = /^(\d{2}):(\d{2})(?::(\d{2}))?$/.exec(value);
  if (!m) return null;
  const secs = Number(m[1]) * 3600 + Number(m[2]) * 60 + Number(m[3] ?? 0);
  if (secs >= 86400) return null;
  const old = kind?.type === "countdownToTime" ? kind.period : undefined;
  const period: TimePeriod = old === "am" || old === "pm" ? (secs < 43200 ? "am" : "pm") : "h24";
  return { type: "countdownToTime", secondsAfterMidnight: secs, period };
}

/** The system clock: `2:05:09 PM` or `14:05:09`. */
export function formatClock(d: Date, use24: boolean, seconds = true): string {
  const h = d.getHours();
  const tail = seconds ? `${two(d.getMinutes())}:${two(d.getSeconds())}` : two(d.getMinutes());
  if (use24) return `${two(h)}:${tail}`;
  return `${h % 12 === 0 ? 12 : h % 12}:${tail} ${h < 12 ? "AM" : "PM"}`;
}
