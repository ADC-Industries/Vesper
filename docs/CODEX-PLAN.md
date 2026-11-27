# Vesper Codex: implementation plan

Step 3 of the server-first build plan, after IDN media/audio (step 1) and the Site Manager (step 2).
The Oriel redesign follows, and its Library page builds on Codex. This document is the working
plan; PRODUCTS.md and ARCHITECTURE.md describe the product, and the protocol README's Codex
section describes the API as built.

**Status (2026-11-22):** M0–M15 are done (M13, the encrypted export and import, last). See
[What's left](#whats-left).

## Decisions (owner, 2026-11-19/20)

- **One Codex per site**, a module of the Vesper server on the **site's server** (the Site Manager
  box). Other buildings reach it over the church VPN. **No replicas** for now.
- **Naming.** A Codex unit is a **Collection** (per campus, ministry or room). The sidebar shelf of
  presentations stays a **Library**, as in ProPresenter (`model::Library`, `/api/v1/libraries`).
  Wire names: `Collection`, `CollectionId`, `/api/v1/codex/collections`.
- **What a collection holds:** presentations, libraries (shelves), playlists, themes, media assets,
  media bins, media roots, global groups, **plus the documents slide actions refer to**: timers,
  messages, props, macros, clear groups and stage layouts. **Screens, looks, show settings, masks,
  video and audio inputs stay per box**, as does everything in `machine/`.
- **Show boxes cache only what is scheduled:** media used by the box's playlists and bins is
  prefetched; anything else is fetched on demand; the cache has a size cap with LRU eviction of
  media nothing scheduled uses. Live output always plays from local disk, never across the network.
  Media not cached yet when triggered behaves like missing media, with availability `copying`, a
  "still copying" notice and a high-priority fetch.
- **Edits are write-through:** a show box forwards content writes to the Codex; if the Codex can't
  be reached, edits answer `503 unavailable` and the show keeps playing. No offline edit queue.
- **Oriel** sends show-time edits through its show box; Codex management and uploads go straight
  to the Codex box.
- **Storage stays JSON documents** (one atomic file per document, `store::apply`); search uses an
  in-memory index. ARCHITECTURE.md's `library.db` SQLite line is corrected.
- **Protection without replicas:** a scheduled, encrypted export of the Codex (collections and
  settings backups; **documents only, never media**: owner, 2026-11-22, see M13) to a folder the
  admin chooses (USB drive, NAS mount), with retention, plus a manual encrypted download. A recovery tool that re-seeds a Codex from a show box's copy
  comes later.
- **Settings:** backup and restore (with preview) only in the first release; templates and sync of
  chosen settings later.
- **Access:** Oriel's `app:vesper` site keys cover Codex (one pairing). Content edits: any paired
  client at every access level (as today). Collection and backup administration: admin (`open`:
  any paired client; `pin`: unlocked; `users`: signed-in admin). Backup lists are visible to any
  paired client; backup contents, preview, restore, export and import are admin only.
- **Replacement box:** a restore onto a new box may adopt the old box's server id (after the old box
  was removed from the site); the IDN device identity is always new and never backed up.
- **No internet dependency.** LAN-down resilience is low priority.

## Decisions (owner, 2026-11-23)

- **The conservative choices are confirmed**: those under M10, M11–M12 (and the show boxes') and
  M13, but the manual export's site part (below).
- **A manual export holds the site's identity only when asked** (`CreateExport.includeSite`, off
  by default; Oriel: a tick box "Include the site's identity"). Scheduled exports on the site's
  server always hold it. See "M13" below.
- **The Codex's own membership is part of Vesper**: the site's device list marks it `builtIn`,
  Oriel shows it as "Codex (built-in)" with no remove button, and the server refuses removing
  it (`409`).
- **Unused pairing keys** (Site Manager, not the Codex): only keys Vesper recorded as made for a
  pairing are revoked when still unused after 24 h; never a key picked by its name.

## Decisions (owner, 2026-11-22)

- **IDN hosts the media files** (owner, 2026-11-22). Codex refers to IDN media-library items
  (`AssetLocation::Library {id, sha256}`). M5–M7 are reworked onto IDN's library on the Codex
  box (IDN v0.3.1-Beta1/Beta3: lasting item IDs, changes, import, marks, trash, space); see "M5–M7
  on IDN's library" below. The show-box side (M8–M9) finds the Codex's library by its name
  (`Library::find_named`, Beta2); see "M8–M9: show boxes" below.
- **Services on the same box just join.** The Codex joins its own site through
  `host::Handle::join_own_site` (IDN Alpha97), approved at once; see below.

## Decisions (owner, 2026-11-21)

- **Switching a box's collection while something is live needs a confirmation.**
  `PUT /codex/assignment` answers `409 liveShow` (naming what is live: a slide, media, a prop)
  unless the request carries `confirm: true`; confirmed, it switches as before (the live slide
  is cleared).
