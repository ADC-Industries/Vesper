// Remembered servers and pairing tokens (localStorage in the web build).

import { LEGACY_STORAGE_PREFIX, STORAGE_PREFIX } from "../config";

export interface KVStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

function defaultStore(): KVStore | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

export interface SavedServer {
  /** Server ID (stable across address changes). */
  id: string;
  name: string;
  /** Origin, e.g. http://booth.local:5555 */
  baseUrl: string;
}

export class Credentials {
  constructor(private kv: KVStore | null = defaultStore()) {}

  private get(key: string): string | null {
    try {
      const v = this.kv?.getItem(STORAGE_PREFIX + key) ?? null;
      if (v !== null || !this.kv) return v;
      // Saved before the rename: move it to the new prefix.
      const legacy = this.kv.getItem(LEGACY_STORAGE_PREFIX + key);
      if (legacy !== null) {
        this.kv.setItem(STORAGE_PREFIX + key, legacy);
        this.kv.removeItem(LEGACY_STORAGE_PREFIX + key);
      }
      return legacy;
    } catch {
      return null;
    }
  }

  private set(key: string, value: string | null): void {
    try {
      this.kv?.removeItem(LEGACY_STORAGE_PREFIX + key);
      if (value === null) this.kv?.removeItem(STORAGE_PREFIX + key);
      else this.kv?.setItem(STORAGE_PREFIX + key, value);
    } catch {
      /* storage blocked: the app still works, it just forgets */
    }
  }

  /** Token for a server ID. */
  token(serverId: string): string | null {
    return this.get(`token.${serverId}`);
  }

  setToken(serverId: string, token: string | null): void {
    this.set(`token.${serverId}`, token);
  }

  lastServer(): SavedServer | null {
    const raw = this.get("lastServer");
    if (!raw) return null;
    try {
      const v = JSON.parse(raw) as SavedServer;
      return typeof v.baseUrl === "string" && typeof v.id === "string" ? v : null;
    } catch {
      return null;
    }
  }

  setLastServer(s: SavedServer | null): void {
    this.set("lastServer", s ? JSON.stringify(s) : null);
  }

  /** Small UI preferences (grid size, panel choices). */
  pref(key: string): string | null {
    return this.get(`pref.${key}`);
  }

  setPref(key: string, value: string): void {
    this.set(`pref.${key}`, value);
  }
}
