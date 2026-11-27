# Vesper Oriel (control app)

The operator UI. It is being redesigned after the owner's canvas
(`docs/ORIEL-REDESIGN-PLAN.md`): round 1 brought the new look (tokens, Inter, shared
components), round 2 the shell: one compact header (the Oriel menu, Show / Edit / Reflow, the page
buttons, the Look picker, "?" and the connection pill) over full-screen pages, and the later rounds rebuilt the pages one by one (Show, Library,
Reflow, Timers, Stage, Screens & Looks, Site, Settings; the slide editor and Bible have their own
rounds). It is a Svelte 5 + TypeScript web app that talks
to the server only through the public API (`crates/protocol/README.md`), wrapped in a Tauri v2 shell
for the desktop. The same build runs in a browser.

"Vesper" is the product family: this control app is **Vesper Oriel** and the server it talks to
is **Vesper**. The names come from `src/lib/config.ts`: `APP_FULL_NAME` ("Vesper Oriel": About box,
browser tab, client name) and the short `APP_NAME` ("Oriel": window title bar, first menu).
`index.html` (`<title>`) and `src-tauri/tauri.conf.json` (`productName` "Vesper Oriel" for the
installers and Start menu, `identifier` `org.oriel.app`, window title "Oriel") repeat them for the
native shell.

## Layout

