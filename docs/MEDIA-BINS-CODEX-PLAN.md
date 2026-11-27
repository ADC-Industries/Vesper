# Media bins, global groups and Codex box status (G17, G14, G15)

Batch 8, lane M (2026-11-24). Gaps from `docs/ORIEL-REDESIGN-PLAN.md` 4.11: **G17** media bins
and global groups editable over the API; **G14** which show boxes show which collection and how
far behind they are; **G15** uploading a show box's dormant collection to the site's Codex.

## G17: bins and global groups as documents

**Model (unchanged).** `MediaBin` and `GlobalGroup` are already collection documents
(`portable/mediaBin/`, `portable/globalGroup/`), loaded into `CollectionContent`, mirrored to
show boxes and imported by `pro-import` (ProPresenter's media and audio bins, their folders,
items and smart folders; its group library). A bin's folder is a path on the bin (folders are
not documents: a folder exists while a bin is in it). **Bin playlists** are `PlaylistNode`s
with `purpose: media | audio`: they already have the full playlist API (`/playlists`, item
edits, duplicate), so G17 adds nothing for them on the server; Oriel lists them with the bins.

**API** (all on the box's active collection, through the Codex writer like every content write:
planned in `content::plan`, stored atomically, `documentChanged {collection}`, the change log
with who did it, deletes into the item trash, restorable with their IDs):

| Route | Body | Does |
|---|---|---|
| `POST /media-bins` | `CreateMediaBin {name?, folder?, browser?, after?, document?}` | a new empty manual bin at the end of its folder (or after `after`); `document` recreates a deleted bin with its ID (undo) |
| `PUT /media-bins/{id}` | `UpdateMediaBin {name?, folder?, after?}` + `If-Match` | rename, move to another folder, reorder (`after: null` = first) |
| `DELETE /media-bins/{id}` | `If-Match` | deletes the bin (into the trash). Its media stays in the collection |
| `POST /media-bins/{id}/edits` | `MediaBinEdit {ops, clientId?}` | item ops `addItem {after?, item}`, `removeItem`, `moveItem {after?}`, `setItem {name?, enabled?}`; answers `{revision, applied, inverse}` like playlist edits. Watched bins refuse item ops (their items follow the folder) |
| `POST /groups` | `CreateGlobalGroup {name?, color?, hotkey?, after?, document?}` | a new global group |
| `PUT /groups/{id}` | `UpdateGlobalGroup {name?, color?, hotkey?, after?}` + `If-Match` | rename, recolour, hotkey, reorder |
| `DELETE /groups/{id}` | `If-Match` | deletes it (into the trash). Sections keep their snapshot; their link dangles as imported ones may |

`GET /media-bins` and `GET /media-bins/{id}` are unchanged (`MediaBinDetail` gains nothing).
Show boxes forward all of these (write-through table). Names follow the content rules (trimmed,
1–200 characters, no control characters); folder segments the same.

**Not done:** propagating a global group's rename or colour into the sections linked to it
(they keep their snapshot, as today); editing a watched bin's folder or import behaviour;
per-item trigger settings (layer, scaling, loop) in Oriel (the op takes a whole item, so they
can come later without a wire change).

**Oriel.** The Library's tree gets a **Bins** group under Media: every media bin (folders
nested), then the media playlists. Picking one shows its items as tiles. "+ New bin", rename,
delete (with a confirmation naming it), move up / down; dragging a media tile onto a bin adds
it, "Remove from bin" on a tile in a bin. A playlist in the list opens its items the same way
(add by drag, remove). Global groups are edited under Design ▸ Groups (name, colour, order).

## G14: which show boxes show which collection

The Codex box already learns each show box's collection from the box's link requests
(`hello`, `changes`, `index`, kept for push targeting). The `changes` request carries the box's
position (`since`), so the Codex knows how far each box has applied the log.

- `Link` keeps per box (by fingerprint): collection, the position it last asked from, when it
  last asked. The Site Manager's device list names it.
