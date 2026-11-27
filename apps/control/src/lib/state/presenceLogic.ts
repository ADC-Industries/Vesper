// Presence and edit locks (G8, docs/PRESENCE-PLAN.md): who else has a document open, who holds
// its edit lock, and how people are named. Plain TypeScript, unit tested.

import type { DocumentKind, DocumentLock, OpenDocument, PresenceClient, PresenceList, PresencePlace } from "$protocol";

export const EMPTY_PRESENCE: PresenceList = { clients: [], locks: [] };

/** How a client is named to people: "alex on Booth laptop", "Booth laptop", the key's name
 *  without the "Oriel – " prefix; a client of another Vesper of the site names it ("Booth
 *  laptop at Chapel Booth"). */
export function personLabel(p: { user?: string; computer?: string; name: string; server?: string }): string {
  const name = p.name.replace(/^Oriel\s*[–-]\s*/, "");
  const who = p.user && p.computer ? `${p.user} on ${p.computer}` : (p.user ?? p.computer ?? (name || "Someone"));
  return p.server ? `${who} at ${p.server}` : who;
}

const PLACE_WORD: Record<PresencePlace, string> = {
  edit: "in Edit",
  reflow: "in Reflow",
  library: "in the Library",
  show: "on the Show page",
  timers: "in Timers",
  other: "elsewhere",
};

/** Places where a document is edited (a write from there takes its lock); the Show page only
 *  browses. */
const EDIT_PLACES: ReadonlySet<PresencePlace> = new Set(["edit", "reflow", "library", "timers"]);

/** The places an older server knows (without `presenceBrowsing` it refuses a message naming
 *  another one, so those entries are left out). */
const FIRST_PLACES: ReadonlySet<PresencePlace> = new Set(["edit", "reflow", "library"]);

/** What a server without `presenceBrowsing` can be sent. */
export function forServer(open: OpenDocument[], browsing: boolean): OpenDocument[] {
  return browsing ? open : open.filter((o) => FIRST_PLACES.has(o.place));
}

/**
 * The entry (an open document or a lock) is in collection `c` (F08-1): two collections can hold
 * the same document IDs (a box's re-upload made a second one), and a client shows only its
 * collection's. Either side unknown (an older peer, or no collection known yet) matches any.
 */
export function inCollection(x: { collection?: string }, c: string | null | undefined): boolean {
  return !c || !x.collection || x.collection === c;
}

/** Stamps what this client has open with the collection it shows (`null`: not known yet). The
 *  Library page's `collection` entry names it already. */
export function withCollection(open: OpenDocument[], c: string | null | undefined): OpenDocument[] {
  return c ? open.map((o) => (o.kind === "collection" || o.collection ? o : { ...o, collection: c })) : open;
}

/** One other person with the document open. */
export interface OtherViewer {
  /** The client (its ID, or its session): unique, unlike the label (D09-3). */
  key: string;
  label: string;
  /** Where: "in Edit", "in Reflow, in the Library". */
  where: string;
  /** Holds the edit lock. */
  editing: boolean;
}

/** The others (not `me`) with the document open in collection `coll`, one per client, editors
 *  first. */
export function othersWith(list: PresenceList, me: string | null, kind: DocumentKind, id: string, coll: string | null = null): OtherViewer[] {
  const lock = lockOf(list, kind, id, coll);
  const byClient = new Map<string, { c: PresenceClient; places: Set<PresencePlace> }>();
  for (const c of list.clients) {
    if (me && c.clientId === me) continue;
    const places = c.open.filter((o) => o.kind === kind && o.id === id && inCollection(o, coll)).map((o) => o.place);
    if (!places.length) continue;
    const key = c.clientId ?? c.session;
    const e = byClient.get(key) ?? { c, places: new Set<PresencePlace>() };
    for (const p of places) e.places.add(p);
    e.c = c;
    byClient.set(key, e);
  }
  const out = [...byClient.entries()].map(([key, { c, places }]) => ({
    key,
    label: personLabel(c),
    where: [...places].map((p) => PLACE_WORD[p] ?? PLACE_WORD.other).join(", "),
    editing: !!lock && lock.clientId === key,
  }));
  return out.sort((a, b) => Number(b.editing) - Number(a.editing) || a.label.localeCompare(b.label));
}

/**
 * The notice for typing kept after a lost edit lock when its editor closes (D09-1, D10-1): the
 * typing is never dropped silently, so the notice carries the text. `null`: nothing kept.
 */
