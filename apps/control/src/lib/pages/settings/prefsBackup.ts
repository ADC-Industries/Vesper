// Oriel's preferences backup (plan 4.10), plain TypeScript, unit tested: which of Oriel's
// remembered preferences go to the Codex (`PUT /codex/backups/oriel`), the watcher that saves
// them a few seconds after they change (it compares them every couple of seconds, so the
// modules that keep them need no hook), and reading a backup back ("Restore my Oriel
// settings").

/** The preferences backed up (`creds.pref` keys): panel sizes, views, bins, presets. */
export const PREF_KEYS = [
  "layout",
  "thumbPx",
  "slideView",
  "rightTab",
  "binTab",
  "mediaColumns",
  "mediaView",
  "mediaVisible",
  "mediaBin",
  "audioSource",
  "stagePresets",
  "bibleTranslation",
  "bibleVerseNumbers",
  "bibleSplit",
  "bibleRecent",
  "bibleSecondTranslation",
  "bibleSecondLayout",
] as const;

/** `OrielPreferences.schemaVersion` of what `collectPrefs` makes. */
export const PREFS_SCHEMA = 1;

export type Prefs = Record<string, string>;

/** The backed-up preferences as they are now (keys never set are left out). */
export function collectPrefs(read: (key: string) => string | null): Prefs {
  const out: Prefs = {};
  for (const k of PREF_KEYS) {
    const v = read(k);
    if (v !== null) out[k] = v;
  }
  return out;
}

/** The preferences in a backup: `OrielPreferences` (`{schemaVersion, preferences}`) or the bare object. */
export function prefsFromBackup(content: unknown): Prefs | null {
  if (!content || typeof content !== "object") return null;
  const c = content as { schemaVersion?: unknown; preferences?: unknown };
  const body = "preferences" in c && c.preferences && typeof c.preferences === "object" ? c.preferences : content;
  if (typeof c.schemaVersion === "number" && c.schemaVersion > PREFS_SCHEMA) return null;
  const out: Prefs = {};
  for (const k of PREF_KEYS) {
    const v = (body as Record<string, unknown>)[k];
    if (typeof v === "string") out[k] = v;
  }
  return out;
}

/** What restoring a backup leaves: the backup's known preferences over this computer's. */
export function mergePrefs(local: Prefs, backup: Prefs): Prefs {
  const out: Prefs = { ...local };
  for (const k of PREF_KEYS) if (k in backup) out[k] = backup[k]!;
  return out;
}

/** A stable text of the preferences (for "did they change?"). */
export function prefsKey(p: Prefs): string {
  return JSON.stringify(Object.keys(p).sort().map((k) => [k, p[k]]));
}

export interface Timers {
  set(fn: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}
const realTimers: Timers = { set: (fn, ms) => setTimeout(fn, ms), clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>) };

/** Runs the last function given `ms` after the last `trigger`. */
export class Debounce {
  private handle: unknown = null;
  constructor(
    private readonly ms: number,
    private readonly timers: Timers = realTimers,
  ) {}
  get pending(): boolean {
    return this.handle !== null;
  }
  trigger(fn: () => void) {
    this.cancel();
    this.handle = this.timers.set(() => {
      this.handle = null;
      fn();
    }, this.ms);
  }
  cancel() {
    if (this.handle !== null) this.timers.clear(this.handle);
    this.handle = null;
  }
}

/**
 * Saves the preferences `delayMs` after they last changed. `check()` is called every couple of
 * seconds; the first check saves once too (the server answers "unchanged" when nothing is new).
 * A failed save is tried again at the next change or `retry()`.
 */
export class PrefsWatcher {
  private saved: string | null = null;
  private seen: string | null = null;
  private readonly debounce: Debounce;

  constructor(
    private readonly opts: {
      read: () => Prefs;
      save: (p: Prefs) => Promise<boolean>;
      delayMs?: number;
      timers?: Timers;
    },
  ) {
    this.debounce = new Debounce(opts.delayMs ?? 4000, opts.timers);
  }

  check() {
    const now = this.opts.read();
    const key = prefsKey(now);
    if (key === this.seen) return;
    this.seen = key;
    if (key === this.saved) {
      this.debounce.cancel();
      return;
    }
    this.debounce.trigger(() => void this.flush());
  }

  /** Saves now (if anything changed since the last save). */
  async flush(): Promise<void> {
    this.debounce.cancel();
    const now = this.opts.read();
    const key = prefsKey(now);
    if (key === this.saved) return;
    if (await this.opts.save(now).catch(() => false)) this.saved = key;
    else this.seen = null;
  }

  /** The preferences were just restored: they are what the backup holds. */
  markSaved(p: Prefs) {
    this.saved = this.seen = prefsKey(p);
  }

  stop() {
    this.debounce.cancel();
  }
}