- `GET /codex` on the Codex box gains `boxes: [CodexBoxSync {fingerprint, name, collection,
  seq, behind, lastSeen, online}]` (online = asked in the last 10 minutes, the push window; a
  box polls every 30 s). `behind` = the collection's head minus the box's position.
- `CollectionSummary` gains `boxes {showing, synced}`: the Vespers showing the collection
  (show boxes online on it, plus the Codex box itself when it is its active one) and how many of
  them are at the head. Oriel: "Synced to 2 of 3 Vespers" in the Library's collection list and
  the Site ▸ Codex table; Site ▸ Codex lists the boxes (name, collection, "up to date" or
  "N changes behind", last seen).
- `codexChanged {status, collections}` when a box's state changes meaningfully (it starts or
  stops following, or catches up / falls behind), rate-limited to one per 2 s.

## G15: uploading a show box's dormant collection

A show box that joined a site with a Codex keeps its own Codex dormant
(`machine/codex.json ownCollection`, documents under `<data>/codex/collections/<id>/`, media in
its own library). Oriel offers to upload it:

- **Offer**: `GET /codex` on the show box already says `dormantCollection`; the Library's
  collection card shows "This box's own collection *Name* (N songs) isn't in the site's Codex.
  Upload it" to admins.
- **API (show box)**: `POST /codex/dormant/upload {name?}` starts a job, `GET
  /codex/dormant/upload` its progress (`documents`, `media` done / total, bytes, state
  `copying | done | failed`, the new collection's ID), `DELETE` cancels. One job at a time.
- **Transfer**: (1) the box asks the Codex over the link to create a collection
  (`vesper.codex.adopt {name, sourceCodex, sourceCollection}`); the Codex answers its ID, or the
  one it made before for the same source (resumable, never two). (2) Documents go in pages
  (`vesper.codex.adopt_docs`, same file bytes as `docs`, 8 MiB pages), written by the Codex's
  writer as creates with their IDs (logged, by the show box). (3) Media: every file the
  documents use goes through the Codex's existing resumable uploads (`codex::uploads`: an
  upload per file with its SHA-256, chunks with their offset, resumed from the Codex's
  `received` after a drop, taken into the library and deduplicated there), carried over the
  link (`vesper.codex.upload_begin` / `upload_chunk`, 4 MiB chunks) instead of REST, since the
  show box has no key on the Codex box; the asset documents are then rewritten to the Codex
  library's items (as `replaces` does for a relink).
- **Conflicts**: the upload is always a **new** collection (never merged into an existing one);
  a name already in use gets " (from *box*)". Document IDs are kept: an ID already in another
  collection is fine (collections are separate). A document of a newer schema than the Codex
  reads stops the upload with the reason. Media already in the Codex library by SHA-256 isn't
  sent again.
- **Afterwards**: the box's own Codex stays dormant and intact (nothing is deleted: the owner
  can still leave the site and wake it); the box is offered to switch to the uploaded
  collection (its assignment). Uploading again creates nothing new (the source is remembered).

## What's done in this lane

- **Stage 0**: this note.
- **Stage 1, G17** (done): the routes above with their tests (`tests/server/media_bins.rs`),
  write-through, the trash and restore for bins and groups, the protocol README ("Media bins and
  global groups"); Oriel's Library: bins and media playlists under Media (open, new, rename,
  move up / down, delete, items with Remove, drag from Images / Videos onto a bin or playlist,
  "Add" in those views), Design ▸ Groups (colour, rename, order, delete, "+ New group"); stub
  routes (`test/stub/routes/bins.mjs`), vitest (`bins.test.ts`), shots (`scripts/shots/bins.mjs`).
- **Stage 2, G14** (done): `CodexStatus.boxes` and `CollectionSummary.boxes` as above, from the
  boxes' `hello` / `changes` / `index` requests (`codex::link::MirrorBox`); its test
  (`tests/server/codex/box_status.rs`: up to date, one change behind with pushes off, caught up
  after the box's own write). Oriel: "Synced to N of M Vespers" in the Library's collection card
  and the all-collections list, and Site ▸ Codex's "Vespers showing its collections" table.
  Not rate-limited: each box's move in the change log sends one `codexChanged` (fine for a few
  boxes; coalesce if sites grow).