```
src/
  main.ts, App.svelte          entry; connect screen or main view
  app.css                      design tokens (CSS custom properties) and base styles
  assets/fonts/                Inter (variable woff2, bundled) and its OFL licence
  lib/config.ts                product name, default port, storage prefix
  lib/server/                  connection layer (plain TS, unit tested)
    rest.ts                    typed REST client over /api/v1
    codex.ts                   Codex REST client (status, collections, assignment, trash, media,
                               search, blobs, storage) and its error mapping (liveShow, 403, …)
    codexBox.ts                side connection to the Codex box on `409 codexElsewhere`
    site.ts                    site REST client (status, settings, unlock / sign-in, devices, join
                               requests and codes, pairings, API keys, users, audit, events, membership)
    mosaic.ts                  Vesper Mosaic REST client (display boxes, screens' feeds and settings,
                               identify, test pattern, channels)
    codexBackups.ts            the Codex's protection: settings backups and restores, Oriel's
                               preferences, export download, nightly exports, import
    socket.ts                  WebSocket: backoff reconnect, command/result promises, resync, ping,
                               4003 (key revoked), codexChanged / siteChanged
    state.ts                   ShowState store ordered by revision, fine-grained selectors
    preview.ts                 binary preview frame parser and blob-URL decoder
    connection.ts              ties them together: status, notices (merged by key), heartbeat
    storage.ts                 remembered server, pairing tokens and UI preferences (localStorage)
  lib/state/                   UI state (Svelte 5 runes), one module per area
    session.svelte.ts          the core: connection and pairing, server info, the first load and the
                               core's refetches, editor and undo, playlists, the open presentation,
                               looks, clear groups, global groups, renaming; `refetch` for the others
    media.svelte.ts            media bins and the selected bin, the audio panel and its actions
    bins.svelte.ts             the Library's media bins, media playlists and global groups (G17): the
                               bin or playlist shown and the actions (create, rename, reorder, delete,
                               add / remove media, drag from Images or Videos onto a bin)
    showDocs.svelte.ts         messages (and the themes their templates use), props, the prop actions
    timers.svelte.ts           the timer documents
    stage.svelte.ts            stage layouts, templates, message presets, the stage actions
    shell.svelte.ts            page and mode (URL hash), dialogs, popovers, prompts, app actions
    show.svelte.ts             the Show page's view: slide view and size, selections, bin, panel sizes
    library.svelte.ts          libraries, the selected one and its items, search; the Library
                               page's state (`libraryPage`: tree view, collections, trash, search)
    screens.svelte.ts          screen configuration list, previewed screen, the Screens page's guard
    screens.ts                 screen configuration logic (pure, unit tested)
    site.svelte.ts             capabilities, the box's Codex status (GET /codex); the Site page's
                               state (`sitePage`: lists, admin actions with the PIN / sign-in
                               prompt on 403, the side connection for backups and exports) and
                               Oriel's preferences backup (`orielPrefs`)
    mosaic.svelte.ts           Site ▸ Displays: the display boxes, their screens and channels, this
                               server's own displays (read-only), the actions
    events.ts                  one dispatcher: documentChanged / codexChanged / siteChanged
  lib/live.ts                  per-slice stores of the show state (live slide, cursor, layers, ...)
  lib/edit/                    content editing logic (plain TS, unit tested): editor, undo, the
                               Reflow queue and text ↔ ops; chords.ts: keys, transposition, capo,
                               Nashville numbers, chords that follow edited words (the server's
                               rules; the editor's chord lines, the song's key, stage chord views)
  lib/shell/                   the frame around the pages
    MainView                   header, banners, the current page, dialogs, toasts, key dispatch
    Header, OrielMenu,         the header; the two-pane Oriel menu (content in orielMenu.ts)
      LookPicker, ConnectionPill
    Banners, StatusBar, Toasts offline / reconnected banners; screens status bar; toasts (toasts.ts)
    ShortcutHelp, StatusPanel, the "?" dialog; server status (in the pill); live output dialog
      LivePanel
    shortcuts.ts               THE shortcut table (dispatcher, menu labels, "?" dialog)
    keyboard.ts                key matching (physical keys, AltGr, text fields, desktop-only keys)
    menu.ts                    menu model and keyboard navigation (incl. two panes)
    pages.ts                   the page registry: routes, hash, capabilities, Done / Close
  lib/pages/                   one folder per page
    show/                      ShowPage (columns, slide area, media bin, right column, status bar):
                               the left column (sources and the playlist tree over their contents),
                               the centre as one list of sections per playlist item (sections.ts,
                               unit tested; documents fetched per section as it scrolls near,
                               presCache), slide tiles with LIVE / NEXT / DISABLED and media cues
    editor/                    EditPage, the slide editor (EditView, inspector, canvas …)
    reflow/                    ReflowPage, ReflowEditor
    library/                   LibraryPage: collection card, tree, songs table with the A–Z rail,
                               media tiles, themes, props, Recently deleted, the item (logic.ts)
    timers/                    TimersPage: timer cards, system clock, the big readout, settings
    stage/                     StagePage: layout cards, stage screens, live picture, stage message,
                               "On this layout" checkboxes (compose.ts, unit tested), Design freely…
    screens/                   ScreensPage: Screens | Looks; ConfigureScreens (staging, Apply /
                               Revert, test patterns, output tabs), LooksView (list, Make Live)
    site/                      SitePage: Devices, Join requests, Displays (Mosaic: mosaic.ts), Codex (export, nightly exports,
                               import), Backups (restore previews, queued restores), Security;
                               the unlock / sign-in prompt; logic.ts (unit tested)
    bible/                     BiblePage (empty state until the scripture module, G11)
    settings/                  the Settings page (SettingsPage: General, Audio, Network & IDN,
                               Keyboard, About); prefsBackup.ts (what is backed up, the watcher
                               and its debounce, restore merge; unit tested)
    PageFrame.svelte           a page's toolbar row (name, controls, Done / Close) and content
  lib/ui/                      shared components (Button, Chip, Dialog, Menu, Tabs, …; see below)
  lib/components/              shared dialogs: connect screen, prompts, context menu, About,
                               Diagnostics, conflicts, the logo
scripts/shots.mjs              screenshots of every screen against the stub server
scripts/shots/<page>.mjs       each page's own screenshots (picked up by shots.mjs; dialogs.mjs: the
                               shared dialogs)
test/stub/                     stub server (REST fixtures, scripted WebSocket) and its fixture capture;
                               per-area fixtures in fixtures/<area>/, per-area routes in routes/<area>.mjs
src-tauri/                     Tauri v2 shell (mDNS discovery command); icons from icons/source.svg
```

## Shell, pages and routing

- **Header** (every page): the Oriel menu (logo button; File, Edit, View, Import, Export, Settings,
  Help; F10 opens it from the keyboard), Show / Edit / Reflow, the page buttons Bible · Library ·
  Stage · Timers · Screens · Site, the Look picker (`setLook`; "Every layer" is `null`), "?" and
  the connection pill (its popover: the server, the Codex role and collection, Reconnect, screens
  and diagnostics). There is no Open, Save or Save As: the server saves every edit. Menu items for
  later features are disabled with "coming later".
- **Routing**: `shell.page` (show, bible, library, stage, timers, screens, site, settings) plus `shell.mode`
  (show, edit, reflow), mirrored in the URL hash (`#/library`, `#/edit`), so browser Back works and
  a page can be bookmarked. `shell.go(route)` navigates; Done / Close (`shell.close()`) go back the
  way the operator came. Leaving the Screens page with unsaved changes asks first. Pages a server
  can't show are disabled with the reason (Library needs `codex`, Screens `screenConfig`, Site
  `site`). Esc closes menus and dialogs but never leaves a page.
