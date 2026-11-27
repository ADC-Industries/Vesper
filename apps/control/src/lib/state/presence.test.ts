import { describe, expect, it } from "vitest";
import type { PresenceList } from "$protocol";
import { collectionSummary, collectionViewers, forServer, inCollection, lockedByOther, lockedTypingNotice, lostLocks, mergeOpen, othersSummary, othersWith, personLabel, withCollection } from "./presenceLogic";

const SONG = "11111111-1111-4111-8111-111111111111";
const list: PresenceList = {
  clients: [
    { session: "s1", clientId: "me", name: "Oriel – Booth laptop", computer: "Booth laptop", open: [{ kind: "presentation", id: SONG, place: "edit" }] },
    { session: "s2", clientId: "pc", name: "Oriel – Booth PC", user: "sam", computer: "Booth PC", open: [{ kind: "presentation", id: SONG, place: "reflow" }] },
    { session: "s3", clientId: "lobby", name: "Oriel – Lobby laptop", open: [{ kind: "presentation", id: SONG, place: "library" }] },
    { session: "s4", clientId: "lobby", name: "Oriel – Lobby laptop", open: [{ kind: "presentation", id: SONG, place: "edit" }] },
    { session: "s5", clientId: "other", name: "Oriel – Office", open: [{ kind: "theme", id: SONG, place: "edit" }] },
  ],
  locks: [{ kind: "presentation", id: SONG, clientId: "pc", name: "Oriel – Booth PC", user: "sam", computer: "Booth PC", since: "2026-11-24T12:00:00Z" }],
};

