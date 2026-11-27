# Vesper Oriel: redesign plan

Approved by the owner on 2026-11-23: every recommended answer in [section 7](#7-questions-for-the-owner)
is accepted. Round 1 started the same day.

**Inputs.** The owner's design canvas (the Oriel boards: Show, Slide editor, Reflow, Library, Bible,
Stage display, Timers, Screens & Looks, Site management), the design analysis of 2026-11-12,
the owner's answers of 2026-11-19, the Codex decisions (2026-11-19 to 11-23), and today's code
(`apps/control`) and API (`crates/protocol/README.md`). Where the canvas and `docs/PRODUCTS.md`
disagree, the plan doesn't pick a winner: each disagreement is a question in
[section 7](#7-questions-for-the-owner), with a recommended answer.

**How to read it.** Sections 1, 4 (each page's first lines), 5 and 7 are for the owner. Sections 2,
3 and the "API" and "Server gaps" lines are for the agents who build it. Server gaps have IDs
(**G1**, **G2**, …), listed together in [section 4.11](#411-server-gaps-in-one-list) so they can be
scheduled.

---

## 1. Goal and scope

**Goal.** Oriel gets the look and layout of the canvas: one compact header instead of a menu bar
and a big toolbar, full pages instead of pop-up dialogs, the dark violet-accented palette, Inter
everywhere, and the Library page on top of Codex. It stays a remote control: every change goes
through the server's REST and WebSocket API, and Oriel keeps no show logic of its own. It runs on
Windows and Linux, as the desktop app and in a browser.

**In this redesign**

| Area | What the owner will see |
|---|---|
| Look | The canvas palette, type, spacing and icons on every screen; Inter bundled in the app |
| Shell | Header with the Oriel menu, Show / Edit / Reflow, page buttons, Look picker, connection pill; offline and "reconnected" banners; status bar; one clean shortcut table |
| Show | Playlist as one scrolling run of sections, the new left column, the right column with output buttons, coloured layer buttons and transport, the tabbed bottom bin |
| Library | The Codex-backed library page: songs by shelf, media, themes, props; search; uploads; view and edit; collections; trash |
| Editors | Slide editor and Reflow restyled as full pages |
| Timers, Stage | Full pages over today's server features |
| Screens & Looks | One page replacing the Configure Screens dialog; Looks once the server can save looks |
| Site | Devices, join requests and Oriel pairings, Codex collections and storage, settings backups and restores, Codex export and import |
| Settings | General, Audio, Network & IDN, Keyboard, About; Oriel's own preferences backed up to the Codex |
| Bible | The page, once the server's scripture module exists (G11) |

**Later, or another product**

- **Vesper Launcher** (its own boards): installs and updates. Oriel only cooperates: Site ▸ Updates
  appears once the Launcher's headless API exists (G10), and the Launcher's "Open Oriel" just
  starts Oriel (or opens the server's address in a browser).
- Mosaic, Cantor, Transept, Herald and Narthex views: their own boards and rounds.
- Imports other than typed lyrics and media files (PowerPoint, PDF, Word, SongSelect, another
  app's library through the API), exports of presentations and playlists (PDF, images, video,
  order of service, packages), printing: menu items shown disabled with "coming later".
- The phone/tablet "stage manager" layout (*suggested* in PRODUCTS.md).
- Features waiting on rendering work in the server: scrolling text, push/wipe/zoom transitions,
  video elements on slides, object builds (G16, G18).

---

## 2. Approach

### 2.1 Rebuild in place, one working app at every step

Most of what the canvas needs already works in today's Oriel: connection, pairing, live state,
commands, previews, thumbnails, editing with undo, Reflow, timers, messages, props, stage, media
bins, audio, screen configuration, Codex search. So this is **a redesign of the existing app, not a
new app tree**. Each round replaces a part (tokens, then the shell, then one page at a time) and
leaves Oriel fully working, because the owner tests it on real hardware between rounds.

- **No long-lived branch, no feature flag.** Each round merges to `main` when its checks pass and
  the owner has seen its screenshots. A round that can't finish leaves the old screen in place.
- **Old screens stay reachable until replaced.** In round 2 the new header routes to the existing
  components (the current Library panel, Configure Screens dialog, Timers and Stage tabs) until
  their pages land. Nothing disappears before its replacement exists.
- **Capabilities decide what shows.** Oriel already reads `GET /api/v1/server`'s capabilities;
  pages and buttons for features a server lacks are hidden or shown disabled with a reason (e.g.
  Library needs `codex`, export needs `codexExport`, Site needs `site`).

### 2.2 Code layout

```
src/
  app.css                 design tokens (CSS custom properties) and base styles
  assets/fonts/           Inter (woff2, OFL licence file alongside)
  lib/ui/                 shared components (section 2.4)
  lib/shell/              Header, OrielMenu, ModeSwitch, PageLinks, ConnectionPill, Banners,
                          StatusBar, Toasts, ShortcutHelp; page routing
  lib/pages/show/         Show page (left column, slide area, right column, bin)
  lib/pages/library/      Library page
  lib/pages/editor/       Slide editor page (today's lib/components/edit, moved)
  lib/pages/reflow/       Reflow page
  lib/pages/{bible,stage,timers,screens,site,settings}/
  lib/server/             connection layer (unchanged role); new codex.ts, site.ts REST clients,
                          codexBox.ts (the side connection to the Codex box, 2.5)
  lib/state/              per-area state (2.3)
```

### 2.3 State

`lib/session.svelte.ts` (about 1,500 lines) holds all UI state today. Round 2 splits it so that
parallel rounds don't collide in one file:

- `state/shell.svelte.ts`: current page and mode, dialogs, menus, toasts, shortcut dispatch.
- `state/show.svelte.ts`: selections, open playlist, slide view and size, bin choice.
- `state/library.svelte.ts`: collection, shelf/folder, list sort and filter, selection, view/edit.
- `state/site.svelte.ts`, `state/screens.svelte.ts` (today's `screenConfig.ts` logic moves here).
- Live show state stays in `lib/server/state.ts` and `lib/live.ts` (the server is the only source
  of truth; nothing is edited locally).
- Pure logic (sorting, grouping, shortcut matching, upload chunking, restore summaries) lives in
  plain `.ts` files with vitest tests, as today.
- `documentChanged`, `codexChanged` and `siteChanged` events trigger refetches through one small
  dispatcher, so each page subscribes to what it shows.

### 2.4 Design tokens and components

Tokens come from the canvas (the analysis's "design language" table is the reference):
surfaces `#0B0C0F` … `#2A2D35`, lines `#2C2F37` / `#3A3D46`, text `#ECEDF1` / `#C5C8D1` /
`#A3A7B3` / `#8A8E9A`, violet accent `#6E5CE6` (selection `#2F2A55`, ring `#A99BFF`, links
`#C9C0FF`), live amber `#F5A524`, layer colours (Media `#4FB487`, Slide `#F5A524`, Props
`#5B8DEF`, Messages `#E86A6A`, Audio `#A99BFF`), group colours, radii (6 px default, 8 px cards,
10–12 px dialogs), shadows, 13 px base type with 11 px uppercase section labels. Status colours
(question 8, answered): violet for selection, amber for live and warnings, red only for errors and
offline, green for "on / healthy", grey for "off", blue for information only.

Shared components in `lib/ui/`, built once in round 1: Button (primary violet, secondary,
outline, danger, amber "live"), Chip (LIVE, NEXT, DISABLED, EDITED, TEST, IN SET), Badge,
StatusDot, SectionLabel, Card, Switch, Segmented, Tabs, Field, Select, ColorWell, Slider,
DataTable (sortable, sticky header), Menu/Popover (✓ marks, separators, keyboard), Dialog,
Toast, Splitter (6 px, glows violet, arrow keys, double-click resets), EmptyState,
PageToolbar, SlideTile, "+ New …" dashed button. Icons stay `lucide-svelte` (already bundled),
16 px, stroke 1.8.

### 2.5 Fonts, network and the Codex box

- **Inter is bundled** (woff2 files in `src/assets/fonts`, its OFL licence next to them). The
  canvas's Google Fonts links are not used. Slide fonts are drawn by the server; the editors'
  font pickers list `GET /api/v1/fonts` as today.
- **No external network at runtime:** no CDN, no web fonts, no analytics. The only hosts Oriel
  talks to are Vesper servers of the site.
- **Two servers at once, when needed.** On a show box, uploads, collection management, settings
  backups and Codex export answer `409 codexElsewhere` with the Codex box's name and addresses.
  The site key Oriel holds works on every server of the site, so Oriel can reach the Codex box
  without pairing again. Oriel then asks once: "Uploads and collections are managed on
  *Codex box name*. Use it for this?" and, on yes, sends those requests there through a REST-only
  side connection (`lib/server/codexBox.ts`), with a small "Managed on *name*" note on the page.
  If none of the addresses answers, it says so and offers to switch Oriel's main connection to
  the Codex box. Show-time edits keep going through the show box (write-through).

---

## 3. The shell

### 3.1 Header (every page)

Left to right, as drawn: **Oriel menu** (logo button), **Show / Edit / Reflow** segmented control,
page buttons **Bible · Library · Stage · Timers · Screens · Site** (current page highlighted),
spacer, **Look** picker (wired to `setLook` and `GET /looks`; it lists the box's looks, "Every
layer" for `null`), **?** (shortcut list), **connection pill** (dot plus "Connected" /
"Reconnecting…"; its popover shows the server name, Codex role and collection, and Reconnect).

- **Oriel menu**: the canvas's two-pane menu (File, Edit, View, Import, Export, Settings, Help).
  Built on today's `menu.ts` model and keyboard handling. **No Open, Save, Save As** (the server
  saves every edit; owner, 2026-11-19). Kept: Export ▸ Back Up Codex… and Import ▸ Codex export
  file… (both go to the Site page's export and import), Import ▸ Lyrics from Text… and Media
  Files…. Items for later features are shown disabled with "coming later", never as a mock toast.
- **Page toolbar** (second row on every page but Show): the page or document name, page
  controls, then **Done** (Edit, Reflow, Screens) or **Close** (Bible, Stage, Timers, Site,
  Library). Both go back to where the operator came from.
- **Routing**: pages are full screens replacing the workspace; `shell.page` plus `shell.mode`,
  mirrored in the URL hash so browser Back works and a page can be bookmarked.
- **Status bar** (Show page bottom): one dot per screen from `state.screens` (name, role, "off"
  when disabled, its output problems), then "Configure screens". Mosaic displays join it once
  their status exists (G9).

### 3.2 Keyboard shortcuts (the cleaned-up table)

The owner agreed (2026-11-19) to one clean table instead of the canvas's clashing lists. Proposed:

| Keys | Action | Note |
|---|---|---|
| Space, →, ↓, Page Down | Next slide | |
| ←, ↑, Page Up | Previous slide | |
| F1 | Clear all | F1 is no longer Help |
| F2 / F3 | Clear slide / Clear media | |
| F4 / F5 / F6 | Clear audio / Clear props / Clear messages | today's order, kept; the canvas's help list (props / messages / audio) is changed to match |
| Ctrl+1 / Ctrl+2 | Audience / Stage screens on or off | PRODUCTS.md and today; the canvas's "Ctrl+1/2/3 = modes" is dropped |
| Ctrl+E / Ctrl+R | Edit / Reflow (press again: back to Show) | |
| Ctrl+S | Back to Show | today's key; there is no Save |
| Ctrl+L | Library | |
| Ctrl+B | Bible | |
| Ctrl+T | Stage display | desktop app only (browsers keep Ctrl+T); menu in a browser |
| Ctrl+Alt+T | Timers | |
| Ctrl+Alt+S | Screens & Looks | replaces Ctrl+Alt+1 |
| Ctrl+Alt+M | Site management | |
| Ctrl+M | Show or hide the media bin | replaces Ctrl+Shift+V; Ctrl+M no longer opens Messages |
| Ctrl+= / Ctrl+− | Larger / smaller slide thumbnails | desktop app only; a browser zooms the page |
| Ctrl+F | Find in the library | focuses the Library search |
| Ctrl+N / Ctrl+Shift+N | New presentation / new playlist | desktop app only, as today |
| Ctrl+Z; Ctrl+Shift+Z or Ctrl+Y | Undo; redo | a text field keeps its own undo |
| Ctrl+X / C / V, Ctrl+D, Delete, Ctrl+A | Cut, copy, paste, duplicate, delete, select all | editors and lists |
| Ctrl+Enter | Reflow: slide break at the caret | Backspace at a slide's start merges it up |
| Arrows (Shift) | Editor: nudge 1 px (10 px) | |
| Ctrl+, | Settings | |
| ? | Shortcut list | |
| Esc | Close menus and dialogs; cancel a drag | doesn't leave a page |

Dropped: the panel shortcuts for tabs that move or go (Audio, Props, Macros, Timers, Messages,
Stage tabs), F1 for Help, Ctrl+1/2/3 for modes, Ctrl+S Save, Ctrl+O, Ctrl+P. The same table feeds
the "?" dialog, the menus and Settings ▸ Keyboard (read-only list for now). Shortcuts are matched
by physical key, as today, and checked on Windows layouts where Ctrl+Alt is AltGr.

### 3.3 Notices and toasts

- Toasts bottom centre, 2.6 s, one per notice `key` (rolling notices replace each other, as today).
  Errors stay until dismissed. Never more than three on screen; older ones fold into "N more" that
  opens Diagnostics.
- Command failures say what failed in plain words ("Couldn't clear the props: the server is
  restarting").

### 3.4 Connection, offline and pairing

- **Offline banner** (red, under the header): "Not connected to the Vesper server —
  reconnecting…", the attempt count, "Retry now". The canvas's line "nothing you change here is
  lost" is **not** used as written: commands are never queued (a slide must not fire late), and
  edits on a show box are refused while the Codex is away. The banner says instead: "Slides and
  edits wait until the connection is back." (Reflow's short typing queue stays, as today.)
- **Reconnected banner** (green, 3.5 s): "Reconnected — everything is back in sync."
- **Codex unreachable on a show box** (`GET /codex`'s `sync.reachable: false`): an amber line in
  the Library and editors: "Editing needs the site's server (*name*). The show keeps playing."
  Editing controls go read-only until it is back.
- **Pairing**: the connect screen (`ConnectScreen.svelte`) keeps its flows (server list from
  mDNS, address, code, waiting for an admin's approval, wrong code at a relay, try again, take
  the request back) and is restyled in the new look with the Oriel logo. The canvas has no
  connect screen, so it follows the canvas's dialogs and cards.
- **Key revoked** (socket closed with 4003): back to the connect screen with "This computer was
  removed from the site. Pair again."

---

## 4. Pages

Each page: **Canvas** (what the board shows, briefly), **Today**, **Changes**, **API**, **Server
gaps**.

### 4.1 Show (operator view)

**Canvas.** Three resizable columns over a bottom bin and a status bar. Left: the shelf and its
items over the playlist tree (drag to reorder, spring-loaded folders, "move to top level" drop
zone, inline rename, headers with colours). Centre: the whole playlist as one scrolling list of
sections with sticky headers, header bands, LIVE / NEXT / DISABLED, a pixel size slider,
Grid / Text / List; right-click toggles disabled; dragging a bin item onto a slide adds a media
cue. Right: Audience / Stage output buttons, screen picker, preview with LIVE / STAND-BY and an
overlaid stereo meter, Clear All, five coloured layer buttons, Video/Audio transport, tabs
Timers / Messages / Stage. Bottom bin: Media / Audio / Props tabs with bin playlists, Video Inputs.

**Today.** One presentation at a time in the slide area, a column-count slider, a vertical icon
layer strip, master meters in their own column, six icon tabs (Audio, Stage, Timers, Messages,
Props, Macros), a media bin with a folder tree.

**Changes.**
- Centre becomes the playlist-as-sections view; thumbnails load per section as it scrolls into
  view (prefetch per presentation), so long playlists stay fast. Picking a library item outside
  the playlist shows it as a single section with "Add to *playlist*".
- Disabled slides stay clickable to select; right-click toggles disabled (`setSlide {enabled}`),
  with the context menu on Shift+F10 / the menu key and on the slide's "…" button.
- Media cues: drop a bin item on a slide (`addSlideAction` media), ✕ removes it
  (`removeSlideAction`); triggering such a slide is the server's normal slide-action run.
- Right column as drawn; Audio and Props move to the bottom bin; the Messages tab builds one
  field per token of each message (`messageShow {tokens}`); Macros move to a bin tab "Macros"
  (question 1).
- Layer buttons clear a lit layer and bring back what was cleared off an empty one (G4, `restoreLayer`).
- The song "Arrangement" picker in a section header uses `setItem {arrangement}`; `next` and the
  stage's next slide follow it (G6).
- **The grid in play order** (batch 6, lane S). With an arrangement (the item's, else the song's
  own) a section shows the arrangement's sequence, as `next` plays it: a repeated group gets a
  tile per occurrence, numbered by its place in the sequence; clicking one sends `triggerSlide
  {step}` so `next` continues from that occurrence. Groups the arrangement leaves out follow in a
  collapsed, dimmed "Not in arrangement" part (recommended over hiding them: still one click for
  an ad-lib; opened on its own while one of them is live). LIVE / NEXT mark the occurrence the
  server reports (`SlidePosition.step`, also on `state.cursor`, so the stage's next agrees);
  without a step (an older server) the first occurrence. No arrangement: document order as before.

**API.** Libraries, presentations, playlists (`/playlists/{id}/edits`), `triggerSlide`,
`next`/`prev`, `clearLayer`/`clearAll`, `setScreensEnabled`, `subscribePreview`, transport
commands, `setVolume` (dB, the slider maps 0–100), timers, messages, props, `setStageLayout`,
`stageMessage`, `/media-bins`, `triggerMedia`, thumbnails.

**Server gaps.** G1 (author, year, kind for the dots and two-line rows), G4 (un-clear), G5
(loop toggle on the playing item), G6 (arrangements), G7 (Video Inputs tab), G9 (Mosaic in the
status bar).

### 4.2 Library (Codex) — the priority

**Canvas** (board "Library"). A full page in three parts.
1. **Where** (left): a card "Library *name* in Codex on *box*" that opens the list of the Codex's
   libraries (= **collections**, question 6), the Codex box and its state, "One Codex for the whole
   site", and "Manage libraries and backups" (→ Site). Then a tree: **Songs** (All songs, then
   folders such as Hymns, Christmas, Kids), **Media** (Images, Videos, Live streams), **Design**
   (Themes, Props). At the bottom a connection card: Connected / Working offline, Synced /
   Syncing / Waiting to sync, "Sync now", "2 others have this library open". The column folds into
   an icon rail while editing.
2. **What** (middle): the folder's items as a sortable table (Title / Author / Year; Theme /
   Typeface / Used; Prop / Position) with a filter, or a tile grid for media; "+ New song",
   "+ New theme", "+ Import", "+ Add source". While a song is edited, this column shows its slide
   thumbnails grouped by section.
3. **The item** (right): kind, name, meta, a **View | Edit** switch, "Add to *playlist*" (or
   "Apply to…" for a theme), Save / Revert while editing, and a lock banner "*X* has this song open
   for editing … Take over editing".
   - **Song, view**: arrangement chips, its theme ("Open theme →"), slides with group labels.
   - **Song, edit**: a slide canvas whose lyrics box follows the theme (drag or resize it to
     override on this slide, "Reset to theme"), extra objects (text, shape, image, video,
     scrolling text, timer), undo/redo, snap, rulers, "Preview as output look"; side panel with the
     object list and tabs **Object / Slide / Song** (object styling, visibility conditions; slide
     label, order, stage notes, build, transition; song title, author, copyright, folder, show
     copyright, artwork, theme, background, arrangement order, size and transition).
   - **Theme**: preview with sample content (Verse / Chorus / Scripture / Long verse) and a
     fit readout; tabs **Text / Background / Layout / Groups** (typeface, size, shrink to fit,
     lines per slide, shadow, outline, box; image / video / live / gradient / colour / none with
     darken and blur; layout presets; per-group style and colour); "Used by N songs".
   - **Media / prop**: preview, properties, "Used by".

**Today.** A Library panel in the left column (shelves and a flat list, double-click opens
Reflow), a search box using `GET /codex/search`, no Codex views at all.

**Changes (first release of the page).**
- **Collections**: the "where" card lists `GET /codex/collections`; picking another collection
  is the box's assignment (`PUT /codex/assignment`), with the confirmation "Something is live
  on this box (*what*). Switch anyway?" on `409 liveShow`, then `confirm: true`. Admin-only
  actions show the site's unlock or sign-in prompt on `403`.
- **All collections view** (from the card's "Manage…", and Site ▸ Codex): create, rename,
  colour, description, delete (into the collection trash), and the **collection trash**, seen
  only here: restore, or "Delete for good" with a second explicit confirmation naming it.
- **Songs**: "All songs" plus one folder per **library shelf** of the collection
  (`/libraries`); rows from `PresentationSummary` (author and year need G1); filter; sort; an
  A–Z jump rail when sorted by title; search box over `GET /codex/search` (this collection, or
  "All collections").
- **Media**: Images and Videos from the collection's media (`/codex/collections/{id}/media`,
  thumbnails by SHA-256); a **Missing** filter with relink (pick a library item, "Find missing
  files" = `media/search`, "Match a folder on this computer" = `media/match` then uploads with
  `replaces`); **Import** uploads files resumably (`/codex/uploads`, chunked with `Content-Range`,
  SHA-256 computed in a Web Worker, resume after a reconnect, progress per file, cancel), into a
  chosen bin. Uploads go to the Codex box (2.5). Storage line from `GET /codex/storage`.
  "Live streams" wait for question 6 and G7.
- **Themes**: listed and previewed (template slides from `GET /themes`, their thumbnails), and
  since batch 5 editable (G2, question 3: linked themes, `docs/THEMES-PLAN.md`): new, duplicate,
  rename, delete with used-by (move the songs to another theme or keep their look), "Edit in
  slide editor"; a song shows its theme with a picker and "Reset to theme"; the editor's
  inspector marks what a slide or object overrides.
- **Props**: list, preview (`/props/{id}/thumbnail`), create, rename, delete; edit opens the
  slide editor with the prop document (as today).
- **View / Edit**: View shows the song as drawn. **Edit** reuses the slide editor's canvas and
  inspector (4.3), arranged as the board draws it (Object / Slide / Song tabs). Edits save as
  they happen (edit ops), so there is no Save; Revert becomes Undo (question 5).
- **Add to playlist** target picker in the page toolbar (the board's "Add to" select); double-click
  or Enter adds; drag onto a playlist in Show still works.
- **Item trash**: "Recently deleted" at the bottom of the tree (`GET /codex/trash`, restore;
  "Empty trash" for admins).
- **Not in the first release**: presence and locks ("2 others…", "Take over") wait for G8;
  offline editing is not offered (question 4); "Synced to 3 of 3 Vespers" waits for G14.

**API.** `/codex`, `/codex/collections` (+ trash, assignment), `/codex/search`, `/codex/trash`,
`/codex/uploads`, `/codex/collections/{id}/media` (+ relink, search, match, reroot),
`/codex/blobs/{sha256}.{ext}/thumbnail`, `/codex/storage`, `/libraries`, `/presentations`,
`/themes`, `/props`, `codexChanged`, `documentChanged`; on a show box the side connection for
uploads and collection admin.

**Server gaps.** G1, G2, G7 (live streams), G8, G14, G15 (offer to upload a show box's dormant
collection), G17 (media bins can't be created or edited over the API today).

### 4.3 Slide editor

**Canvas.** Toolbar with undo/redo and inserts (Text, Shape ▾, Media ▾, Scrolling text, Timer),
a text formatting bar, the canvas with rulers, centre snapping, marquee, multi-select, a text
field under the canvas, an inspector whose tabs follow the selection (Presentation / Slide /
Build; Shape / Text / Build; Arrange / Build) and the object list docked in the inspector.
Visibility conditions, linked text, Copy/Paste Style, align and match size.

**Today.** The Edit view exists (`lib/components/edit/`): rulers, snapping, multi-select,
object list on the left, a floating insert bar, inspectors Presentation / Slide / Build (stub) /
Shape / Text / Actions, in-place text editing, undo/redo, conflict dialog.

**Changes.** Restyle as a full page with the canvas's toolbar and formatting bar; object list
moves into the inspector; Arrange tab for multi-select; Copy/Paste Style (client-side over
`setTextStyle` / `setTextBox`); Group select on the Slide tab; transitions grid. Keep in-place
text editing **and** add the text field under the canvas (question 2). Keep slide actions as an
"Actions" tab (the canvas has none; they exist and the classic apps have them). Features the
server can't draw yet are shown disabled with a reason, never faked.

**API.** `/presentations/{id}/edits`, `/render`, `/fonts`, `/groups`, props and stage-layout
documents, `addSlideAction` etc.

**Server gaps.** G16 (scrolling text, push/wipe/zoom/slide transitions, video elements, dotted and
double strokes, missing link sources and visibility conditions, artwork), G18 (builds).

### 4.4 Reflow

**Canvas.** A full page: text blocks per slide in bands by group (vertical label, colour bar),
light boxes in serif italic capitals, a "…" menu per slide (start a group, continue the group
above, slide break, merge up, remove media cue, delete), hotkey and cue badges, thumbnails on the
right, size sliders, Done.

**Today.** Nearly the same structure inside the Show layout, with a group `<select>`, save status
and conflict handling.

**Changes.** Full page with Done; the "…" menu replaces the triangle select ("Continue the group
above" = `moveSlide` into the previous section); hotkey and cue badges; the canvas's text style
(fixed style; it doesn't follow the presentation's font). Keep the save status and the "Keep
mine / Use theirs" conflict prompt.

**API.** Existing edit ops. **Server gaps.** None.

### 4.5 Bible

**Canvas.** Translation picker, book list, chapter grid; reference or word search; verse
toggles; slide options (verses per slide, reference on slide, theme); generated slide previews;
"Add to playlist" and an amber "Go live".

**Today.** A disabled toolbar button.

**Changes.** The whole page, once the server has a scripture module. Bundled open translations
work offline; API.Bible and ESV appear when the church enters its own key (Settings ▸ Bible),
with each provider's attribution line on the slides.

**API / server gaps.** G11, all of it: translations list, books and chapters, passage fetch, a
reference parser, word search, slide generation with a theme, provider keys and caching, and
where generated presentations are stored (question 12).

### 4.6 Stage display

**Canvas.** Layout cards (Band, Speaker, Lyrics only, Production, "+ New layout"), stage screens
with a layout picker each, a large preview, the stage message with Show / Hide / Flash, and
element checkboxes (clock, timer, current, next, notes, message) with settings (timer, 12/24 h,
text size).

**Today.** A Stage tab: screens, layout thumbnails, message with flash and presets, "Edit
Layouts…" (layouts are free-form one-slide documents edited in the Edit view).

**Changes.** The full page. Picking a layout card shows it on the selected stage screen
(`setStageLayout`); the preview is the live stage screen (`subscribePreview`). The checkboxes
compose a layout document from data-link elements (create with a template, then edit ops); a
"Design freely…" button opens it in the slide editor (question 10). Message presets stay.

**API.** `/stage-layouts` (+ templates, document, edits, thumbnail), `setStageLayout`,
`stageMessage`, `stageMessageClear`, `state.stageScreens`, data links' clock and timer formats.

**Server gaps.** G12 (a box-wide clock format; today it is per data link, which works but isn't
one setting).

### 4.7 Timers

**Canvas.** Timer cards with live values and start/pause, a system clock card with 12/24 h, a big
96 px readout with status, Start/Pause, Reset, −1 / +1 min, and settings (name, type, duration or
target time, overrun, "turn amber with N minutes left", a summary, delete).

**Today.** A Timers tab with create, kind, duration/time, start/stop/reset, overrun and duplicate.

**Changes.** The full page; the Show page keeps a compact Timers tab. Elapsed timers keep their
optional "Stop at" (the server has it).

**API.** `/timers` CRUD, `timerStart` / `timerStop` / `timerReset` / `timerAdjust` (±60 s),
`state.timers`.

**Server gaps.** G12 (per-timer warning threshold; today colour triggers live on data links).

### 4.8 Screens & Looks

**Canvas.** One page with Screens | Looks. Screens: Audience and Stage lists with on/off
switches, screen cards (EDITED, TEST), identify, test patterns; a screen editor with colour,
arrangement (Single / Mirror / Grouped / Edge Blend), an output canvas, and output tabs
Hardware / Color / Corner Pin / Alpha Key, with an output picker (System, IDN, SDI,
Placeholder). "Apply live" or Apply / Revert. Looks: list, a layer matrix per audience screen
(mask, messages, props, announcements, theme override, slide, media, video input), Make Live,
transition.

**Today.** `ConfigureScreens.svelte`, a modal: lists, outputs, Apply / Revert / Undo, conflicts,
identify, role, size, frame rate, alpha, video delay, IDN groups, embedded audio, enable,
duplicate, delete. No look editor; test patterns not reachable.

**Changes.** The page replaces the dialog and keeps all its staging, validation and conflict
logic (`screenConfig.ts`). Shown: what the server does (IDN outputs, alpha, delay, background
colour, enable, identify, test patterns with "Show on every screen / this screen", embedded audio,
frame rate, duplicate and delete: question 9). Looks: the list and "Make Live" now
(`setLook`); the matrix editor once looks can be saved (G3).

**API.** `/screens`, `/outputs`, `/output-transports`, `identifyScreens`, `setTestPattern`,
`setScreensEnabled`, `/looks`, `setLook`, `/themes`.

**Server gaps.** G3 (look create/update/delete, masks), G13 (display and SDI outputs,
placeholders, Mirror / Grouped / Edge Blend, Fill/Key, colour correction, corner pin, the list of
the server's displays).

### 4.9 Site management

**Canvas.** Toolbar with the site name, device counts and the site code ("single use", expiry,
New code) or "Site codes are off". Left tabs with badges: **Devices** (role filter, Identify all,
table with roles, status, version, CPU, GPU, uptime; Identify, Restart, Logs, Remove from site
with a confirmation), **Join requests** (join mode radio: code or approval / code then approval /
approval only; request cards with the comparison code, name, role, Deny, Approve; "Recently"),
**Updates**, **Codex libraries** (one Codex, its size, last backup, Back up now; a table of
libraries), **Backups** (per product and machine, schedule, Restore… with a preview, Export).

**Today.** Nothing (pairing is on the connect screen; Settings shows server info).

**Changes.**
- **Devices**: `GET /site/devices` with health, problems, version; Identify, Restart, Logs
  (snapshots, file view), Remove (confirmation as drawn). The Codex's own membership shows as
  **"Codex (built-in)"** with no Remove. Each machine's details (system, CPU, GPUs, memory,
  uptime: G9, from its IDN Node) under its name; roles aren't reported yet.
- **Join requests**: machines asking to join (`/site/join-requests`, approve with a name, deny)
  **and Oriel computers waiting for approval** (`/site/pairings`, approve, deny with a reason);
  the join mode; the approval window ("Open a 10-minute setup window"); the site code
  (`/site/join-code`, "New code"); "Recently" from `/site/events`.
- **Codex** (named "Codex" in the tab; question 6 for "libraries" vs "collections"): the Codex
  box, the all-collections table (4.2), storage, and **Protect the Codex**:
  - **Download an export now**: passphrase twice (at least 8 characters), the note "Media files
    aren't included: back up the media folder separately", the tick box **"Include the site's
    identity"** (off by default; only on the site's server; `CreateExport.includeSite`), then the browser/desktop download.
  - **Nightly exports**: on/off, folder on the Codex box, time, keep 7 nightly + 4 weekly, the
    stored passphrase (set / clear; never shown), last and next export, problems in words, the
    list of export files.
  - **Import an export**: upload the file, passphrase, a preview per collection (new / same /
    newer / older / in trash), settings backups and the site identity, choose replace or skip,
    apply; "Restart needed" when the site identity was staged.
  The canvas's per-library "Back up now" and "Last backup" become this one export (question 11).
- **Backups** (settings): one row per backup set (Vesper per box, Oriel per computer), last
  snapshot, "Back up now", **Restore…** with the server's preview ("3 screens, 2 looks changed;
  1 output binding added", items to tick, what needs a restart), the result, and queued restores
  to show boxes with their state (queued / applying / applied / failed / expired, Cancel). A
  replacement box can take over a removed box's identity ("adopt") with a clear warning. The
  canvas's schedule picker becomes a read-only "Daily and after changes" (question 11).
- **Security** (not on the canvas, question 11): access level (open / PIN / users), unlock and
  lock, users, the site's API keys ("Paired computers") with revoke, the audit list.
- **Membership** (in Security or a "This server" card): join another server's site with its
  address and code, the join state, leave.
- **Updates** (G10, first round done: `docs/LAUNCHER-PLAN.md`): every machine's Launcher with its
  products, versions, state and health, "Update available", Update / Roll back (queued for the
  Launcher's next report), and the site's update channel (upload a signed package with its
  `.minisig`, delete). Shown when the server announces `updates`.

**API.** `/site`, `/site/settings`, `/site/session`, `/site/unlock`, `/site/devices` (+ identify,
restart, logs, allow), `/site/join-requests`, `/site/pairings`, `/site/join-code(s)`,
`/site/pairing`, `/site/api-keys`, `/site/users`, `/site/audit`, `/site/events`,
`/site/membership`, `/site/join`, `/site/leave`, `siteEvent`, `siteChanged`; `/codex/backups…`,
`/codex/export…`; on a show box the side connection for backups and export.

**Server gaps.** G9, G10, G14.

### 4.10 Settings and other dialogs

- **Settings** (Ctrl+,), a page with sections: General (this computer's name, theme options,
  thumbnail size defaults), Audio (today's Audio settings), Network & IDN (the server, its
  address, version, protocol, IDN library version, output transports), Keyboard (the table),
  Bible (provider keys, once G11), About. Kept from today's dialogs.
- **Oriel's preferences backup**: Oriel saves its preferences (panel sizes, views, bin choice,
  shortcuts list version) to the Codex (`PUT /codex/backups/oriel`) a few seconds after they
  change, and offers "Restore my Oriel settings" on a new computer
  (`GET /codex/backups/oriel`). On a show box they go to the Codex box (2.5).
- **Diagnostics**, **About**, conflict prompts, name and confirm dialogs: restyled.

### 4.11 Server gaps in one list

| ID | Gap | Needed by | Size (rough) |
|---|---|---|---|
| G1 | **Done (batch 4).** Presentation summaries with authors, copyright year, CCLI number, kind (song, scripture, text, media) and theme | Show rows and dots, Library table | small |
| G2 | **Done (batch 5, `docs/THEMES-PLAN.md`; batch 9: template slides and roles in the editor, imported songs linked keeping their look).** Themes: create, edit, delete; songs that follow their theme with per-slide overrides; "used by" | Library themes, song editing | large (question 3) |
| G3 | **Done (batch 8, `docs/LOOKS-MASKS-PLAN.md`).** Looks: create, update, delete, transition; masks (documents and rendering) | Looks editor | medium; masks large |
| G4 | **Done (batch 4).** Restore a cleared layer ("un-clear") | Show layer buttons | small |
| G5 | **Done (batch 4).** Loop on/off for the playing item | Show transport | small |
| G6 | **Done (batch 4).** `next` and stage "next" follow arrangements | Show arrangement picker | medium |
| G7 | **Done apart from the items below (batches 9, 11; `docs/LIVE-SOURCES-PLAN.md`):** live IDN sources on the video-input layer (`GET /sources`, `sourcesChanged`, `triggerLiveSource`, thumbnails), the bin's Video Inputs tab and the Show page's strip; the looks' Video input row hides it; batch 11: a source as an element fill (editor's "Live source"), logical inputs (`triggerVideoInput`, playlist items, slide actions), alpha, interlaced, H.264/HEVC (lane V), and its sound on the program bus, paired by timecode (lane A); batch 12 (lane I): logical inputs and their bindings (`/video-inputs`, the bin tab's Inputs with rename / bind / unbind / delete), the fill picker's inputs, NDI\|HX AAC sound, `VideoInput` visibility conditions follow the layer. Open: real-hardware checks (HX camera sound) | Bin's Video Inputs, Library live streams | large |
| G8 | **Done (batches 8, 9 and 11, `docs/PRESENCE-PLAN.md`).** Presence and edit locks (who has what open, take over): presence, hard locks with take-over, badges and banners in the Library, Edit and Reflow; site-wide over the box-to-Codex link (batch 9); browsing on the Show page, props and timers (locked where edited, playlists presence only), "2 others have this collection open" (batch 11) | Library | medium |
| G9 | **Mostly done (batch 9, IDN Beta8).** Device details: this Vesper's version, OS, CPU, GPU, uptime and products (`GET /server/details`) in Site ▸ Devices ("This Vesper") and the status bar; each machine's products from the services its IDN Node reports, and its system, CPU, GPUs, memory (free: "updated up to 30 s ago"), uptime and Node restart from `SiteDevice.machine` / `nodeStartedAt` (IDN Beta8 `DeviceHealth::machine`, `node_started`), in the same words as "This Vesper"; nothing for a device whose Node doesn't report them. Mosaic display status (batch 14): a display box's screens and what they show under its name, and Site ▸ Displays (`docs/MOSAIC-PLAN.md`) | Site Devices, status bar | medium |
| G10 | **Round 1 done (batch 13): `docs/LAUNCHER-PLAN.md`, `crates/vesper-launcher`, `/api/v1/updates`, Site ▸ Updates; tray app and first-run wizard next.** Updates over the API (the Launcher's headless mode) | Site Updates | large, other product |
| G11 | Scripture module (translations, passages, references, search, slide generation, providers; batch 9: passages split again when their theme changes, book names in other languages, `docs/SCRIPTURE-PLAN.md`) | Bible | large |
| G12 | **Done (batch 4).** Per-timer warning threshold; a box-wide clock format | Timers, Stage | small |
| G13 | **Mostly done (batches 5, 6, 11, 12; `docs/DISPLAY-OUTPUTS-PLAN.md`):** display outputs (KMS, windows), colour correction, corner pin, display list, Fill/Key on two connectors (batch 11), arrangements Single / Mirror / Grouped (bezels) / Edge Blend (gamma-aware ramps) with the output canvas, tiles, placeholders and identifying a free display (batch 12). Open: SDI outputs, per-seam blend widths and black-level uplift, vblank-locked clocks | Screens | large |
| G14 | **Done (batch 8, `docs/MEDIA-BINS-CODEX-PLAN.md`).** On the Codex box: which show boxes show which collection, and how far behind they are | Library, Site Codex | small–medium |
| G15 | **Done (batch 9, `docs/MEDIA-BINS-CODEX-PLAN.md`).** Upload a show box's dormant collection to the site's Codex | Library | medium |
| G16 | **Partly done (batch 8, `docs/TRANSITIONS-PLAN.md`):** push/wipe/zoom/slide transitions with direction and easing, scrolling text, video elements, double strokes; batch 11: video element sound on the slide layer (one player for every screen, Shape tab Mute / Level) and live source sound (G7 stage 3 item 3, `videoInput` layer level, the strip's mute). batch 12 (section 5 there): link sources (timer elapsed / remaining, presentation name and notes, CCLI copyright line, song artwork), visibility conditions (message, stage message, layer live, playlist item, video / audio state) with the editor's Visibility section, the song artwork field. batch 14: the CCLI line on its own on a song's first / last slides (`ccli.showOn`), the Visibility section's "Video input live" condition (the show's inputs to pick from). Open: poster frames, scrolling text fades | Slide editor | large |
| G17 | **Done (batch 8, `docs/MEDIA-BINS-CODEX-PLAN.md`).** Media bins (and global groups) editable over the API | Library media, bin playlists | small–medium |
| G18 | **Done (batch 9, `docs/BUILDS-PLAN.md`).** Object builds: build order with triggers, effects and delays; Next steps through them; imported builds | Slide editor, Library slide tab | medium |
| G19 | Found in round 3: render a theme's template slides (thumbnails, done in batch 5); "used by" for media and props (done: `GET /media/{id}/used-by`, `/props/{id}/used-by`, the Library item's "Used by"); the Codex box's name in `GET /codex`'s sync part (done: `sync.name`, from the box's hello); what is live in the `liveShow` error (Oriel builds it from the show state today); found in round 4: a list of the media library's items (done: `GET /codex/library`, relink offers every item of the file's kind; checked in batch 11), an in-app unlock or sign-in for admin actions answered 403 (done in round 11 by Oriel); found in round 11: a device's services in `SiteDevice` (done with IDN Beta6: Restart offers a picker of them), a way for a non-admin to take over another computer's Oriel preferences (today `GET …/snapshots/{id}` needs an admin) | Library, Site | small each |
| G20 | **Done (batch 13, `docs/CONTROL-SURFACES-PLAN.md`).** Control surfaces: a Bitfocus Companion module (`integrations/companion/`), Remote API compatibility for older Stream Deck / Companion profiles (Settings ▸ Network & IDN ▸ Control surfaces, off by default), macros triggerable (`triggerMacro`, `GET /macros`). Open: a Macros bin tab in Oriel | Settings, Show bin | medium |
| G21 | **Done (batch 13, `docs/OMT-PLAN.md`).** OMT (Open Media Transport): outputs beside IDN ones (alpha, embedded audio) and OMT sources (`omt:` names) on the video-input layer, in fills and as logical-input bindings; the output picker and the bin's Video Inputs tab show them. IDN v0.3.3: the received colour matrix, premultiplied alpha both ways, previews for thumbnails. Open: tally, quality profiles | Screens, Show bin | medium |
| G22 | **Done (batch 13, `docs/CAPTIONS-PLAN.md`).** Live captions: the captions layer (top-most, per look), speech from the program audio or a live source (whisper on the server's CPU, models fetched on first use) or lines from another captioner, Settings ▸ Captions, the Show page's captions strip with a live line, the looks matrix's Captions row. Open: CEA-608 in streams and recordings, GPU whisper in a helper process, hardware audio inputs | Show, Settings, Looks | medium |
| G23 | **Done (batch 13, `docs/RECORD-STREAM-PLAN.md`).** Recording and streaming: a screen with the program audio recorded (MP4/MKV, split, free-space checks, `<data>/recordings`) and streamed to RTMP(S) / SRT targets with write-only keys, status over REST; the Show page's REC / LIVE indicator and Settings ▸ Recording & streaming. Open: pause/resume, adaptive bit rate, a WebSocket status message, Windows encoder check | Show right column, Settings | large |

None of them blocks rounds 1–9 below: those use what exists and hide or disable the rest.

---

## 5. Rounds

Each round ends with the checks in section 6 and a set of screenshots for the owner (1920×1080
and 1366×768, compared side by side with the board). Estimates are agent working time, measured against this project's real runs (a full server milestone such as Codex M13 took one agent about 40 minutes with tests and docs); they don't include the owner's review between rounds. Recalibrated on 2026-11-23 from rounds 1, 2, 3, 5 and 7, which took about half their first estimates; the open rounds show the new figures.

| # | Round | Owner looks at | Depends on | Estimate |
|---|---|---|---|---|
| 1 | **Foundations** (done 2026-11-23): tokens, Inter bundled, `lib/ui` components, today's screens restyled; the screenshot harness and the synthetic test data (6.2) | Every existing screen in the new look | answers to questions 1, 8 | 45 min (took 25) |
| 2 | **Shell** (done 2026-11-23): header, Oriel menu, mode switch, page routing (old components behind the new buttons), connection pill, banners, toasts, status bar, the shortcut table and "?" dialog, connect screen restyled; `session.svelte.ts` split (2.3) | Header and menu on every page, offline and reconnect banners, shortcut list | 1 | 45 min (took 28) |
| 3 | **Library, browsing** (done 2026-11-23, lane A): Codex REST clients, the side connection to the Codex box, the page's three parts, collections card and switch (with the live confirmation), songs by shelf, search, media tiles, themes and props read-only, add to playlist, item trash | Library page with synthetic collections, the liveShow prompt, a show box's "managed on" prompt | 2 | 1 h (took 20 min) |
| 4 | **Library, managing and editing** (done 2026-11-23, lane A): uploads with progress and resume, missing media and relink, storage, all-collections view and collection trash, song View/Edit with today's editor components in the board's arrangement | Uploading a folder, relinking, editing a song in the Library, deleting and restoring a collection | 3 | 30 min (took 27) |
| 5 | **Show, left and centre** (done 2026-11-23, lane B): sources and playlist tree, header bands, playlist-as-sections, LIVE/NEXT/DISABLED, sizes, Grid/Text/List, right-click disable, media cues | A long synthetic playlist, scrolling and triggering | 2 | 45 min (took 21) |
| 6 | **Show, right column and bin** (done 2026-11-23, lane B): output buttons, preview with meter, Clear All and layer buttons, transport, compact tabs, bottom bin with Media/Audio/Props/Macros | The full Show page during playback | 5 | 25 min (took 16) |
| 7 | **Reflow page** (done 2026-11-23, lane C) | Reflow with groups, menu, badges | 2 | 20 min (took 13) |
| 8 | **Slide editor page** (done 2026-11-23, lane A): toolbar, formatting bar, inspector reorganised, Arrange tab, Copy/Paste Style; Library's song editing picks the new look up | Editing a slide, multi-select, the inspector tabs | 4 | 30 min (took 16) |
| 9 | **Timers and Stage pages** (done 2026-11-23, lane C) | Both pages with running timers and a stage screen | 2 | 20 min |
| 10 | **Screens & Looks page** (done 2026-11-23, lane C) (Looks list and Make Live; the matrix after G3) | Screens editing with Apply/Revert, test patterns, identify | 2 | 25 min; 9 and 10 together took 23 |
| 11 | **Site page** (done 2026-11-23, lane B): devices, join requests and pairings, Codex (collections, storage, export, nightly exports, import), settings backups and restores, security, membership | Each tab on a two-box synthetic site | 3 (Codex clients) | 30 min |
| 12 | **Settings, preferences backup, dialogs; PRODUCTS.md and Oriel README updated** (done 2026-11-23, lane B) | Settings sections, "Restore my Oriel settings" | 2 | 15 min; 11 and 12 together took 29 |
| 13 | **Bible page** | Bible page with a bundled translation | G11 | 25 min after G11 |
| later | Looks matrix (G3), Screens extras (G13), Updates (G10), presence (G8), editor features (G16, G18) | | server rounds | |

**Open:** round 13 (Bible) once G11 exists. Rounds 1–12 were done on 2026-11-23, plus a smoke test of
rounds 1–7, 9 and 10 against a real server (four Oriel fixes, two server fixes).

**Follow-ups found in rounds 8, 11 and 12** (small, Oriel only unless marked):
- Editor at 1366×768: the Arrange tab's "Duplicate all / Delete all" sit half under the docked object list.
- Settings is a full-screen layer, not a registered page, so the header still highlights the page underneath.
- The Oriel menu's Settings pane still lists Inputs, Devices, Media, Import and Groups (they open a "can't be changed yet" pane): trim them.
- Pass `siteEvent` through the socket layer so "Recently" updates without refetching `/site/events`.
- Show "Already in this collection" from the upload answer's new `existing` flag.
- Diagnostics, conflict and prompt dialogs: finish the restyle.
- Check the export download (`<a download>` blob) in the desktop app on Windows (WebView2).
- Smoke-test rounds 8, 11 and 12 against a real server (two-box site for the Site page).


**Parallel work (up to 3 agents in git worktrees).** Rounds 1 and 2 run alone: they touch every
file. After round 2 the code is split by page (2.2, 2.3), so these can run side by side:

| Lane A | Lane B | Lane C |
|---|---|---|
| 3 → 4 (Library) | 5 → 6 (Show) | 7, 9, 10 (Reflow, Timers & Stage, Screens) |
| 8 (Slide editor, after 4) | 11 → 12 (Site, Settings) | 13 (Bible, when G11 exists) |

Rules for parallel lanes: each worktree has its own `node_modules` and, if it builds the server
for its dev data, its own `CARGO_TARGET_DIR`; shared files (`lib/ui`, `app.css`, `keyboard.ts`, the menu model,
the page registry) change only in rounds 1–2 or in a short, separately merged commit announced to
the other lanes; each lane rebases on `main` before review.

---

## 6. Testing

### 6.1 Every round

```sh
cd apps/control && npm run check && npx vitest run && npm run build
```

- **vitest** for all logic that isn't markup: shortcut matching (the table in 3.2, AltGr
  combinations), menu model, playlist sectioning and thumbnail windows, library sorting and the
  A–Z rail, upload chunking and resume (with a fake server), the `codexElsewhere` redirect,
  liveShow confirmation flow, restore summaries, Codex export form validation (passphrase twice,
  8 characters), event dispatch (`documentChanged`, `codexChanged`, `siteChanged`).
- Server-side changes for any G item follow CLAUDE.md (integration tests as modules of the one
  server test binary, clippy, bindings test).

### 6.2 Screenshots of every page

- **A stub server** in `apps/control/test/stub/`: fixtures for the REST routes and a scripted
  WebSocket (state, events, preview frames), so every page renders with deterministic content.
  Round 1 builds it with the screenshot script.
- **Synthetic data only**: made-up song titles and lyrics (not even public-domain hymns, which
  could match the church's own library), made-up names and hosts, generated media (colour
  bars, tones). A small generator writes the same content as a data folder for a real dev server
  (one Codex box and, for Site and show-box views, a second box joined to its site). **Never** the
  church's data in `data/import/` or `ProPresentor/`, and never a copy of it.
- **The screenshot script** (`apps/control/scripts/shots.mjs`, `playwright-core` with the
  headless Chromium already installed on the build machine) opens each page and its main states
  (offline banner, menus open, editing, dialogs) at 1920×1080 and 1366×768, writes PNGs to a
  scratch folder (never committed), and the agent **looks at them** against the board before
  asking for review.
- Once per page round, the same pages against a real dev server with the synthetic data folder, to
  catch what the stub hides (timing, previews, uploads).

### 6.3 Windows and Linux

- Linux: `npm run tauri build` after rounds 2, 6 and 12; the browser build served by `vesper`.
- Windows: the owner (or a Windows CI runner, if one is set up) checks the desktop app and Edge
  after rounds 2, 6 and 12: fonts (Inter, not Segoe), scrollbars, AltGr shortcuts, file pickers
  for uploads and export downloads, WebView2 focus behaviour.

---

## 7. Questions for the owner

**Answered (owner, 2026-11-23): yes to every recommendation below.** The text is kept as the record.

1. **Main window layout.** The canvas has one compact header with the Oriel menu and page buttons,
   output buttons and five coloured layer buttons under the preview, tabs Timers / Messages /
   Stage, and Audio and Props in the bottom bin. PRODUCTS.md specifies a menu bar, the big
   toolbar (Search · Text · Theme | Show · Edit · Reflow · Bible · More | Media · Look · Live ·
   Audience · Stage · Status), a vertical layer strip and tabs Audio, Stage, Timers, Messages,
   Props, Macros. The Library board also draws its own header (Show | Library | Reflow | Bible).
   *Recommended:* the canvas wins, with the same header on every page (the Library's "Add to
   playlist" picker moves into its page toolbar); Macros become a bottom-bin tab; the connect
   screen stays, restyled; PRODUCTS.md is updated in round 12.
2. **One editor or two?** In the Show board, "Edit" opens the selected song in the Library page's
   edit mode (lyrics box following the theme), while the Slide editor board is a separate,
   fuller editor; PRODUCTS.md has one Edit view with in-place text, a floating insert bar and an
   Actions inspector. *Recommended:* one editor built once: "Edit" opens the Library in Edit mode
   for songs; other presentations, props and stage layouts open the full Slide editor page. Keep
   in-place typing and add the text field under the canvas; keep slide actions as a tab.
3. **Themes linked or copied?** The Library board treats a theme as live: songs follow it, editing
   it changes every song that uses it, slides can override and "Reset to theme". PRODUCTS.md says
   themes are "applied as a copy", and the server can't edit themes at all yet.
   *Recommended:* linked themes, as drawn (it's what volunteers expect), as a server project (G2)
   scheduled after rounds 3–4; until then themes are read-only in the Library.
   *Approved and built (batch 5):* linked themes, `docs/THEMES-PLAN.md`.
4. **Editing while the Codex is away.** The Library board keeps edits "on this computer" and syncs
   them later; the Codex decision (2026-11-19) is write-through with no offline queue (edits are
   refused, the show keeps playing). *Recommended:* follow the Codex decision; the Library and
   editors turn read-only with an amber line while the Codex can't be reached.
5. **Save / Revert buttons.** The Library board has Save and Revert in Edit mode; you dropped Save
   because the server saves every edit. *Recommended:* no Save or Revert in the Library (edits
   save as you type; Undo replaces Revert). Screens & Looks keep Apply / Revert, because output
   changes should be deliberate during a service.
6. **Names in the Library.** The boards say "Libraries in Codex" and "Codex libraries" for what
   the server calls **collections** (your decision), and "folders" (Hymns, Christmas) for what the
   server calls **libraries** (shelves). They also put "Live streams" (IDN sources) in the
   collection, while your Codex decision keeps inputs per box. *Recommended:* say "Collection" in
   Oriel, show shelves as the folders under Songs, and keep live sources in the Show page's Video
   Inputs bin (per box), not in the Library.
7. **Who-is-editing.** The Library board shows "*X* is editing … Take over" and "2 others have this
   library open". The server has no presence or locks; PRODUCTS.md asks for conflict prompts that
   name who changed what. *Recommended:* first release uses conflict prompts only (as today);
   presence and locks later (G8).
8. **Colours.** The canvas shows Audience/Stage "on" in green and "off" in red, warnings in blue
   ("Needs attention", "Conflict", the timer warning), and amber for the stage timer warning.
   PRODUCTS.md keeps orange/red for "live" and "on air" only. *Recommended:* violet for
   selection, amber for live and for warnings, red only for errors and offline, green for "on /
   healthy", grey for "off", blue for information only.
9. **Screens features.** The canvas shows Mirror / Grouped / Edge Blend, Fill/Key, colour
   correction, corner pin, System and SDI outputs and Placeholders, which the server can't do
   yet, and leaves out things Oriel has today (embedded audio, IDN groups, frame rate per screen,
   screen on/off, duplicate, delete). *Recommended:* show only what works, keep today's working
   features in the new layout, and put the rest on the roadmap (G13).
10. **Stage layouts.** The canvas composes layouts from checkboxes; today's layouts are free-form
    designs in the editor, and the canvas drops stage-message presets. *Recommended:* both:
    checkboxes for the common cases, "Design freely…" for the rest; keep message presets.
11. **Site page details.** The canvas has per-library "Back up now" and "Last backup", a backup
    schedule per product, and an Updates tab; the server has one encrypted export of the whole
    Codex, fixed settings-backup timing (daily and after changes), and no update API yet. The
    canvas has no place for the site's access level (open / PIN / users), paired computers' keys,
    or Oriel computers waiting for approval. *Recommended:* one "Protect the Codex" card instead
    of per-library backups; schedule shown read-only; Updates hidden until the Launcher; a
    Security tab for access, users and paired computers; Oriel approvals listed under Join
    requests.
12. **Bible.** The canvas offers KJV, WEB and a licensed translation; PRODUCTS.md names KJV, ASV,
    WEB and BSB. Where do generated passages live, and can "Go live" skip the playlist?
    *Recommended:* bundle KJV, ASV, WEB and BSB; each generated passage becomes a presentation in
    a "Scripture" shelf of the active collection; "Go live" creates it and shows it at once
    without adding it to the playlist.
13. **Two shortcut choices** (section 3.2, the cleaned-up table: no table existed yet, so this is a
    proposal). F4 / F5 / F6 keep today's order (clear audio / props / messages) rather than the
    canvas's help list (props / messages / audio), so operators' habits don't change; and Ctrl+S,
    with Save gone, stays "back to Show" as today. *Recommended:* yes to both, and the rest of the
    table as proposed.
