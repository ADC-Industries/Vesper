// A small dispatcher for the server's change events, so each page subscribes to what it shows
// and refetches it: `documentChanged` (a document of the active collection), `codexChanged`
// (collections, storage, trash, export …), `siteChanged` (devices, join requests, pairings …) and
// `siteEvent` (one of the site's typed events, as it happens).
// Plain TypeScript, unit tested.

import type { CodexPart, DocumentKind, SiteEvent, SitePart } from "$protocol";
import type { DocumentChangedMessage } from "../server";

export type ChangeEvent =
  | { type: "documentChanged"; kind: DocumentKind; id: string; msg: DocumentChangedMessage }
  | { type: "codexChanged"; parts: CodexPart[]; collection?: string }
  | { type: "siteChanged"; parts: SitePart[] }
  | { type: "siteEvent"; event: SiteEvent };

export type ChangeType = ChangeEvent["type"];
type Handler<T extends ChangeType> = (e: Extract<ChangeEvent, { type: T }>) => void;

export class ChangeEvents {
  private handlers = new Map<ChangeType, Set<(e: ChangeEvent) => void>>();

  /** Calls `fn` for every event of `type`; returns the unsubscribe function. */
  on<T extends ChangeType>(type: T, fn: Handler<T>): () => void {
    let set = this.handlers.get(type);
    if (!set) this.handlers.set(type, (set = new Set()));
    const h = fn as (e: ChangeEvent) => void;
    set.add(h);
    return () => set.delete(h);
  }

  /** Delivers an event; a handler that throws doesn't stop the others. */
  emit(e: ChangeEvent): void {
    for (const h of [...(this.handlers.get(e.type) ?? [])]) {
      try {
        h(e);
      } catch (err) {
        console.error(`${e.type} handler failed`, err);
      }
    }
  }

  clear(): void {
    this.handlers.clear();
  }
}

/** The app's one dispatcher (fed by the connection in `session.begin`). */
export const changes = new ChangeEvents();
