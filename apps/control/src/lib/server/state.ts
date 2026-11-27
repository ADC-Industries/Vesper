// The show state as a Svelte store, ordered by revision.
//
// The server sends the full ShowState on connect and after every change, with a revision
// that only goes up for the lifetime of that server process. A client can see revisions out
// of order only across connections (a restarted server starts again from a low number), so:
//
// - within one connection a state is applied only if its revision is newer;
// - `resync()` is called on every new connection, and the next state is then accepted
//   whatever its revision.

import type { ShowState } from "$protocol";
import type { Readable, Subscriber, Unsubscriber } from "svelte/store";

export interface RevisionedState {
  revision: number;
  state: ShowState;
}

export interface ShowStore extends Readable<RevisionedState | null> {
  /** Applies a state if it is newer. Returns whether it was applied. */
  apply(revision: number, state: ShowState): boolean;
  /** Accept the next state whatever its revision (call on every new connection). */
  resync(): void;
  /** The current value without subscribing. */
  readonly current: RevisionedState | null;
  /** Revision of the current state (-1 before the first). */
  readonly revision: number;
  /** Resolves once a state with at least this revision is applied. */
  waitFor(revision: number): Promise<void>;
}

export function createShowStore(): ShowStore {
  let value: RevisionedState | null = null;
  let resyncing = true;
  const subs = new Set<Subscriber<RevisionedState | null>>();
  const waiters: { revision: number; resolve: () => void }[] = [];

  function set(v: RevisionedState) {
    value = v;
    for (const s of subs) s(v);
    for (let i = waiters.length - 1; i >= 0; i--) {
      const w = waiters[i]!;
      if (v.revision >= w.revision) {
        waiters.splice(i, 1);
        w.resolve();
      }
    }
  }

  return {
    subscribe(run: Subscriber<RevisionedState | null>): Unsubscriber {
      subs.add(run);
      run(value);
      return () => subs.delete(run);
    },
    apply(revision, state) {
      if (!resyncing && value && revision <= value.revision) return false;
      resyncing = false;
      set({ revision, state });
      return true;
    },
    resync() {
      resyncing = true;
      // Revisions from the previous connection mean nothing now.
      for (const w of waiters.splice(0)) w.resolve();
    },
    get current() {
      return value;
    },
    get revision() {
      return value?.revision ?? -1;
    },
    waitFor(revision) {
      if (value && value.revision >= revision) return Promise.resolve();
      return new Promise((resolve) => waiters.push({ revision, resolve }));
    },
  };
}

/**
 * A derived store that only notifies when the selected value changes (by `===`, or a custom
 * comparison). State arrives several times per second (fps counters), and most of the UI
 * only cares about one small part of it.
 */
export function select<T>(
  source: Readable<RevisionedState | null>,
  pick: (s: ShowState | null) => T,
  equal: (a: T, b: T) => boolean = Object.is,
): Readable<T> {
  return {
    subscribe(run) {
      let first = true;
      let last: T;
      return source.subscribe((v) => {
        const next = pick(v?.state ?? null);
        if (first || !equal(last, next)) {
          first = false;
          last = next;
          run(next);
        }
      });
    },
  };
}

/** Structural equality for small JSON values (layer states, cursor). */
export function jsonEqual<T>(a: T, b: T): boolean {
  return a === b || JSON.stringify(a) === JSON.stringify(b);
}