- **Connection**: a red banner while the server can't be reached ("Slides and edits wait until the
  connection is back": commands are never queued), a green one for 3.5 s once it is back. A
  socket closed with 4003 (the key was revoked) goes back to the connect screen: "This computer was
  removed from the site. Pair again."
- **Status bar** (Show page): one dot per screen (name, role, off, output problems) and
  "Configure screens".
- **Toasts**: bottom centre, 2.6 s (errors stay until dismissed), one per notice key, at most
  three lines: older ones fold into "N more", which opens Diagnostics.

**Where the old menu bar and toolbar went:** Search → the Library page (Ctrl+F, Edit ▸ Find in
Library); Look → the header's Look picker; Live → View ▸ Live Output…; Status → the connection
pill; Media → View ▸ Media Bin (Ctrl+M); Audience / Stage → View ▸ Audience Screens / Stage Screens
(Ctrl+1 / Ctrl+2); Configure Screens → the Screens page (Ctrl+Alt+S, the status bar); the stage
screens' layouts and Edit Layouts → the Stage page; Window ▸ Diagnostics → Help ▸ Diagnostics and
the pill; Settings sections → the Settings pane; the right column's six icon tabs → compact Timers / Messages / Stage tabs, with Audio, Props and Macros in the bottom bin (round 6).

## Keyboard shortcuts

`lib/shell/shortcuts.ts` is the one table (the plan's 3.2): the key dispatcher, the menus'
accelerator labels and the "?" dialog all read it. Keys are matched by physical key
(`KeyboardEvent.code`) and exact modifiers, so layouts don't change them and AltGr (Ctrl+Alt on
Windows) never fires a Ctrl shortcut; a text field keeps its own keys (typing, its undo / redo,
Ctrl+Alt combinations). Desktop-only keys (Ctrl+T, Ctrl+= / −, Ctrl+N, Ctrl+Shift+N) are left to the
browser outside the desktop app.

## Parallel lanes: who owns what

After round 2 up to three agents work side by side (plan section 5), each in its own page folder
and its own state, shot and stub files:

| Lane | Owns |
|---|---|
| A (Library, Slide editor) | `lib/pages/library/`, `lib/pages/editor/`, `lib/state/library.svelte.ts`, new `lib/server/codex.ts`, `codexBox.ts`; `scripts/shots/{library,edit}.mjs`, `test/stub/fixtures/library/`, `test/stub/routes/library.mjs` |
| B (Show, Site, Settings) | `lib/pages/show/`, `lib/pages/site/`, `lib/pages/settings/`, `lib/state/show.svelte.ts`, `lib/state/media.svelte.ts`, `lib/state/showDocs.svelte.ts`, `lib/state/site.svelte.ts`, `lib/server/site.ts`, `lib/server/codexBackups.ts`; `scripts/shots/{show,site,settings}.mjs`, `test/stub/fixtures/{show,site}/`, `test/stub/routes/{show,site}.mjs` |
| C (Reflow, Timers, Stage, Screens, Bible) | `lib/pages/reflow/`, `lib/pages/timers/`, `lib/pages/stage/`, `lib/pages/screens/`, `lib/pages/bible/`, `lib/state/timers.svelte.ts`, `lib/state/stage.svelte.ts`, `lib/state/screens*.ts`; `scripts/shots/{reflow,timers,stage,screens,bible}.mjs`, `test/stub/fixtures/<page>/`, `test/stub/routes/<page>.mjs` |

**Read across lanes, changed by the owner only:** the editor (lane A) reads timers
(`timers.svelte.ts`, lane C), messages and props (`showDocs.svelte.ts`) and media bins
(`media.svelte.ts`, lane B); the Show page's messages (lane B) read timers (lane C). A lane that
needs another field or action there asks the owner, or adds it in a shared commit.

**Shared files** change only in a short, separately merged commit announced to the other lanes:
`lib/ui/`, `src/app.css`, `lib/shell/shortcuts.ts` (the shortcut table), `lib/shell/menu.ts` and
`lib/shell/orielMenu.ts` (the menu model), `lib/shell/pages.ts` (the page registry), the rest of
`lib/shell/`, `lib/state/session.svelte.ts` (the core), `lib/state/shell.svelte.ts`,
`lib/state/events.ts`, `lib/state/errors.ts`, `lib/state/prefs.ts`, `lib/server/` (existing files),
`scripts/shots.mjs`, `test/stub/server.mjs` and `capture.mjs`, and the captured fixtures
(`test/stub/fixtures/*.json`). A lane that needs a new shared piece adds it there in such a commit
first.