- **Stage 3, G15** (batch 9, lane C, done): `codex::adopt`. The link kinds `vesper.codex.adopt`
  (the collection for a source, staged in `codex/adopting/<id>/` until its documents are in;
  what was made for which source in `codex/adopted.json`), `adopt_docs` (pages of files, a
  newer schema refused with the reason), `adopt_finish` (moved in, every document logged as
  added by the box), `upload_begin` / `upload_chunk` / `upload_finish` (the Codex runs its own
  upload routes as the box's machine: `replaces` relinks the asset; bytes its library has are
  relinked at `begin` without sending; an open upload of the same bytes is answered again, so
  a drop resumes from its offset; a chunk is its JSON header, a NUL byte, then up to 4 MiB of
  bytes). M13's import wasn't reused: it takes a whole export at once, and the job needs pages
  and resumption. The show box: `GET/POST/DELETE /codex/dormant/upload` (admins), the job on a
  thread of its own, state in `machine/codex-adopt.json` (assets done), resumed by the site tick
  after a restart; the dormant library opened through IDN's `LibraryServer::open` for the job
  and closed after. Test `tests/server/codex/dormant_upload.rs` (two boxes: a newer-schema
  document stops it with the reason; again without it resumes into the same collection; B's own
  image sent and relinked, the shared one relinked without sending; the log names B; a third
  start makes nothing new). Oriel: `DormantOffer` on the Library's "where" column (offer,
  progress bar, Cancel, "Show it here"), stub `{"dormant": true}`, shots
  `page-library-dormant-{offer,copying,done}`. Not done: linked media (`Linked` under a media
  root) stay as they are (the Codex resolves them through its own roots); a test that drops the
  link mid-file (the resume path is the same `begin` that answers the open upload).

## What's left

- **Stage 3, G15**: done in batch 9 (above). The original notes: Found
  while sizing it: the dormant collection's files can be found through IDN's public API
  (`LibraryServer::open(<data>/codex/library, "Codex")` then `path_of(item)`, as
  `codex::library` does; the box's own library is closed while it is a client, so it can be
  opened for the upload and closed after), and `codex::uploads` already takes `replaces`
  (an upload relinks an existing asset), so the documents can go first with their old
  locations and each file relinks its asset when it lands. Worth checking first: whether M13's
  import (`codex::export`, which adds a collection from an export's documents with their IDs
  and logs them) can take the dormant collection's documents directly, instead of a new
  `adopt_docs`. In order:
  1. Link kinds `vesper.codex.adopt` (create or find the collection made from this source:
     `{name, sourceCodex, sourceCollection}` → `{collection}`; the source recorded in the
     collection's document as `adoptedFrom`), `adopt_docs` (pages of document files, written
     by the Codex writer as creates with their IDs, `by.machine` = the show box), and
     `upload_begin` / `upload_chunk` / `upload_finish` over `codex::uploads` (the same
     resumable state as REST uploads: `received` answered on `begin`, so a drop resumes).
  2. On the show box, `codex::adopt`: a job reading the dormant collection
     (`<data>/codex/collections/<own>/`), sending documents then each media file the assets
     use (library items resolved through the dormant library; `Managed` files from
     `codex/media/`), rewriting the uploaded collection's asset documents to the Codex's
     library items once each file is in (like `replaces` on a relink). State in
     `machine/codex-adopt.json` (resumed after a restart). `POST/GET/DELETE
     /codex/dormant/upload` (admin), `codexChanged {status}` with progress.
  3. Tests (two boxes, in `tests/server/codex/`): upload a dormant collection with one image,
     interrupt mid-file and resume, upload again (no second collection), a newer-schema
     document stops it with the reason.
  4. Oriel: the offer on the Library's collection card for admins on a show box with
     `dormantCollection`, progress (documents, files, bytes), then "Show it here" (the
     assignment). Stub routes and shots.
- G17 follow-ups: a global group's edits into linked sections (owner question: recommended
  "no": sections are snapshots, as imported); trigger settings per bin item in Oriel; folder
  rename for bins (today: move each bin).
