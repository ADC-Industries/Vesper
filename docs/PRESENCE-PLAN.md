# Presence and edit locks (G8)

Status: stage 0 (this note), stage 1 (presence) and stage 2 (edit locks per box) were built in
batch 8; stage 3 (site-wide over the box-to-Codex link) in batch 9 (lane C, "Site-wide" below);
browsing (playlists, props, timers, the Show page) and "2 others have this collection open" in
batch 11 (lane O, "Browsing" below).

**Built.** Server: `crates/vesper-server/src/presence.rs` (sessions, locks, the lock check
middleware), `api/presence.rs` (routes), `ws.rs` (the `presence` message, expiry with the
socket); tests in `tests/server/presence.rs`. Oriel: `state/presence.svelte.ts` and
`presenceLogic.ts`, `components/PresenceBadge.svelte` (Edit and Reflow headers, the Library's
item) and `LockBanner.svelte`, "*X* is editing" on the Library's rows, a read-only editor and
Reflow while someone else holds the lock, kept typing in the Reflow queue (`423` → status
`locked`) and in the slide editor (`EditState.lockedEdits`), sent when the lock is back;
stub `test/stub/routes/presence.mjs`, shots `scripts/shots/presence.mjs`. Design data: the
Library board ("*X* is editing", "*X* has this song open for editing … Take over editing",
"2 others have this library open" with who is browsing or editing what).

## Who a client is

Today a request names its client three ways: the **site key** (`Access::Paired`, its id and its
name `Oriel – Booth laptop`, or the server machine itself), the **site user** it signed in as
(access level `users`), and the **`clientId`** Oriel makes per run (`Editor.clientId`, sent as
`X-Client-Id` and in every edit body, echoed in `documentChanged`). The WebSocket hello also
carries `clientName` (Oriel sends the computer's name).

Presence is per **WebSocket connection** (a *session*, a server-made id such as `s12`), because
the connection is what the heartbeat watches (X3-7: pings every 20 s, dropped after 60 s of
silence). A session reports its `clientId`, so REST writes (which carry `X-Client-Id`) can be
tied to it. A client is shown as the site user when signed in, else the computer, else the
key's name: "Alex on Booth laptop", "Booth laptop".

## Presence (stage 1)

- Client → server, over the WebSocket, whenever it changes and after every (re)connect:
  `{"type":"presence","clientId":"…","computer":"Booth laptop","open":[{"kind":"presentation","id":"…","place":"edit"}]}`.
  The list replaces the session's previous one. `place` is `edit` (slide editor), `reflow` or
  `library` (the Library page, viewing or editing). Kinds: `presentation`, `theme`,
  `stageLayout` (any `DocumentKind` is accepted; at most 32 entries).
- Server: `GET /api/v1/presence` → `PresenceList {clients, locks}`; each client
  `{session, clientId?, name, user?, computer?, open}`. Every change (a presence message that
  changes something, a session opening or closing, a lock taken, taken over or released) is
  broadcast as `{"type":"presenceChanged","presence":{…}}` with the whole list (a handful of
  clients: no diffs). A session's entry goes when its socket closes, or when the heartbeat
  gives up on it (60 s).
- Oriel: "*N* others have this open" next to the title in the Edit and Reflow headers and on
  the Library's item; a tooltip / list names them ("Alex on Booth laptop · editing"); the
  Library's rows say "*X* is editing" in blue for a document someone else holds the lock of.

## Edit locks (stage 2)

**Recommendation: hard locks, always take-over-able.** The board makes a locked song view-only
with "Take over editing", which is a hard lock in the UI; enforcing it on the server keeps
other clients (an older Oriel, a script) from typing into a document someone is editing. A hard
lock can't strand anyone because anyone who may edit can take it over at once, and it ends
with its holder's connection.

- **Taking.** A write to a presentation (edit ops, move, delete), a theme (edit ops, rename,
  delete) or a stage layout (edit ops, update, delete) from a `clientId` with a live session
  takes the document's lock for that `clientId` (on the first edit: opening alone doesn't).
  Oriel also takes it explicitly when the operator starts editing in the Library
  (`POST /api/v1/presence/locks {kind, id}`).