**How the area state modules hang off the core:** the core's `begin` calls each module's
`attach()` once per connection; it subscribes to `documentChanged` (`state/events.ts`) for its
own kinds and refetches through `session.refetch(kind, fn)` (a failure becomes one notice per
kind). The core's `refreshAll` loads each module's lists (`load()`; media gets the bin tree from
the core's first batch through `setBins`) and tells media when the playlist tree changed
(`playlistsChanged`, `pickAudioSource`). Props and stage layouts open in the Edit view through the
core's `showOneSlide`, and their changes reach the open one through `oneSlideChanged`. These calls
are the contract between the core and the modules: changing their names or meaning is a shared
commit; what happens inside them is the module owner's. A new document kind needs no core change:
its module's `attach()` handles it (add the `attach()` call to `begin` for a new module, in a
shared commit).

**Screenshots and stub data per page:** a page's screenshots go in `scripts/shots/<page>.mjs`
(`shots.mjs` runs every file there, from the Show page, and returns to it afterwards; its header
lists what the file gets). A page's own fixtures go in `test/stub/fixtures/<area>/*.json` (served
like the captured ones; a name used twice stops the stub), and its own routes (writes, generated
answers) in `test/stub/routes/<area>.mjs` (`server.mjs`'s header has the interface). Neither
needs a change to the shared script or server.

## Look: tokens and shared components

The look follows the owner's design canvas (`docs/ORIEL-REDESIGN-PLAN.md`, section 2.4).

- **Tokens** live in `src/app.css` as CSS custom properties: surfaces (`--surface-well` …
  `--surface-overlay`), lines, text (`--text`, `--text-2`, `--text-label`, `--text-meta`), the
  violet accent (`--accent`, `--accent-sel`, `--accent-ring`, `--accent-link`), live amber
  (`--live`, `--live-ink`), status colours, layer and group colours, radii (`--r` 6 px, `--r-card`
  8 px, `--r-dialog` 12 px), shadows, the type scale (13 px base, 11 px uppercase section labels)
  and spacing. Components use tokens, not hard-coded colours. The older names (`--panel`,
  `--bg-1`, …) are aliases kept until the pages that use them are rebuilt.
- **Status colours:** violet for selection, amber for live and warnings, red only for errors and
  offline, green for "on / healthy", grey for "off", blue for information only.
- **Inter** is bundled (`src/assets/fonts/`, from `@fontsource-variable/inter` 5.2.8, SIL Open
  Font License in `OFL.txt`). Nothing is fetched from the internet at runtime: no CDN, no web
  fonts.
- **`src/lib/ui/`**: Button (primary, secondary, outline, danger, live, ghost), NewButton (dashed
  "+ New …"), Chip (LIVE, NEXT, DISABLED, EDITED, TEST, IN SET), Badge, StatusDot, SectionLabel,
  Card, Switch, Segmented, Tabs, Field, Select, ColorWell, Slider, DataTable (sortable, sticky
  header), Menu and Popover, Dialog, Toast, Splitter (arrow keys, Shift for 50 px, double-click
  resets), EmptyState, PageToolbar, SlideTile. Icons are `lucide-svelte` at 16 px, stroke 1.8.
  Their keyboard and sorting logic is in `lib/ui/logic.ts` (unit tested).

## Screenshots and the stub server

`npm run shots [-- <folder>]` builds `dist/`, starts the stub server and takes screenshots of
every screen (connect, pairing, key revoked, the Show page, each pane of the Oriel menu, the Look
picker, the connection pill, "?", Edit, Reflow and every page, toasts, the offline and reconnected
banners) at 1920×1080 and 1366×768. Each page's own shots are in `scripts/shots/<page>.mjs`. PNGs go to the folder given, else `$ORIEL_SHOTS_DIR`, else
`<tmp>/oriel-shots`; they are never committed. The script is Linux-only for now: it drives the
headless Chromium that Playwright installed on the build machine (`ORIEL_CHROMIUM` points it at
another Chromium); `playwright-core` is pinned to the version that matches it. Set
`ORIEL_SHOTS_NO_BUILD=1` to skip the build, and `ORIEL_SHOTS_ONLY=library,site` to take only those pages' own shots.

The stub server (`test/stub/server.mjs`) serves `dist/` and answers `/api/v1` from JSON fixtures
in `test/stub/fixtures/` (and `fixtures/<area>/`, plus each `routes/<area>.mjs`), with generated SVG thumbnails, a scripted WebSocket (hello, state,
results, timer ticks, meters, preview frames) and three modes: `local`, `unpaired` and `down`. `routes/site.mjs` adds a synthetic two-box site (documentation addresses only): devices, a join request, a waiting Oriel pairing, settings backups and restores, exports and an import preview; `routes/mosaic.mjs` adds its display boxes, screens and channels (its header lists the stub controls, e.g. access level `pin` with the PIN 1234); `routes/slidesImport.mjs` a PowerPoint import job that moves on one step per poll and ends with a synthetic report
(`stub.revoke()` closes the sockets with 4003).
Run it on its own with `node test/stub/server.mjs --port 5599` and open `http://127.0.0.1:5599`.
The fixtures are **synthetic only** (made-up titles, lyrics, names and hosts). They were captured
from a real server with `test/stub/capture.mjs` (its header says how); never use a church's data
for them.

Protocol types are imported from the generated bindings (`$protocol` is an alias for
`crates/protocol/bindings`); nothing here redefines them.

## Develop in a browser

```sh
# 1. the server (from the repo root)
cargo build --release -p vesper-server
./target/release/vesper --data <data dir> --listen 127.0.0.1:5555

# 2. the UI
cd apps/control
npm ci
npm run dev                      # http://localhost:5173, proxies /api (HTTP + WebSocket)
VESPER_SERVER=http://10.0.0.5:5555 npm run dev  # proxy to another server
```

Requests from the server machine itself need no pairing. From another machine, the connect
screen asks for the 6-digit code that the server prints in its log, and for this computer's name
(default: the platform and browser, e.g. "Windows · Chrome"; remembered), so the site's admins
can tell pairing requests and keys apart (`Oriel – Windows · Chrome`).

| command | |
|---|---|
| `npm run check` | svelte-check, TypeScript strict, warnings fail |
| `npm test` | vitest: connection layer, notices, heartbeat, preview frames, revisions, the shortcut table and key matching, menus, routing, toasts, status bar, change events, undo stack, edit queue, Reflow text ↔ ops, the shared components' keys and sorting, the pages' logic (Show sections, Library, Stage, Site: device summaries, export form, import choices), the preferences backup's debounce and merge |
| `npm run build` | static files in `dist/` (relative paths, so the server can serve them from any URL) |
| `npm run preview` | serves `dist/` with the same `/api` proxy |
| `npm run shots` | screenshots of every screen against the stub server (see above) |

## Desktop app (Tauri)

`src-tauri/` is excluded from the root Cargo workspace because it needs the WebKitGTK system
libraries on Linux. To build it on Ubuntu 24.04:

```sh
sudo apt update
sudo apt install -y build-essential curl wget file pkg-config \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev libjavascriptcoregtk-4.1-dev \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev

cd apps/control
npm ci
npm run tauri dev       # runs `npm run dev` and opens the window
npm run tauri build     # release build plus .deb / .AppImage in src-tauri/target/release/bundle/
```

Windows needs the WebView2 runtime (preinstalled on Windows 10/11) and the MSVC build tools;
macOS needs the Xcode command line tools. The commands are the same.

Installers (Linux `.deb` and AppImage, the Windows NSIS installer cross-built on Linux):
`packaging/oriel/build.sh linux|windows`, see `docs/PACKAGING.md`.

The shell adds one command, `discover_servers`, which browses `_vesper._tcp` over mDNS (through
`protocol::discover`) so the connect screen can list servers on the LAN. In a browser the
connect screen shows only the address field.

**Desktop build and CORS:** the packaged app's pages come from `tauri://localhost`, so its REST
calls to `http://<server>:5555` are cross-origin; the server answers them with CORS headers on
`/api/v1`. `npm run tauri dev` goes through the Vite proxy.

**Served by the server:** `npm run build` writes `dist/`, and `vesper` serves it at
`http://<server>:5555/` when started from the repository root (or with `--ui-dir <dist>`), so
any browser on the LAN can open the control app without installing anything.

## Behaviour notes

- Show state is never edited locally: every change is a command, and the UI follows the
  server's `state` messages. Commands are not queued while offline, so a slide never fires
  late.
- After a reconnect the server's next `state` is accepted whatever its revision (a restarted
  server counts from zero), the documents are refetched, and preview subscriptions are sent
  again.
