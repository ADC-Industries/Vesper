// Presence and edit locks in Oriel (G8, docs/PRESENCE-PLAN.md): what this client has open (each
// page says what it shows, `setOpen`), sent to the server as `presence`; the server's list of
// who has what open and who holds which edit lock (`presenceChanged`); take over; and the lock
// this client lost to someone else (`lost`, shown as a banner until dismissed or taken back).
// Per collection (F08-1): what this client has open names the collection the box shows, and
// badges and banners count only that collection's (an older peer's entries match any).

import type { DocumentKind, DocumentLock, OpenDocument, PresenceList } from "$protocol";
import { untrack } from "svelte";
import { session } from "./session.svelte";
import { siteState } from "./site.svelte";
import { errorText } from "./errors";
import {
  EMPTY_PRESENCE,
  collectionViewers,
  forServer,
  inCollection,
  lockedByOther,
  lostLocks,
  mergeOpen,
  othersWith,
  withCollection,
  type CollectionViewer,
  type OtherViewer,
} from "./presenceLogic";

class PresenceState {
  list = $state.raw<PresenceList>(EMPTY_PRESENCE);
  /** Locks this client held that someone else took over (newest last). */
  lost = $state.raw<DocumentLock[]>([]);
  private sources = new Map<string, OpenDocument[]>();

  /** The server announces `presence`. */
  get supported(): boolean {
    return !!session.server?.capabilities.includes("presence");
  }

  /** The server knows the places `show` and `timers` (and locks props and timers). */
  get browsing(): boolean {
    return !!session.server?.capabilities.includes("presenceBrowsing");
  }

  /** The collection this client shows (the box's active collection; null: not known yet). */
  get collection(): string | null {
    return siteState.codex?.collection ?? null;
  }

  /** This client's id (its edits' `clientId`). */
  get me(): string | null {
    return session.editor?.clientId ?? null;
  }

  /** Follows the connection: the list from `presenceChanged`, read again after a reconnect.
   *  Returns the unsubscribe function. */
  attach(): () => void {
    const conn = session.conn;
    if (!conn) return () => {};
    this.list = EMPTY_PRESENCE;
    this.lost = [];
    // The box switched collections (or its status came in): say so with what is open.
    const offCollection = $effect.root(() => {
      $effect(() => {
        void this.collection;
        untrack(() => this.send());
      });
    });
    const offs = [
      conn.onPresenceChanged((p) => this.apply(p)),
      conn.onSynced(() => void this.load()),
      offCollection,
    ];
    this.send();
    return () => offs.forEach((o) => o());
  }

  private apply(p: PresenceList) {
    const lost = lostLocks(this.list, p, this.me);
    this.list = p;
    if (lost.length) this.lost = [...this.lost.filter((l) => !lost.some((n) => n.kind === l.kind && n.id === l.id)), ...lost];
    // Taken back (or released by the other client): no longer lost.
    this.lost = this.lost.filter((l) => lockedByOther(p, this.me, l.kind, l.id, l.collection ?? null));
  }

  /** Reads the list (an older server without it: stays empty). */
  async load(): Promise<void> {
    const conn = session.conn;
    if (!conn || !this.supported) return;
    try {
      const p = await conn.rest.presence();
      if (session.conn === conn) this.apply(p);
    } catch {
      /* an older server */
    }
  }

  /** What `source` (a page: "edit", "reflow", "library", "collection", "show", "timers")
   *  shows now; `null` when nothing. */
  setOpen(source: string, docs: OpenDocument[] | null): void {
    if (docs?.length) this.sources.set(source, docs);
    else this.sources.delete(source);
    this.send();
  }

  private send() {
    const conn = session.conn;
    if (!conn || !this.supported) return;
    conn.socket.setPresence({
      type: "presence",
      clientId: this.me ?? undefined,
      computer: session.computerName(),
      open: withCollection(forServer(mergeOpen(this.sources.values()), this.browsing), this.collection),
    });
  }

  /** The others with the document open. */
  others(kind: DocumentKind, id: string | null | undefined): OtherViewer[] {
    return id ? othersWith(this.list, this.me, kind, id, this.collection) : [];
  }

  /** The others with the collection open on the Library page, and what they are doing. */
  collectionOthers(collection: string | null | undefined, nameOf: (kind: DocumentKind, id: string) => string | null): CollectionViewer[] {
    return collection ? collectionViewers(this.list, this.me, collection, nameOf) : [];
  }

  /** The document's lock when someone else holds it. */
  lockedBy(kind: DocumentKind, id: string | null | undefined): DocumentLock | null {
    return id ? lockedByOther(this.list, this.me, kind, id, this.collection) : null;
  }

  /** This client lost the document's lock to someone else (and hasn't taken it back). */
  lostTo(kind: DocumentKind, id: string | null | undefined): DocumentLock | null {
    return (id && this.lost.find((l) => l.kind === kind && l.id === id && inCollection(l, this.collection))) || null;
  }

  /** Takes the document's lock from whoever holds it. True when this client holds it now. */
  async takeOver(kind: DocumentKind, id: string): Promise<boolean> {
    const conn = session.conn;
    if (!conn) return false;
    try {
      await conn.rest.takeLock(kind, id, true);
      this.lost = this.lost.filter((l) => !(l.kind === kind && l.id === id));
      await this.load();
      return true;
    } catch (e) {
      conn.pushNotice("error", `Could not take over editing: ${errorText(e)}`);
      return false;
    }
  }

  /** Takes the document's lock when nobody holds it (starting to edit); quiet when someone
   *  does (the list shows who) or the server is older. */
  async claim(kind: DocumentKind, id: string): Promise<void> {
    const conn = session.conn;
    if (!conn || !this.supported) return;
    await conn.rest.takeLock(kind, id).catch(() => {});
  }

  /** Forgets a lost lock's banner (the editor stays read-only while the other client edits). */
  dismissLost(kind: DocumentKind, id: string): void {
    this.lost = this.lost.filter((l) => !(l.kind === kind && l.id === id));
  }
}

export const presence = new PresenceState();

/** Copies typing that couldn't be saved to the clipboard (or, failing that, shows it). */
export async function copyTyping(text: string): Promise<void> {
  const conn = session.conn;
  try {
    await navigator.clipboard.writeText(text);
    conn?.pushNotice("info", "Your typing was copied to the clipboard.");
  } catch {
    conn?.pushNotice("warning", `Oriel could not use the clipboard. What you typed: “${text}”`);
  }
}