- **Refused.** The same writes from anyone else while the lock is held answer **`423 locked`**
  (new `ErrorCode::Locked`); the message names the holder, and `GET /presence` has the lock.
  A write without a `clientId`, or from a `clientId` with no live session (scripts, the
  forwarded write of a show box's client), never takes a lock but is refused by one.
- **Take over.** `POST /api/v1/presence/locks {kind, id, takeOver: true}` moves the lock to the
  caller (any client that may edit; no admin needed) → `DocumentLock`; without `takeOver` a
  held lock answers `423`. `DELETE /api/v1/presence/locks/{kind}/{id}` releases the caller's.
- **Expiry.** A lock goes when its holder closes the document (its presence no longer lists
  it), or when the holder's last session with that `clientId` ends (socket closed, or 60 s of
  heartbeat silence). Nothing is persisted: a restart frees every lock.
- **The client that lost the lock** sees, from `presenceChanged`, the banner "*Alex* took over
  editing" and its editor turns read-only. Typing still waiting in its edit queue is **kept,
  never dropped**: the queue gets `423` for it, pauses those slides (status `locked`, like a
  conflict), and the banner offers **Take back** (take over again; the queue sends the kept
  typing) or **Copy my typing** (to the clipboard, then the queue drops it). Closing the editor
  with kept typing reports it with the text, as conflicts already do.
- **With revisions and conflicts.** Locks sit in front of the existing rules and change
  nothing in them: `expectText`, `If-Match` and the `409 conflict` prompts still apply (two
  boxes in stage 2, or a take-over racing a save, fall back to them). A write that passes the
  lock check is the same write as before.
- **Others** (not holding it) see "*X* is editing" and a read-only view with **Take over
  editing**, as on the board.
- **Per collection** (bug run 4, F08-1). A lock is in the collection its write or take is for:
  a show box's forwarded collection on the Codex, else the box's active collection; the
  box-to-Codex `LOCK` request carries the box's collection (`collection`, optional). Two
  collections holding the same document IDs (a box that re-uploads its collection after
  leaving and rejoining the site makes a second one) don't block each other, and taking over
  in one leaves the other's lock alone. A lock or request without a collection (an older show
  box) matches the document in any collection. Presence is per collection too (batch 14):
  `OpenDocument.collection` and `DocumentLock.collection` (optional) name the collection; Oriel
  stamps what it has open with the collection the box shows (`siteState.codex.collection`, sent
  again when the box switches) and counts only that collection's entries and locks in badges,
  "*X* is editing" and lock banners; an open entry takes or keeps a lock only in its own
  collection. Entries without one (older peers) match any.

## Show box and Codex box

**Recommendation: one site-wide lock, held on the Codex** (stage 3, built), because every content
write lands there: a show box forwards it before any handler of its own runs. Without a Codex
link (stage 2, single boxes) each box keeps the presence and locks of its own clients, and its lock check runs
before write-through: a show box refuses its clients' writes to a document another of its
clients holds; the Codex box also checks the forwarded writes against its own clients' locks
(a forwarded write never takes one). Two clients on two show boxes fall back to the conflict
prompts, as today. Single-box sites (most churches) are complete at stage 2.

## API summary

