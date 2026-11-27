// Toast rules (docs/ORIEL-REDESIGN-PLAN.md 3.3). Plain TypeScript, unit tested.
//
// One toast per notice key (the connection merges notices with the same key), info and warnings
// go after 2.6 s, errors stay until dismissed, at most three on screen: older ones fold into
// "N more", which opens Diagnostics.

import type { Notice } from "../server";

export const TOAST_MS = 2600;
export const MAX_TOASTS = 3;

/** The newest toasts to show and how many older ones fold into "N more". */
export function visibleToasts<T extends Pick<Notice, "id">>(notices: readonly T[], max = MAX_TOASTS): { shown: T[]; folded: number } {
  if (notices.length <= max) return { shown: [...notices], folded: 0 };
  // With a "N more" line, two toasts fit beside it (three lines in all).
  const keep = Math.max(1, max - 1);
  return { shown: notices.slice(-keep), folded: notices.length - keep };
}

/** How long a notice stays (ms), or null for errors (until dismissed). */
export function toastLifetime(level: Notice["level"]): number | null {
  return level === "error" ? null : TOAST_MS;
}