- When the socket has been quiet for a few seconds, a heartbeat sends a `ping` over the
  WebSocket; if no `pong` comes back within 3 s, the socket is dropped so the red offline banner
  shows at once.
- Server notices with a `key` replace each other (one toast per key), so a long list of missing
  files shows as one summary per presentation; the Diagnostics dialog keeps the full list.
- Panel sizes, the slide view (Grid / Text / List), the right-column tab and the media bin
  choice are remembered per browser (localStorage). View ▸ Reset Panel Sizes restores the
  defaults.
- Shortcuts: see "Keyboard shortcuts" above and the "?" dialog. Browser keys (reload, copy and
  paste, zoom, tabs) are left alone.
- **Editing** goes through the server's edit API (`crates/protocol/README.md`, "Editing
  content"). File ▸ New Presentation (Ctrl+N; browsers keep Ctrl+N for themselves, the desktop
  app gets it; also Import ▸ Lyrics from Text…) asks for a name, creates it in the selected library and opens it in Reflow. The
  left column's "+" buttons and right-click menus create, rename (inline), duplicate, delete
  (after a confirmation) and add to playlists. Drag a presentation onto a playlist to add it;
  hold it there and the playlist opens, so it can be dropped at a position (insertion line).
  Playlist items reorder by dragging; Esc cancels a drag. Headers have a name and a colour.
