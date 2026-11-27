// Media transport helpers: time labels, the position shown between state updates (the server
// refreshes positions at most 4 times a second), and scrub-bar maths.

import type { MediaTransport } from "$protocol";

/** `m:ss` (or `h:mm:ss`), rounding down; negative and unknown values show as `0:00`. */
export function formatTime(seconds: number | null | undefined): string {
  const s = Math.max(0, Math.floor(Number.isFinite(seconds) ? (seconds as number) : 0));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const ss = String(s % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** Where a playing video is `elapsedMs` after the state that reported `t` (paused and stopped
 * videos stay put). Positions count from the start of the played range; a loop wraps, a
 * single play holds at the end. */
export function positionAt(t: MediaTransport, elapsedMs: number): number {
  if (t.state !== "playing") return t.position;
  const p = t.position + (Math.max(0, elapsedMs) / 1000) * (t.rate || 1);
  const d = t.duration;
  if (d == null || d <= 0) return p;
  return t.loop ? p % d : Math.min(p, d);
}

/** Fraction 0–1 of the bar for `position` (unknown duration: 0). */
export function fraction(position: number, duration: number | null | undefined): number {
  if (duration == null || !(duration > 0)) return 0;
  return Math.min(1, Math.max(0, position / duration));
}

/** The position under a pointer at `clientX` on a bar spanning `left`..`left + width`. */
export function seekTarget(clientX: number, left: number, width: number, duration: number | null | undefined): number {
  if (duration == null || !(duration > 0) || !(width > 0)) return 0;
  return Math.min(1, Math.max(0, (clientX - left) / width)) * duration;
}

/** `0:12 / 1:30` */
export function timeLabel(position: number, duration: number | null | undefined): string {
  return duration == null ? formatTime(position) : `${formatTime(position)} / ${formatTime(duration)}`;
}

/**
 * The item `d` steps from `current` in `ids` (wrapping); when `current` is not in the list,
 * the first (forward) or last (back) one. `null` for an empty list.
 */
export function neighbour(ids: readonly string[], current: string | null | undefined, d: 1 | -1): string | null {
  if (!ids.length) return null;
  const i = current ? ids.indexOf(current) : -1;
  if (i < 0) return d > 0 ? ids[0]! : ids[ids.length - 1]!;
  return ids[(i + d + ids.length) % ids.length]!;
}

/** "Previous" restarts the item when it has played this long (seconds), else steps back. */
export const RESTART_AFTER = 3;

/** `29.97`, `30`, `59.94` */
export function rateLabel(num: number, den: number): string {
  const r = num / Math.max(1, den);
  return Number.isInteger(r) ? String(r) : r.toFixed(2);
}

/** What Previous / Next do (`stepAction`). */
export type StepAction = { kind: "restart" } | { kind: "server"; next: boolean } | { kind: "trigger"; asset: string } | null;

/**
 * Previous (`d` −1) or Next (+1) of the transport. Previous restarts an item that has played
 * longer than `RESTART_AFTER`. A video on the Slide layer (a foreground cue) isn't part of the
 * bin: Previous only restarts it, Next does nothing. Otherwise the bin's list is stepped
 * (`neighbour`), or, for audio with a playlist (`serverSteps`), the server steps.
 */
export function stepAction(o: {
  d: 1 | -1;
  target: { asset: string; layer: string } | null;
  position: number;
  list: readonly string[];
  serverSteps?: boolean;
}): StepAction {
  const { d, target } = o;
  if (d < 0 && target && o.position > RESTART_AFTER) return { kind: "restart" };
  if (target?.layer === "slide") return d < 0 ? { kind: "restart" } : null;
  if (o.serverSteps) return { kind: "server", next: d > 0 };
  const next = neighbour(o.list, target?.asset, d);
  if (!next) return d < 0 && target ? { kind: "restart" } : null;
  return { kind: "trigger", asset: next };
}
