// What the operator sees when opening a presentation fails. A presentation that no longer
// exists (deleted by someone, an undone "New Presentation", a stale live or remembered ID) is
// not an error: the selection is cleared quietly, with at most an info line when the operator
// asked for it. Anything else (offline, server error) is a real error.

import { ApiRequestError } from "../server";

export interface OpenFailure {
  /** The presentation is gone: clear the selection / open state that points at it. */
  missing: boolean;
  /** Toast to show, or null. */
  notice: { level: "info" | "error"; message: string } | null;
}

export function openFailure(e: unknown, quiet: boolean): OpenFailure {
  if (e instanceof ApiRequestError && e.code === "notFound") {
    return {
      missing: true,
      notice: quiet ? null : { level: "info", message: "That presentation no longer exists (it was deleted)." },
    };
  }
  const msg = e instanceof Error ? e.message : String(e);
  return { missing: false, notice: { level: "error", message: `Could not open the presentation: ${msg}` } };
}