- **Undo / redo** (the Oriel menu's Edit pane, Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y) is per client: each entry sends
  the ops the server returned as `inverse` (or recreates deleted documents). Typing in one
  slide within 2 s is one step. In other text fields Ctrl+Z stays the field's own undo.
- **Reflow** (Ctrl+R, a full page with the presentation's name, slide and group counts, the
  save status, text and thumbnail size sliders and Done): one text block per slide in bands by
  group (vertical label, colour bar), in a fixed serif italic capitals style (not the
  presentation's font), with hotkey and media cue badges, thumbnails on the right. Enter is a new line;
  **Ctrl+Enter** (or Insert Slide Break) splits the slide at the caret; Backspace at the start
  of a slide joins it to the one above; Tab / Shift+Tab move between slides. Each slide's "…"
  menu (the menu key or Shift+F10 in its text) starts a group there (the presentation's groups,
  the server's global groups, or a new one), continues the group above (the group's slides move
  into the one above), breaks the slide at the caret, merges it up, removes its media cue or
  deletes it (`pages/reflow/slideMenu.ts`). Typing is saved 400 ms after the last key as `setSlideText` with `expectText`; the
  status shows Saved / Saving… / Offline. Edits made offline wait in order and are sent when
  the server is back. If someone else changed the slide meanwhile, the editor shows both
  texts: Keep mine (overwrite) or Use theirs. Other operators' edits appear without a reload
  (`documentChanged`).
- **Site page** (Ctrl+Alt+M): tabs Devices, Join requests (badge: machines and Oriel computers
  waiting), Codex, Backups and Security; Updates stay hidden until the Launcher's API exists. The
  toolbar shows the site code with its countdown and New code. Lists refetch on `siteChanged` /
  `codexChanged`; "Recently" takes each `siteEvent` as it comes, and everything is read again
  after a reconnect. Admin actions refused with 403 ask for the site's PIN (access `pin`) or a
  sign-in (`users`) and run again. On a show box, backups and exports go to the Codex box over the
  side connection (asked once; never from the background). Export downloads go through the
  browser's (or WebView's) download.
- **Settings page** (Ctrl+,; the Oriel menu's Settings pane; `#/settings`): a page like the
  others (registered in `pages.ts`, not a header button: the Oriel menu's button is lit while it
  is open) with General, Audio, Network & IDN, Keyboard (read-only, from the shortcut table),
  Bible (disabled until G11) and About; Close (or browser Back) goes back, Esc stays.
- **Oriel's preferences backup:** the preferences in `prefsBackup.ts`'s list (panel sizes, views,
  bins, presets) are compared every 2 s and saved to `PUT /codex/backups/oriel` 4 s after the last
  change (and once per connection; the server answers "unchanged"). Settings ▸ General offers
  "Restore my Oriel settings" (this computer's backup) and taking over another Oriel computer's
  (`GET /codex/backups/oriel?set=`: any paired client, no admin needed); restoring writes them
  and reloads Oriel.
- After a mouse click, buttons give focus back to the page, so Space always means "next"
  rather than pressing the last clicked slide or clear button again.