describe("presence", () => {
  it("names people by user and computer, else the key's name", () => {
    expect(personLabel({ name: "Oriel – Booth PC", user: "sam", computer: "Booth PC" })).toBe("sam on Booth PC");
    expect(personLabel({ name: "Oriel – Booth PC", computer: "Booth PC" })).toBe("Booth PC");
    expect(personLabel({ name: "Oriel – Lobby laptop" })).toBe("Lobby laptop");
    expect(personLabel({ name: "" })).toBe("Someone");
    // Site-wide: a client of another Vesper names its box.
    expect(personLabel({ name: "Oriel – Booth PC", user: "sam", computer: "Booth PC", server: "Chapel Booth" })).toBe("sam on Booth PC at Chapel Booth");
  });

  it("lists the others with a document open, once per client, the editor first", () => {
    const o = othersWith(list, "me", "presentation", SONG);
    expect(o).toEqual([
      { key: "pc", label: "sam on Booth PC", where: "in Reflow", editing: true },
      { key: "lobby", label: "Lobby laptop", where: "in the Library, in Edit", editing: false },
    ]);
    expect(othersSummary(o)).toBe("2 others have this open");
    expect(othersSummary(o.slice(0, 1))).toBe("sam on Booth PC has this open");
    expect(othersSummary([])).toBe("");
    // Kinds don't mix.
    expect(othersWith(list, "me", "theme", SONG).map((x) => x.label)).toEqual(["Office"]);
  });

  it("keys rows by client, as two clients can share a label (D09-3)", () => {
    const twins: PresenceList = {
      clients: [
        { session: "a", clientId: "c1", name: "Oriel – Windows · Chrome", open: [{ kind: "presentation", id: SONG, place: "library" }] },
        { session: "b", clientId: "c2", name: "Oriel – Windows · Chrome", open: [{ kind: "presentation", id: SONG, place: "library" }] },
        { session: "c", name: "Oriel – Windows · Chrome", open: [{ kind: "presentation", id: SONG, place: "edit" }] },
      ],
      locks: [],
    };
    const o = othersWith(twins, "me", "presentation", SONG);
    expect(o.map((x) => x.label)).toEqual(["Windows · Chrome", "Windows · Chrome", "Windows · Chrome"]);
    expect(new Set(o.map((x) => x.key)).size).toBe(3);
  });

  it("reports kept typing with its text when the editor closes (D09-1, D10-1)", () => {
    expect(lockedTypingNotice([])).toBeNull();
    expect(lockedTypingNotice(["The lantern sways", "over the bay"])).toBe(
      "Typing not saved: someone else took over editing. What you typed: “The lantern sways\n\nover the bay”",
    );
  });

  it("knows who else holds a lock, and which locks were taken over", () => {
    expect(lockedByOther(list, "me", "presentation", SONG)?.clientId).toBe("pc");
    expect(lockedByOther(list, "pc", "presentation", SONG)).toBeNull();
    const mine: PresenceList = { ...list, locks: [{ ...list.locks[0]!, clientId: "me" }] };
    expect(lostLocks(mine, list, "me").map((l) => l.clientId)).toEqual(["pc"]);
    expect(lostLocks(mine, { ...list, locks: [] }, "me")).toEqual([]);
    expect(lostLocks(list, mine, "me")).toEqual([]);
  });

  it("merges what every page has open", () => {
    const a = { kind: "presentation" as const, id: SONG, place: "edit" as const };
    expect(mergeOpen([[a], [a, { ...a, place: "library" as const }]])).toEqual([a, { ...a, place: "library" }]);
  });

  it("names the Show page and Timers, and leaves new places out for an older server", () => {
    const TIMER = "33333333-3333-4333-8333-333333333333";
    const l: PresenceList = {
      clients: [
        { session: "s1", clientId: "booth", name: "Oriel – Booth", open: [{ kind: "presentation", id: SONG, place: "show" }, { kind: "timer", id: TIMER, place: "timers" }] },
        { session: "s2", clientId: "new", name: "Oriel – Newer", open: [{ kind: "presentation", id: SONG, place: "other" }] },
      ],
      locks: [],
    };
    expect(othersWith(l, "me", "presentation", SONG).map((o) => o.where)).toEqual(["on the Show page", "elsewhere"]);
    expect(othersWith(l, "me", "timer", TIMER)[0]!.where).toBe("in Timers");
    const open = l.clients[0]!.open;
    expect(forServer(open, true)).toEqual(open);
    expect(forServer([...open, { kind: "presentation", id: SONG, place: "edit" }], false)).toEqual([{ kind: "presentation", id: SONG, place: "edit" }]);
  });

  it("says who else has the collection open and what each is doing", () => {
    const COL = "22222222-2222-4222-8222-222222222222";
    const THEME = "44444444-4444-4444-8444-444444444444";
    const col = { kind: "collection" as const, id: COL, place: "library" as const };
    const l: PresenceList = {
      clients: [
        { session: "s1", clientId: "me", name: "Oriel – Mine", open: [col] },
        { session: "s2", clientId: "pc", name: "Oriel – Booth PC", user: "sam", computer: "Booth PC", open: [col, { kind: "presentation", id: SONG, place: "library" }] },
        { session: "s3", clientId: "lobby", name: "Oriel – Lobby", open: [col, { kind: "theme", id: THEME, place: "library" }] },
        { session: "s4", clientId: "hall", name: "Oriel – Hall", open: [col, { kind: "presentation", id: SONG, place: "show" }] },
        { session: "s5", clientId: "away", name: "Oriel – Away", open: [{ kind: "collection", id: "other", place: "library" }] },
        { session: "s6", clientId: "show", name: "Oriel – Show only", open: [{ kind: "presentation", id: SONG, place: "show" }] },
      ],
      locks: [{ kind: "presentation", id: SONG, clientId: "pc", name: "Oriel – Booth PC", since: "2026-11-24T12:00:00Z" }],
    };
    const names = (_k: string, id: string) => (id === SONG ? "Morning Light" : null);
    const o = collectionViewers(l, "me", COL, names);
    expect(o).toEqual([
      { key: "pc", label: "sam on Booth PC", doing: "editing Morning Light", editing: true },
      { key: "hall", label: "Hall", doing: "browsing", editing: false },
      { key: "lobby", label: "Lobby", doing: "looking at a theme", editing: false },
    ]);
    expect(collectionSummary(o)).toBe("3 others have this collection open");
    expect(collectionSummary(o.slice(1, 2))).toBe("Hall has this collection open");
    expect(collectionSummary([])).toBe("");
  });

  it("counts only the collection shown; an entry without one (an older peer) matches any (F08-1)", () => {
    const A = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const B = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    const doc = { kind: "presentation" as const, id: SONG, place: "edit" as const };
    const l: PresenceList = {
      clients: [
        { session: "s1", clientId: "inA", name: "Oriel – Booth", open: [{ ...doc, collection: A }] },
        { session: "s2", clientId: "inB", name: "Oriel – Chapel", open: [{ ...doc, collection: B }] },
        { session: "s3", clientId: "old", name: "Oriel – Older", open: [doc] },
      ],
      locks: [{ kind: "presentation", id: SONG, clientId: "inB", name: "Oriel – Chapel", since: "2026-11-24T12:00:00Z", collection: B }],
    };
    // Showing A: B's viewer and B's lock are another copy of the song.
    expect(othersWith(l, "me", "presentation", SONG, A).map((o) => o.key)).toEqual(["inA", "old"]);
    expect(lockedByOther(l, "me", "presentation", SONG, A)).toBeNull();
    // Showing B: its lock holder edits, the older client still counts.
    expect(othersWith(l, "me", "presentation", SONG, B)).toEqual([
      { key: "inB", label: "Chapel", where: "in Edit", editing: true },
      { key: "old", label: "Older", where: "in Edit", editing: false },
    ]);
    expect(lockedByOther(l, "me", "presentation", SONG, B)?.clientId).toBe("inB");
    // Collection not known yet, or a lock without one: matches any.
    expect(othersWith(l, "me", "presentation", SONG)).toHaveLength(3);
    const oldLock: PresenceList = { ...l, locks: [{ ...l.locks[0]!, collection: undefined }] };
    expect(lockedByOther(oldLock, "me", "presentation", SONG, A)?.clientId).toBe("inB");
    // A lock lost in B is found again only in B.
    const mine: PresenceList = { ...l, locks: [{ ...l.locks[0]!, clientId: "me" }] };
    expect(lostLocks(mine, l, "me").map((x) => x.collection)).toEqual([B]);
    const inA: PresenceList = { ...l, locks: [{ ...l.locks[0]!, collection: A }] };
    expect(lostLocks(mine, inA, "me")).toEqual([]);
    // The collection viewers' "editing" counts only the shown collection's locks.
    const col = { kind: "collection" as const, id: A, place: "library" as const };
    const v: PresenceList = { clients: [{ session: "s9", clientId: "inB", name: "Oriel – Chapel", open: [col] }], locks: l.locks };
    expect(collectionViewers(v, "me", A, () => "Morning Light")[0]!.doing).toBe("browsing");
    expect(inCollection({}, A)).toBe(true);
    expect(inCollection({ collection: B }, null)).toBe(true);
    expect(inCollection({ collection: B }, A)).toBe(false);
  });

  it("names the collection shown on what this client has open", () => {
    const A = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    const open = [
      { kind: "presentation" as const, id: SONG, place: "edit" as const },
      { kind: "collection" as const, id: A, place: "library" as const },
    ];
    expect(withCollection(open, null)).toEqual(open);
    expect(withCollection(open, A)).toEqual([{ ...open[0], collection: A }, open[1]]);
  });
});
