// "Synced to 2 of 3 Vespers" (G14): the Codex box's view of the show boxes following its
// collections, in words. Unit tested.

import type { CodexBoxSync, CollectionBoxes } from "$protocol";

/** "Synced to 2 of 3 Vespers" for a collection (null when no Vesper shows it, or on a show box,
 * which doesn't know). `ok`: every one is up to date. */
export function syncedText(b: CollectionBoxes | undefined | null): { text: string; ok: boolean } | null {
  if (!b || !b.showing) return null;
  return { text: `Synced to ${b.synced} of ${b.showing} Vesper${b.showing === 1 ? "" : "s"}`, ok: b.synced === b.showing };
}

/** A show box's state in words, with its dot. */
export function boxState(b: Pick<CodexBoxSync, "online" | "behind">): { status: "ok" | "warn" | "off"; text: string } {
  if (!b.online) return { status: "off", text: "Not heard from" };
  if (b.behind == null) return { status: "warn", text: "Copying" };
  if (b.behind === 0) return { status: "ok", text: "Up to date" };
  return { status: "warn", text: `${b.behind} change${b.behind === 1 ? "" : "s"} behind` };
}
