// The one-time swap of a pairing credential from before site keys for the client's site key.

import type { ServerInfo } from "$protocol";
import { RestClient, type RestOptions } from "./rest";

/** Where the client keeps its key per server. */
export interface TokenStore {
  setToken(serverId: string, token: string | null): void;
}

/**
 * A client paired before site keys sends its old credential once more: the server answers
 * `upgradeCredential`, hands over the client's named site key (`POST /api/v1/pair/upgrade`),
 * and drops the old credential when the key is first used. The key is stored and used from
 * then on, so nobody pairs again. If the swap fails (an older server, or one that can't make
 * keys now), the old credential keeps working and the swap is tried again at the next connect.
 *
 * `opts.browser`: this is a web browser, so its key lives a year (as when pairing).
 *
 * Returns the server info and the token to connect with.
 */
export async function settleCredential(
  baseUrl: string,
  info: ServerInfo,
  token: string | null,
  store: TokenStore,
  opts: Pick<RestOptions, "fetch" | "timeoutMs"> & { browser?: boolean } = {},
): Promise<{ info: ServerInfo; token: string | null }> {
  if (!token || info.auth !== "paired" || info.upgradeCredential !== true) return { info, token };
  const { browser = false, ...rest } = opts;
  let key: string;
  try {
    key = (await new RestClient({ ...rest, baseUrl, token: () => token }).upgradeCredential(browser)).token;
  } catch {
    return { info, token };
  }
  store.setToken(info.id, key);
  try {
    // The key's first use: the server drops the old credential now.
    const next = await new RestClient({ ...rest, baseUrl, token: () => key }).serverInfo();
    return { info: next, token: key };
  } catch {
    return { info, token: key };
  }
}