export function lockedTypingNotice(texts: string[]): string | null {
  const t = texts.filter((x) => x !== "");
  if (!t.length) return null;
  return `Typing not saved: someone else took over editing. What you typed: “${t.join("\n\n")}”`;
}

/** "2 others have this open", "Booth laptop has this open". */
export function othersSummary(others: OtherViewer[]): string {
  if (!others.length) return "";
  if (others.length === 1) return `${others[0]!.label} has this open`;
  return `${others.length} others have this open`;
}

/** The document's edit lock in collection `c`, if anyone holds it. */
export function lockOf(list: PresenceList, kind: DocumentKind, id: string, c: string | null = null): DocumentLock | null {
  return list.locks.find((l) => l.kind === kind && l.id === id && inCollection(l, c)) ?? null;
}

/** The lock (in collection `c`) when someone other than `me` holds it. */
export function lockedByOther(list: PresenceList, me: string | null, kind: DocumentKind, id: string, c: string | null = null): DocumentLock | null {
  const l = lockOf(list, kind, id, c);
  return l && l.clientId !== me ? l : null;
}

/** The locks `me` held in `before` that someone else holds in `after` (taken over), each in
 *  the collection it was in. */
export function lostLocks(before: PresenceList, after: PresenceList, me: string | null): DocumentLock[] {
  if (!me) return [];
  return before.locks
    .filter((l) => l.clientId === me)
    .map((l) => lockOf(after, l.kind, l.id, l.collection ?? null))
    .filter((l): l is DocumentLock => !!l && l.clientId !== me);
}

/** Every source's open documents as one list (each document and place once). */
export function mergeOpen(sources: Iterable<OpenDocument[]>): OpenDocument[] {
  const seen = new Set<string>();
  const out: OpenDocument[] = [];
  for (const list of sources)
    for (const o of list) {
      const k = `${o.kind}|${o.id}|${o.place}`;
      if (seen.has(k)) continue;
      seen.add(k);
      out.push(o);
    }
  return out.slice(0, 32);
}

/** One other person with the collection open on the Library page, and what they are doing. */
export interface CollectionViewer {
  key: string;
  label: string;
  /** "editing Morning Light", "browsing". */
  doing: string;
  editing: boolean;
}

/**
 * The others with the collection open (Library page, `{kind: "collection", place: "library"}`)
 * and what each is doing: "editing *X*" for the documents whose lock they hold, else "browsing"
 * (or "looking at *X*" for what they have open in the Library). `nameOf` names a document
 * (null: unknown, shown as "a song", "a theme", …).
 */
export function collectionViewers(
  list: PresenceList,
  me: string | null,
  collection: string,
  nameOf: (kind: DocumentKind, id: string) => string | null,
): CollectionViewer[] {
  const seen = new Set<string>();
  const out: CollectionViewer[] = [];
  for (const c of list.clients) {
    if (me && c.clientId === me) continue;
    if (!c.open.some((o) => o.kind === "collection" && o.id === collection && o.place === "library")) continue;
    const key = c.clientId ?? c.session;
    if (seen.has(key)) continue;
    seen.add(key);
    const named = (kind: DocumentKind, id: string) => nameOf(kind, id) ?? `a ${KIND_WORD[kind] ?? "document"}`;
    const held = list.locks.filter((l) => l.clientId === key && inCollection(l, collection));
    const looking = c.open.filter((o) => o.kind !== "collection" && EDIT_PLACES.has(o.place) && inCollection(o, collection));
    const doing = held.length
      ? `editing ${held.map((l) => named(l.kind, l.id)).join(", ")}`
      : looking.length
        ? `looking at ${[...new Set(looking.map((o) => named(o.kind, o.id)))].join(", ")}`
        : "browsing";
    out.push({ key, label: personLabel(c), doing, editing: held.length > 0 });
  }
  return out.sort((a, b) => Number(b.editing) - Number(a.editing) || a.label.localeCompare(b.label));
}

const KIND_WORD: Partial<Record<DocumentKind, string>> = {
  presentation: "song",
  theme: "theme",
  prop: "prop",
  timer: "timer",
  stageLayout: "stage layout",
  playlistNode: "playlist",
};

/** "2 others have this collection open", "Booth laptop has this collection open". */
export function collectionSummary(others: { label: string }[]): string {
  if (!others.length) return "";
  if (others.length === 1) return `${others[0]!.label} has this collection open`;
  return `${others.length} others have this collection open`;
}