| | |
|---|---|
| WS `presence` (client) | `{clientId?, computer?, open: OpenDocument[]}` |
| WS `presenceChanged` (server) | `{presence: PresenceList}` |
| `GET /api/v1/presence` | `PresenceList {clients: PresenceClient[], locks: DocumentLock[]}` |
| `POST /api/v1/presence/locks` | `TakeLock {kind, id, takeOver?}` + `X-Client-Id` → `DocumentLock`; `423 locked` |
| `DELETE /api/v1/presence/locks/{kind}/{id}` | `204` (the caller's lock only) |
| `423 locked` | on the content writes above, from a client that doesn't hold the lock |

Capability `presence`.

## Site-wide (stage 3, built in batch 9)

`codex::site_presence` (server), on the box-to-Codex link:

- **Reports.** A show box sends its own sessions to the Codex (`vesper.codex.presence {name,
  clients}`) on every change of its sessions or locks and every 20 s (a thread per server,
  woken by `Presence::wait_change`). The Codex keeps them as the box's *remote* sessions
  (dropped 60 s after its last report) and answers the site's merged list as that box sees it:
  clients of other boxes carry `server` (the box's name; the Codex box's own clients carry the
  Codex box's name), the box's own clients don't. The show box shows that list
  (`Presence::set_site`): `GET /presence` and `presenceChanged` are the site's.
- **Pushes.** Whenever the Codex's list changes (its own clients, a box's report, a lock), it
  sends each reporting box its view (`vesper.codex.presence_changed`, a thread each; a box
  that doesn't answer still reports every 20 s), so "*X* took over editing" reaches every box
  at once. Pushes, report answers and lock answers carry a `stamp` `{epoch, generation}` (the
  Codex's run and its presence generation; a report asks for it with `stamped: true`): they
  travel on separate threads and links, and a show box never replaces its list with an older
  one of the same run.
- **Mixed versions** (bug run 3, E05-1): a box or Codex from before `show`/`timers` refuses a
  whole message with an unknown place. Reports, lock requests and their answers carry
  `allPlaces: true`; a peer that didn't send it gets its lists cut down (`timers` → `library`,
  `show` and `other` left out). A show box assumes an older Codex until an answer says
  otherwise (then reports again at once), and again after a failed report.
- **Locks are the Codex's.** Remote sessions count like its own: a forwarded write from a
  show box's client that has the document open takes the lock for it; a write from anyone
  else, through any box, answers `423` naming the holder (and the box). A show box's `POST` /
  `DELETE /presence/locks` go to the Codex (`vesper.codex.lock {name, clients, op, kind, id,
  clientId, takeOver}`: the box's sessions are taken in first, so its caller is live there)
  and answer what the Codex answered. A show box's own lock check stands aside while it shows
  the site's list (all its content writes go to the Codex, which checks them).
- **Unreachable Codex.** A failed report puts the box back on its own list (its writes answer
  `503` meanwhile, so it needs no locks of its own); the next report that gets through brings
  the site's list back.
- Wire: `PresenceClient.server`, `DocumentLock.server` (optional, additive). Oriel names the
  box: "jo on Chapel laptop at Chapel Booth" (`personLabel`); shot
  `presence-library-locked-other-box`. Test: `tests/server/codex/site_presence.rs` (a lock
  taken on A refuses B's write, take-over through B and back through A, a closed socket on B
  leaves A's list).

## Browsing (batch 11, lane O)

Stage 0 plan, built in this batch (server, tests, Oriel, stub, shots):

- **Places.** `PresencePlace` gains `show` (the Show page: the playlist it lists, the library
  item picked, the presentation selected) and `timers` (the Timers page: the selected timer);
  an unknown place reads as `other` (`#[serde(other)]`, so the next new place doesn't break
  older boxes). Capability `presenceBrowsing`: an older server refuses a presence message that
  names a place it doesn't know, so Oriel leaves `show` / `timers` entries out without it.
- **Library open.** The Library page reports `{kind: "collection", id: <the box's collection>,
  place: "library"}`. The connection card says "*N* others have this collection open" (the
  board's "library" is a collection in Oriel, plan question 6); a click lists each person and
  what they are doing: "editing *song*" (the locks they hold), "looking at *X*" (what they
  have open in a place that edits), else "browsing".
- **Badges.** The Show page's header (the playlist, or the picked song), the Library item for
  props (and "*X* is editing" on the props' rows), the Timers page's settings. The Edit and
  Library badges now also show who has the song open on the Show page ("viewing on the Show
  page").
- **Locks only where things are edited.** A write takes a lock only for a client that has the
  document open in a place that edits it (`edit`, `reflow`, `library`, `timers`), and a lock
  lasts only while such a place lists it: browsing on the Show page never takes or keeps one.
  New locked writes: props (`PUT`, `DELETE /props/{id}`, `POST …/edits`: props are edited in
  Edit like the other one-slide documents) and a timer's settings (`PUT`, `DELETE
  /timers/{id}`; start, pause and adjust are commands, never locked). The Timers page turns
  the settings read-only with the lock banner and "Take over editing".
- **Owner question: lock playlists?** *Recommendation: no* (built so). Playlist edits are small
  operations (add, move, remove an item) the operator makes during a service, often while a
  planner adds songs; a hard lock would make the booth click "Take over" mid-service. They
  merge per item already, so playlists get presence ("2 others have this open" on the Show
  page) and no lock.
- Tests: `presence::tests::browsing_on_the_show_page_never_takes_a_lock` (unit),
  `tests/server/presence.rs` `browsing_is_listed_and_timers_and_props_lock`; vitest
  `state/presence.test.ts`; shots `presence-library-collection`, `presence-show`,
  `presence-timers-locked`.

## Deferred

1. Site-wide presence between two show boxes goes through the Codex box (no box-to-box
   link); a show box whose Codex can't be reached shows only its own clients.
