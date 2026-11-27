// Pairing that waits for a site admin's approval (no code typed, or a server of the site that
// isn't the site's server): ask again until it is answered.

import type { PairPending, PairResponse } from "$protocol";
import { ApiRequestError, type RestClient } from "./rest";

/** The pairing waits for approval. */
export function isPending(r: PairResponse | PairPending): r is PairPending {
  return "pairingId" in r;
}

/** Set `cancelled` to stop waiting (the client went back). */
export interface WaitControl {
  cancelled: boolean;
}

/** Why a pairing ended without a key, for people. */
export function pairingEndedText(e: unknown): string | null {
  if (!(e instanceof ApiRequestError)) return null;
  if (e.code === "forbidden") {
    const why = e.message.split(": ").slice(1).join(": ").trim();
    return `The site's administrator refused this computer${why && why !== "not approved" ? ` (${why})` : ""}.`;
  }
  if (e.code === "notFound")
    return "No one approved this computer in time. Ask again, or enter the code shown on the server.";
  return null;
}

/**
 * Waits for a pending pairing's answer, asking every `intervalMs`: the key once approved,
 * `null` when cancelled. Throws `ApiRequestError` when it is refused (`forbidden`) or ended
 * unanswered (`notFound`); a server out of reach for a moment is asked again.
 */
export async function awaitPairing(
  rest: RestClient,
  pending: PairPending,
  ctl: WaitControl,
  intervalMs = 1000,
): Promise<PairResponse | null> {
  for (;;) {
    await new Promise((r) => setTimeout(r, intervalMs));
    if (ctl.cancelled) return null;
    let r: PairResponse | PairPending;
    try {
      r = await rest.pairingStatus(pending.pairingId);
    } catch (e) {
      if (e instanceof ApiRequestError && (e.code === "network" || e.code === "timeout")) continue;
      throw e;
    }
    if (ctl.cancelled) return null;
    if (!isPending(r)) return r;
  }
}
