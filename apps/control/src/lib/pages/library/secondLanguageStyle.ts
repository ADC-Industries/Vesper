// Small helpers of the theme's second-language style section (ThemeSecondLanguage.svelte).
import { ApiRequestError } from "../../server";

/** A short message for a failed load or save. */
export function errorText(e: unknown): string {
  if (e instanceof ApiRequestError && e.code === "conflict") return "Someone else changed this theme meanwhile: it shows their version now; make the change again.";
  return e instanceof Error ? e.message : String(e);
}

/**
 * Saves run one after another, each with the revision the one before it got (`If-Match`), so
 * quick changes never send a stale revision (H11-8). After a failure the saves queued behind it
 * are dropped (the caller reloads); `reset` takes a freshly loaded revision.
 */
export class SerialSaves<T> {
  revision: string | null = null;
  /** Saves waiting or in flight. */
  pending = 0;
  private chain: Promise<void> = Promise.resolve();
  private generation = 0;

  constructor(
    private readonly send: (value: T, revision: string | null) => Promise<string>,
    private readonly failed: (e: unknown) => void,
    private readonly settled: () => void = () => {},
  ) {}

  save(value: T): Promise<void> {
    const gen = this.generation;
    this.pending++;
    this.chain = this.chain.then(async () => {
      try {
        if (gen !== this.generation) return;
        const rev = await this.send(value, this.revision);
        // A reload meanwhile (another theme, a rename) brought its own revision.
        if (gen === this.generation) this.revision = rev;
      } catch (e) {
        this.generation++;
        this.failed(e);
      } finally {
        this.pending--;
        this.settled();
      }
    });
    return this.chain;
  }

  /** A fresh load: its revision; saves still queued from before are dropped. */
  reset(revision: string | null) {
    this.generation++;
    this.revision = revision;
  }
}