- **Deleting a collection moves it to a collection trash** (`codex/collection-trash/<id>/`, the
  folder moved intact with a `deleted.json`). It disappears from the collection list and every
  normal view (its item trash entries too, which still expire after 30 days), and is listed by
  `GET /codex/collection-trash`, from where an admin restores it or deletes it for good (a
  second, audited delete: documents, change log and its item trash entries). Deleted
  collections never expire; the 30-day expiry stays for items. The collection a box shows still
  can't be deleted. The media of deleted collections count as used (marked in use in the
  media library).

## M5–M7 on IDN's library (2026-11-22)

- **The library** (`codex::library`): `LibraryServer::open(<data>/codex/library, "Codex")` on the
  box whose Codex role is `host`, at every load until it is open. IDN refuses a library whose
  index can't be read: the Codex reports it (`CodexStatus.mediaLibrary {available: false,
  problem}`), its library media are missing, uploads can't finish (`503`), nothing is wiped or
  recreated, and the next load tries again. Served to the site with the Codex's own membership
  (`serve(member)`, followed every site tick; the channel kept while the membership is). One
  folder per collection, by its name (made a plain folder name).
- **Migration** (`codex::media_migration`, resumable, `codex/library-migration.json`): every
  `codex/media/<sha256>.<ext>` is staged as `codex/library-import/<blob>/<original name>` and
  imported (moved) into the folder of the first collection that uses it; documents' `managed`
  locations (collections, collection trash, item trash) become `library {id, sha256}`. Files
  whose ending IDN doesn't take as media stay managed in `codex/media/` and move in at a
  later start once it takes them (since Beta3 the library takes every ending Vesper plays:
  `library::take_endings`). Documents holding a library location are written as schema 3
  (older servers refuse them rather than lose the location); others stay 2. Same bytes already in
  the library: that item, and the staged copy goes. `media-gc.json` goes.
- **Uploads**: Oriel's resumable REST upload API is unchanged (Oriel isn't an IDN member). On
  finish the checked file is imported under its uploaded name into the collection's folder
  (dedup by IDN), the asset saved with its library location and the item marked in use.
- **Probe and thumbnails**: IDN probes every item in the background with the Codex's prober
  (the media crate's probe); the media worker's probe of an asset shares it (one probe per
  file). IDN gives no thumbnails on the host side, so the Codex keeps `codex/thumbs/<sha>-320.jpg`.
- **Closing**: the server's stop closes the library with IDN's `close()` (index saved, the
  folder free for the next open; a second open in one program waits up to 10 s, then is
  refused: reported, nothing wiped).
- **Delete protection**: no more blob GC. The Codex `set`s its marks (holder `vesper-codex`) to
  every item a document uses, at start and after writes that change media references
  (debounced); unused media sit in the library unmarked, and the Codex never deletes library
  items (the owner does, through IDN's delete and trash).
- **Storage** from IDN's `space()` plus per-collection usage from the catalog.
- **Missing media / relink**: a library asset is available while its item is in the library;
  relink by content hash looks the SHA-256 up in the library first; the Codex follows the
  library's `changes()` and announces `codexChanged {media}` for the collections whose items
  changed or vanished. Linked media (roots) are unchanged.

## M8–M9: show boxes (2026-11-22)

- **The link** (`codex::link`): the Codex serves its site on **one channel** with its own
  membership; its handler answers `vesper.codex.hello`, `changes`, `index` and `docs` (JSON,
  `v: 1`) and hands every other request, file and upload to IDN's `LibraryServer` (its public
  `request`, `file`, `on_push`, `on_received`): one fingerprint for documents and media.
  Vesper's kinds are answered only to devices the Site Manager lists (not revoked). The bulk
  snapshot is `index` (the change-log position read first, then every document's kind, ID and
  SHA-256) plus `docs` in pages (answers stop at 8 MiB); documents travel as their files'
  bytes, so a show box keeps them exactly. After each change-log append the Codex sends
  `vesper.codex.changed {collection, seq}` to the boxes that asked about that collection in the
  last 10 minutes (a thread per push, short timeouts).
- **The show box** (`codex::mirror`): a thread follows the site. Enrolled in another site, the
  box starts its own channel with that membership (`CodexOptions.channel_port`, CLI
  `--codex-channel-port`: a fixed UDP port for firewalls), finds the Codex over that channel
  (IDN Beta4: `find_on_via` the fingerprint saved in `machine/codex.json`, else
  `find_named_via(channel, "Codex")`; one endpoint toward the Codex box, so the Codex's
  requests reach Vesper's handler), opens IDN's cache on it (`codex-cache/media`, cap `CodexOptions.cache_bytes`, 32 GiB, CLI
  `--codex-cache-gib`), and asks `hello`. The site hosts a Codex: the box's own Codex goes
  `dormant` (kept intact, read only, still shown) while the first copy arrives, then the box is
  a `client` (`machine/codex.json {role: client, collection, codex {serverId, fingerprint},
  ownCollection}`) and `reloadData` shows the copy. Leaving the site (or being removed) wakes
  the own Codex: `host`, `ownCollection` shown again (through the same reload).
- **The mirror**: `codex-cache/collections/<id>/{collection.json, portable/…, state.json
  {codexId, seq}}`. A first copy, a `resync`, a gap or another Codex compares the index with
  the local files (SHA-256) and fetches or removes only what differs; otherwise the change log
  is followed from the saved position (the last word per document, in seq order), the position
  saved after each batch (a restart resumes there). The Codex's pushes wake the box; it also
  polls every `CodexOptions.mirror_poll` (30 s). Changes of the shown collection are rebuilt
  from disk and swapped in under the Codex writer (`CollectionUpdate::Mirror`: the show follows
  like an edit), with `documentChanged {collection}` for what differs. The Codex unreachable:
  the box keeps what it has, `sync.reachable: false`.
- **Media**: the library items the shown collection's playlists and media bins use (their
  media triggers, and the media of the presentations they hold) are pinned (`Cache::pin`, after
  each change); a cue triggered (media, or a presentation becoming the current one) `want`s its
  items. `MediaLibrary` on a show box resolves library items to `cache.path(id)` only; not here
  yet is availability `copying` (the cue behaves like missing media, a `still copying` notice).
- **Status**: `CodexStatus.sync` (`CodexSync`: reachable, the Codex's server ID and fingerprint,
  last sync, seq, the Codex's seq, behind, problem, the cache's pinned / cached / copying /
  missing / failed / bytes / cap) and `dormantCollection`; `codexChanged {status}` when it
  changes meaningfully.
- **Writes** on a show box: write-through (M10, below). `PUT /codex/assignment` on a show box
  copies the collection first (`503` while the Codex can't be reached), then switches as on the
  Codex box.

## M10: write-through edits (done, 2026-11-22)

- **Where** (`codex::write_through`, a middleware of the REST routes on a show box): a fixed
  table of routes (`FORWARDED`) is forwarded; another (`CODEX_BOX`) answers `409
  codexElsewhere` with `ErrorResponse.codex` (`CodexBox`: server ID, name, fingerprint, REST
  base URLs); everything else is the box's own. Forwarded: the content writes (libraries,
  presentations, playlists, timers, messages, props, stage layouts), the item trash (its list
  and restore), relinks to a target, client folder match and re-root. Codex box: uploads,
  collection CRUD, the collection trash, emptying the trash, the relink search under roots
  (it reads the Codex box's disks for minutes, past a link request's timeout).
- **How**: `vesper.codex.write {collection, machine, client {access, id, name}, method, path,
  query, headers (If-Match, Content-Type, X-Client-Id), body}`. The Codex checks the route is
  in `FORWARDED` and runs the request through its own router (`tower::oneshot`) with an
  in-process `api::Forwarded` extension: `authenticate` takes the client from it, the change
  log's `by.machine` is the show box, and the content writer acts on the box's collection
  (`api::forwarded_collection`) instead of the Codex box's active one. The answer (status,
  `ETag`, body) goes back unchanged, plus the change-log position; the show box then runs a
  poll at once (`Mirror::refresh`; polls are serialized with their swap) before answering, so
  the client reads its own edit from the box.
- **The Codex box's addresses**: `hello` carries its REST port and its listening address
  (when not a wildcard); the show box adds the site server's host (its membership's
  discovery address) with that port.
- **Search**: the box's own collection locally; `all` or another collection by
  `vesper.codex.search` (`503` while unreachable).
- **Unreachable**: `503 unavailable` ("edits need the site's server …; the show keeps
  playing"). No offline queue.

Owner confirmed 2026-11-23: collection administration from a show box is not
forwarded (the plan has Codex management go straight to the Codex box, and admin checks
depend on the box's own PIN unlock / sign-in state) but answered with `codexElsewhere`; the
Codex trusts the show box for who the client is (the box is a listed machine of the site
and checked the client's site key itself) and for its server ID in `by.machine`; a forwarded
request from the show box's own trusted machine acts as the Codex box's `server machine`
client (only the forwarded routes); bodies up to 8 MiB are forwarded (the REST routes take
2 MiB); the box's `documentChanged` after a forwarded write carries no `clientId` echo or
revision (it comes from the mirror's swap).

## M14: search (done, 2026-11-22)

- **The index** (`codex::search`): in memory, one per collection, built over
  `CollectionContent` (not the files), with its own dictionary of folded words (lower case,
  accents of Latin letters removed, Vietnamese included; apostrophes inside words dropped) and
  postings per field. It follows each collection by comparing the content's document
  fingerprints (`CollectionContent::hashes`) with the ones it indexed: only documents added,
  changed or removed are re-indexed; collections no longer in the Codex (collection trash,
  deleted for good) are dropped, a restored one is indexed again.
- **When:** a thread at the lowest CPU priority builds it at start and syncs after every change
  of the documents (`Shared::set_data`, `set_codex`, `set_both` wake it: every Codex writer
  commit, a trash restore, collection create/delete/restore/purge, a switch, a reload). A search
  syncs first too, so it sees its own edits at once.
- **What:** presentations (name; details: song title, artist, authors, publisher, CCLI number,
  category, scripture reference; group names; slide labels; slide text; slide and presentation
  notes), libraries, playlists and folders, media assets (name, original file reference), themes
  (name, template slide names), props (name, slide text), messages (name, text), timers, macros,
  stage layouts. Not indexed: global groups, media bins, clear groups.
- **Queries:** every word must match the start of a word (anywhere in the document); `"…"` is a
  phrase of whole words in order within one field (the last word a prefix while the quote is
  open). Ranking: per word the best field it matches in (name 100, details/group/label/template
  25, origin 15, text 10, notes 5; a whole word × 5, a prefix × 4), +2000 when the name is the
  query, +1000 when it starts with it; then by name.
- **API:** `GET /api/v1/codex/search?q=&collection=&kinds=&limit=` → `SearchResults {hits,
  total}`; a hit has kind, id, collection and its name, title, the field, a snippet with the match
  marked, and the slide index (with slide and section IDs) for presentations. Default scope: the
  box's active collection; `collection=all`: every collection (not the trash).
- **Speed** (unit test, synthetic, 3000 presentations of 12 slides, 885,000 words; debug build
  on the dev box): built in ~0.9 s; queries 2–17 ms (a two-word phrase over common words 45 ms).
  An optimised build is several times faster.
- **Oriel:** the toolbar's presentation search uses it (names, slide text, groups, labels, notes,
  with the snippet), falling back to the name filter against servers without the `codex`
  capability (M15).
- **Show boxes:** the index code takes any `Source {id, name, content}`; a show box indexes its
  mirrored collection (the mirror's swap wakes it through `Shared::set_data`) and forwards
  `collection=all` or another collection to the Codex with `vesper.codex.search` (M10: `503`
  while it can't be reached).

## M11–M12: settings backups and restore (2026-11-22)

Built on the Codex box (`codex::backups`; wire types in `model::backup`, `protocol::backup`;
REST in the protocol README's Codex section):

- **Sets** (`backups/sets/<set>.json`): one per product and machine. Vesper: one per box, by its
  server ID, with its name and (for a machine of the site) its site fingerprint. Oriel: one per
  paired client, by its site key's id (`local` for the server machine's trusted clients).
  `product` is a string, so products to come need no Codex code.
- **Snapshots** (`backups/snapshots/<set>/<snapshot>.json`, `seq` per set) list items
  (`key`, `kind`, `id`, `name`, own `schemaVersion`, `sha256`, `size`); the bytes are blobs
  (`backups/blobs/<sha256>`, written before the snapshot). Same items as the newest: no new
  snapshot. Retention after every new snapshot (newest 10, plus the newest of the 14 most recent
  days and 8 most recent weeks *that have snapshots*), then unreferenced blobs go.
- **A Vesper box's items**: `portable/{screen,look,showSettings,mask,videoInput,audioInput}/`,
  `machine/{outputBinding,inputBinding,audioInputBinding,mediaRootBinding,audioDeviceConfig,
  fontSubstitutionTable}/`, `machine/server.json` (name and ID). An allowlist, checked again
  when a restore is applied.
- **When**: the box's settings are read from disk every minute (`BackupOptions::poll`); a
  snapshot daily (`interval`, `--backup-interval-hours`) and once they stayed changed for 10
  minutes (`change_delay`); `POST /codex/backups/snapshots` on demand. Reading the files
  instead of hooking every writer catches every path (API, `reloadData`, edits on disk).
- **Restore**: preview (per item, summary, what needs a restart); restore all or chosen keys;
  a `beforeRestore` snapshot first (the undo; the newest is reused when identical; the snapshot
  being restored is never pruned by it); files written atomically, never over a newer schema;
  then `reloadData` (screens, looks, outputs, show settings, audio device live; server name and
  font substitutions after a restart). The server ID is kept; device identities are never in a
  snapshot.
- **Replacement box**: `adoptServerId` only when the set's recorded site fingerprint is listed
  as `removed` by the site's devices (`409` otherwise, and for a set without a fingerprint).

**Show boxes over the link (done, 2026-11-22; `codex::backups::remote`).** The show box always
asks (designed while the Codex's requests didn't reach show boxes, IDN Beta3); since Beta4
the Codex also tells the box at once when it queues one (`vesper.codex.restore_queued`), and
the poll stays the fallback:

1. **Backup offer** (`vesper.codex.backup_offer`, box → Codex): a show box runs the same
   schedule as the Codex box (`Backups::tick`: daily, ~10 minutes after a change, every
   `poll`), and `POST /codex/backups/snapshots` on the show box offers at once. The box knows
   its set's newest snapshot from the last answer (in memory; after a restart the first look
   offers, and the Codex answers the newest when nothing changed). The Codex
   (`Backups::accept_from_box`) checks the offer against the device it came from: Vesper only;
   never the Codex box's own server ID; the offer's fingerprint must be the sender's; a server
   ID whose set another device offered is refused unless the site lists that device as
   `removed` (a box that joined again). The set records the sender's fingerprint.
2. **Restores of show boxes** (`codex::backups::restores`, `codex/backups/restores/<id>.json`):
   `POST …/restore` for another box (the set's own box, or `RestoreRequest.target`) checks the
   target has a set with a site fingerprint (it offered over the link), checks a replacement's
   takeover early, and queues it: `202 QueuedRestore` (`queued`). At each poll the box asks
   `vesper.codex.pending_restore` (matched by its device fingerprint); if one waits it offers
   its `beforeRestore` snapshot with `vesper.codex.restore_begin` (the Codex takes it, never
   pruning the snapshot being restored, calls `prepare_restore` against it and answers the
   package; `applying`), runs `apply_here` (files, `reloadData`) and reports with
   `vesper.codex.restore_result` (`applied` with the `RestoreResult`, `serverIdAdopted` for a
   replacement; or `failed` with why). A refusal at `restore_begin` marks it `failed`. Not
   taken within 24 h (`BackupOptions::restore_expiry`): `expired`; `applying` for an hour
   without a report: `failed`. `GET /codex/backups/restores[/{id}]`, `DELETE …/{id}` (cancel
   while queued), admins; finished ones are listed for 30 days.
3. **Preview for another box**: against its newest snapshot on the Codex (`currentAsOf`); a
   fresher one would need a Codex → box request (see [What's left](#whats-left)).

Owner confirmed 2026-11-23 (show boxes): one restore waits per box at a time; a box that never offered its settings over the link can't be a restore target; queued
restores expire after 24 h; restore states are admin-only to read (as restores are); a
fingerprint may offer under a new server ID without a check (adoption needs that); the
Codex box restoring another box's set onto itself needs an explicit `target` (the replacement
case).

Owner confirmed 2026-11-23: `machine/codex.json` (role, assignment), `site.json`
(PIN hash, sign-ins), fonts and the Site Manager's folder stay out of snapshots; only this box's
own set is restored over REST; adopting a server ID needs the old box *listed* as removed (a
box the site never listed can't hand its ID over); Oriel's set of the server machine's trusted
local clients is shared (`local`); retention counts periods that have snapshots, so a box that
was off keeps its older ones.

## M13: encrypted export and import (done, 2026-11-22)

Owner decisions (2026-11-22): scheduled exports use a passphrase an admin stores (REST, admins);
every manual export asks its own; the stored one lives in `machine/` (0600), is never answered
back, logged, snapshotted or exported, and without it scheduled exports don't run (the status
says why). **Documents only, never media**: the media folder needs its own backup (a NAS copy of
`codex/library/`), and every answer says so. Retention per site, 7 nightly + 4 weekly by default,
into a folder the admin sets (default `<data>/codex/exports/`), nightly at a set time. On the
site's server the export holds the Site Manager's folder: a scheduled export always, a manual one
only when the admin ticks it (`includeSite`, off by default; owner, 2026-11-23); its import is
the admin's choice and needs a restart.

Built (`codex::export`; wire types in `protocol::export`; REST in the protocol README's Codex
section, capability `codexExport`):

- **Format: an IDN backup** (`idn_open_protocol::backup::make`/`open`, kind `vesper-codex`; no
  crypto of Vesper's own). IDN's `make` backs up one folder as it is, so the export is assembled
  in a staging folder (`<data>/codex-export-staging/`, removed after) while the Codex writer and
  the settings backups are held (one consistent copy): `export.json` (`ExportManifest`),
  `codex/codex.json`, `codex/collections/<id>/` (documents, `collection.json`, `changes.jsonl`),
  `codex/backups/` (sets, snapshots, blobs). Over 512 MB (IDN's `MAX_SIZE`): `409`.
- **The site part** is IDN's own site backup, nested: `site/site-manager.idnbackup` is
  `backup::make(<data>/machine/site-manager, "site-manager", idn_site_manager::BACKUP_SKIP,
  passphrase)`, so exactly what IDN's Site Manager backs up (its skip list: reports, logs,
  alerts, the audit, the MQTT store, `restore-pending`, its own backups folder), usable with
  IDN's own restore tools too. Only on the box that runs the site's Site Manager: in every
  scheduled export there, in a manual one only with `includeSite: true` (`409` when asked on a
  box that doesn't run it).
- **Not in it:** media, the item trash and the collection trash, uploads, thumbnails, the media
  library's index, anything of `machine/` (identities, the stored passphrase, settings).
- **Scheduled** (`export::schedule`): looked at every minute (`ExportOptions::tick`; tests use
  `every`); due when the newest file in the folder is older than the last slot passed, so a box
  that was off exports when it is back. Files `vesper-codex-YYYY-MM-DD-HHMMSS.vcodex`, written
  atomically; retention keeps the newest of each of `keepNightly` days and `keepWeekly` weeks
  that have exports (local time); nothing else in the folder is touched.
- **Import** (`export::import`): upload (`<data>/codex-import/`, an hour), preview with the
  passphrase (each collection `new`, `same`, `newer`/`older` by its newest change, `inTrash`;
  backup sets `present` by ID or by product and machine; the site part; the manifest), apply
  with explicit choices through the writer. Defaults: new collections added; existing ones kept
  unless in `replace`; backup sets this Codex lacks added (with snapshots and blobs), its own
  kept; the site never touched unless `site: true`.
- **Replace**: refused for the collection the box shows (as a delete is). The current copy goes
  to the collection trash first **as a collection of its own** (a new ID, "… (before import)"):
  with the same ID it could never be restored while the imported one exists, nor the imported
  one deleted while the trash holds the ID. The collection keeps its ID and its change log, which
  logs every document the import added, changed or removed (`by` the admin), so show boxes
  following it catch up through the log; `documentChanged`, `codexChanged`.
- **New collections** keep their exported change log; the Codex keeps its own ID (`codex.json`
  is not imported), so show boxes that followed the exported Codex resync by index once.
- **Site restore**: `backup::stage` into the Site Manager's folder; IDN's `host::start` already
  runs `backup::apply_staged` before it opens the folder (no Vesper code needed), refusing a
  restore staged a week ago or when machines joined since. `restartNeeded: true`. At that
  start the Codex's `join_own_site` finds its membership is of the site before, sets it aside
  (`machine/codex-member/old-site-<ID>/`) and joins the restored site anew with the same
  identity (IDN Beta5). The old Codex box's member stays in the restored site's device list,
  offline, until an admin removes it.
- **Media after an import onto a fresh box**: missing until the files are back; the relink
  search finds them by SHA-256 (in the library or under a media root; tested with synthetic
  media).
- **Small items in this run**: on a show box the settings backup routes (but offering its own
  snapshot) and the export/import routes answer `409 codexElsewhere` with `CodexBox`; the route
  table in `codex/api.rs` lists the backup and export routes; `mdns` is announced only while the
  server is announced over mDNS.
- **Oriel**: no Codex or backup settings view exists yet, so the manual download and the export
  settings are left to the Oriel redesign (which must say media isn't included and ask the
  passphrase twice, at least 8 characters). On the site's server the manual export offers a
  tick box **"Include the site's identity"**, off by default (`includeSite`).

Owner confirmed 2026-11-23: the trashes aren't exported; the importing Codex keeps
its own ID; collections in the collection trash aren't imported (restore or purge them first);
backup sets present here (same product and machine) are kept, the file's not merged; scheduled
exports catch up at once when the last slot was missed (and at once after the passphrase is first
stored); the export settings and the passphrase are the Codex box's (`machine/`), not part of the
export; the scheduled list and the settings are admin-only to read; an import keeps the uploaded
file at most an hour (4 at a time). Changed by the owner (2026-11-23): a manual export includes
the site part **only when asked** (`CreateExport.includeSite`, off by default; `409` on a box that
doesn't run the site's Site Manager); scheduled exports on the site's server always include it.

## IDN (from the IDN maintainers, 2026-11-20)

- **File channel (`channel/1`)**: peer-to-peer between enrolled machines; resumable; SHA-256
  checked; up to 64 GB per file; rate cap `channel.max_rate_mbps`; files only (a folder is many
  files). Library API: `idn_open_protocol::channel` (push/fetch, `PathSink`).
- **No backup API for programs**: Codex keeps its own settings backups (moved over the file
  channel).
- **No change notification or content-addressed store** for programs' documents: Codex keeps its
  own manifest and change log. (Media changes come from IDN's library, `changes()`, since the
  library took over the media files; see the 2026-11-22 decisions.)
- **Overlap**: IDN's site media library (§N.26) and IDN Data downloads overlap Codex. The clean
  room suggests Codex is the library (store and catalog) and IDN keeps the transport and media
  decoding (`idn_ffmpeg::media` probe/thumbnail). The owner decided otherwise (2026-11-22): IDN
  hosts the media files (see the decisions above).
- **The Codex box's own membership:** the Codex box (the site's server, which runs the Site
  Manager through `host::Handle`) needs its own site membership for `channel/1`. Done (IDN
  Alpha97): at the Site Manager's start the Codex calls `handle.join_own_site(dir, "Codex",
  DeviceRole::App)` (`codex::member`), idempotently (an existing membership is returned as it
  is), with `dir = <data>/machine/codex-member/`, and keeps the `Enrollment` for M8. Vesper wires
  no self-approval of its own. In the site's device list it is marked `builtIn` (owner,
  2026-11-23): Oriel shows it as "Codex (built-in)", part of Vesper, with no remove button, and
  the server refuses removing it (`409`: it would only join again at the next start).
- **Show boxes** use their own `Enrollment` and `Channel` (no IDN Node). Discovery runs through
  the site directory on the trust connection, with no multicast. Peers are addressed by
  fingerprint (`channel.devices()`).
- **Across buildings** the VPN must be **routed without NAT**. Every box must reach the Site
  Manager's discovery TCP port, and boxes must reach each other's channel UDP port, so Vesper sets
  a fixed `ChannelConfig.port` that the firewalls open. Throttle with `channel.max_rate_mbps`.

Never work around an IDN gap here: report it to the IDN maintainers.

## Layout

```
<data>/codex/                          only on the Codex box
  codex.json                           {schemaVersion, id, createdAt}
  collections/<colId>/collection.json  Collection
  collections/<colId>/portable/<kind>/<id>.json   same files and rules as today
  collections/<colId>/changes.jsonl    change log: seq, at, kind, id, change, revision, by
  library/                             the Codex's media library (IDN's site media library):
                                       a folder per collection, items with lasting IDs and
                                       SHA-256s; IDN's index, facts and trash in library/.library/
  library-migration.json               the old store moved into the library (resumable)
  library-import/<blob>/<name>         its staging
  media/<sha256>.<ext>                 the old managed store (what the library doesn't take)
  thumbs/<sha>-<w>.jpg                 media thumbnails by SHA-256 (cache, 320 px)
  uploads/<uploadId>.part|.json        resumable uploads (uploads/<uploadId>/<name> while imported)
  trash/<entry>.json  trash.jsonl      deleted documents and assets (30 days), the trash's index
  collection-trash/<colId>/            deleted collections, intact (no expiry), with deleted.json
  backups/sets/ backups/snapshots/ backups/blobs/<sha256>
  exports/                             scheduled encrypted exports (default folder; M13)
<data>/codex-cache/                    on a show box that isn't the Codex
  collections/<colId>/portable/...     mirrored documents, state.json {codexId, seq}
  media/                               IDN's library cache (`library::cache::Cache`, size-capped)
<data>/machine/codex.json              {role, collection (active), codex {serverId, fingerprint},
                                       ownCollection (the dormant own Codex's, on a show box)}
<data>/machine/codex-member/           the Codex's own identity in its site (never backed up)
<data>/machine/codex-export.json       scheduled exports' settings; codex-export-passphrase (0600)
<data>/codex-import/                   uploaded exports waiting for their import (an hour)
```

Roles: **host** (this box holds the site's Codex), **client** (mirrors one collection), **dormant**
(this box's own Codex, kept while it is a member of another site; leaving wakes it).

**Migration of a standalone install** (it becomes its own Codex): on first start without `codex/`,
move the collection kinds from `portable/<kind>` into `codex/collections/<new>/portable/<kind>` and
`media/` into `codex/media/` (same-filesystem renames), resumably (`codex/migration.json`). One
collection named after the server is created and made active. `pro-import` writes only into a
fresh folder (owner, interim): it refuses a data folder holding `codex/`, since it recreates
`portable/` and `machine/` and would destroy the box's identity, site membership, screens and
looks. A fresh import folder migrates the same way on first start.

**Joining a site that has a Codex:** the box's own Codex goes dormant (done, M8–M9); Oriel is to
offer uploading its collection to the site's Codex as a new collection (not built yet).

## Model (crates/model/src/codex.rs)

- `Collection {id, name, description?, color?, sortKey, purpose: content|signage|unknown, createdAt}`.
- Change log entry `{seq, at, kind, id, change: added|changed|removed, revision?, by}`; `seq` is
  monotonic per collection; compaction after 90 days or 10k entries (`resync: true` below the floor).
- Trash entry `{id, collection, kind, docId, name, deletedAt, expiresAt, by, document, subtree}`
  (a folder's sub-folders and playlists are deleted and restored with it).
- `BackupSet` (one per product and machine), `SettingsSnapshot`, `SnapshotItem` (content-addressed
  blobs), `RestorePreview`, `RestoreRequest` (with a `beforeRestore` snapshot as the undo). Snapshot
  items carry their own `schemaVersion`, so products that don't exist yet need no Codex code.
- Never in a snapshot: `machine/site-member/` and `machine/codex-member/` (device identities),
  `cache/`, `clients.json`, tokens.
- Retention defaults: last 10 snapshots plus 14 daily and 8 weekly; trash 30 days; change log 90 days.

## API outline

REST under `/api/v1/codex` with the protocol README's conventions (`If-Match`, `409` with
`current`, `400` with `field`), WebSocket `codexChanged {parts, collection?}` ("refetch these"),
`documentChanged` gains optional `collection`. Main routes: status; collections CRUD and the box's
assignment; `changes?since=`; search; resumable uploads (`Content-Range`, SHA check, dedup);
media list, relink (manual, upload, root search, client folder match, re-root); blobs and
thumbnails with `Range`; storage usage; trash; backup sets, snapshots, preview, restore; encrypted
export and import (M13); Oriel preferences backup. Existing content routes act on the box's active collection.
Servers with Codex announce the `codex` capability (M15).

Box to Codex over `channel/1` (JSON requests, `v: 1`): `hello`, `changes`, `index`, `docs` (the
bulk snapshot in pages of at most 8 MiB), `write` (carrying the original client for the audit),
`search`, `backup_offer`, and the box asking for queued restores (`pending_restore`,
`restore_begin`, `restore_result`); `changed` and `restore_queued` pushed to the boxes, with a
30 s poll as fallback; media through IDN's library (`library::cache::Cache`).

## Milestones

Each is a few hours. Integration tests go in `crates/vesper-server/tests/server/codex/` (one module
per area) inside the single server test binary; unit tests in `crates/vesper-codex/src/`.
Synthetic data only.
Gates every milestone: clippy `-D warnings`, the full workspace suite, bindings test after wire
changes, `cargo fmt -p <crate>` only.

| # | Milestone |
|---|---|
| M0 | ✅ Docs: PRODUCTS.md, ARCHITECTURE.md (one Codex per site, no replicas, Collections, JSON storage), protocol README Codex skeleton |
| M1 | ✅ Refactor `data::ShowData` into `CollectionContent` (the collection's documents, media dir, indexes) plus box config; `content::plan`, `showdocs`, `asset_path`, `with_content` on it. No behaviour change. |
| M2 | ✅ Codex store and layout, `Collection` type, `machine/codex.json`, resumable migration, load all collections, the active collection |
| M3 | ✅ Codex writer (serialized), collection CRUD and assignment, `GET /codex`, `codexChanged`, `documentChanged.collection`, access checks |
| M4 | ✅ Change log (seq, compaction, resync), trash and restore |
| M5 | ✅ Resumable uploads, SHA check, dedup, probe, media thumbnails (reworked 2026-11-22: imported into IDN's media library, IDN's prober) |
| M6 | ✅ Storage usage, ~~blob GC~~ (reworked: IDN's `space()` and catalog; "in use" marks instead of GC) |
| M7 | ✅ Missing media: availability, relink paths (reworked: library items, relink by SHA-256 in the catalog, following the library's changes) |
| M8 | ✅ Box-to-Codex link over `channel/1` for **documents** (one channel on the Codex for documents and its media library; `hello`, `changes`, `index`, `docs`, pushed `changed`; roles `dormant` / `client`, waking on leave) |
| M9 | ✅ Mirror of the documents: initial sync, following changes (push and poll, resync), sync status; media through IDN's `library::cache::Cache` (pins for scheduled media, on-demand fetch, its size cap, `copying` availability); never blocks render. The Codex's pushes reach show boxes since IDN Beta4 (one endpoint per box: the mirror finds the Codex over its own channel) |
| M10 | ✅ Write-through edits from a show box (`write` over the link, carrying the original client for the audit), conflicts across boxes (`If-Match` against the Codex's revision), 503 when the Codex is down; the mirror applies the box's own edits from the change log at once; uploads and Codex management answered `409 codexElsewhere` naming the Codex box; search beyond the box's collection forwarded (see "M10" above) |
| M11 | ✅ Backup sets and snapshots for Vesper and Oriel (schedule, on-change, retention); show boxes offer theirs over the link (see below) |
| M12 | ✅ Restore preview and restore (same box; show boxes and replacement boxes through restores queued on the Codex, taken at the box's poll) |
| M13 | ✅ Encrypted export (an IDN backup, documents only, never media) and import (preview, then apply); scheduled exports with a stored passphrase and retention; the site's Site Manager folder on the site's server (a manual export: only when ticked), restored at the next start (see "M13" above) |
| M14 | ✅ Search index (see "M14: search" above) |
| M15 | ✅ Protocol README's Codex section as one description, bindings checked (every Codex wire type exported), `ws_probe --codex` (status, collections, `--codex-search`, live `codexChanged` / `documentChanged`), the `codex` capability in `GET /api/v1/server` (Oriel's search fallback uses it) |

## What's left

- **Oriel's export and import UI**: the manual encrypted download (passphrase twice, at least 8
  characters; on the site's server a tick box "Include the site's identity", off by default),
  the scheduled exports' settings and the import preview, saying that media isn't
  included; left to the Oriel redesign (no Codex settings view exists yet).
- **Uploading a dormant collection**: a show box's own collection (dormant while it is in the
  site) uploaded to the site's Codex as a new collection, offered by Oriel.
- **Incremental mirror updates**: a show box rebuilds the shown collection's whole content from
  `codex-cache/` after each sync that changed it (`Mirror::swap`, `CollectionContent::load`); applying
  only the changed documents (as the Codex's writer does with `with_change`) would be cheaper on
  big collections, and could carry the forwarded edit's `clientId` and revision in its
  `documentChanged` (today they come without).
- **Oriel's redirect UI**: Oriel doesn't use the Codex management routes yet; on a show box they
  answer `409 codexElsewhere` with `CodexBox` (server ID, name, addresses), and Oriel is to
  offer connecting to the Codex box for uploads and collection administration.
- **Show boxes and settings backups in Oriel**: on a show box the backup routes (lists, Oriel's
  preferences) answer `409 codexElsewhere` naming the Codex box (M13); Oriel is to send them
  there.
- **Restore preview for another box** against its settings now (needs a Codex → box request;
  today against its newest snapshot on the Codex, `currentAsOf`).

Later (from the decisions): per-document version history; full collection export/import;
ProPresenter import through the API; merging duplicate media; several collections per box;
per-collection permissions; settings templates and sync; moving the Codex to another box;
recovery from a show box's copy; Bible translations; Narthex signage collections; SongSelect and
CCLI.

## Risks

VPNs that NAT between buildings (unsupported); the Codex as a single point of failure (hence the scheduled export);
regressions from the M1 refactor; resumable migration on real installs (tests use synthetic data
only); live-output contention on a Codex box that also runs outputs (caps and low priority for
hashing, probing, thumbnails and channel serving); version skew between Codex and show boxes; disk
space; backups hold secrets (encrypted exports, admin-only contents, never the device identity);
revision stability (serialization must stay deterministic).
