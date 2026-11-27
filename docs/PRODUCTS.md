# Vesper product family

Each product has a description, a feature list, UI requirements and required functionality
(the minimum for a first release). Items marked *(suggested)* haven't been confirmed yet.
`ARCHITECTURE.md` has the technical design and milestones.

## Shared across all products

**Architecture**
- **Backends do the work, UIs are remote controls.** Every product has a backend service with
  IDN built in. UIs talk to it only through its API (HTTP + WebSocket).
- **One identity per machine.** One Vesper backend process per machine, with modules
  (presentation, Site Manager, Codex, switcher, mixer, display, intercom) switched on per box.
- **Auto-join a default site.** The first Vesper server on a network creates the site; other
  Vesper boxes discover it and ask to join, and an operator approves them (or approves during a
  setup window).
- **LAN-only.** Nothing depends on the internet.

**UI requirements (all products)**
- One design language: Oriel's dark, dense, ProPresenter-style look.
  - Same palette, typography and icon set (our own icons).
  - Orange/red accent only for "live" or "on air".
- Every UI runs as a **desktop app** (Tauri: Windows, macOS, Linux) and in a **browser** served by
  its backend, which is useful on tablets and phones.
- **Connection state is always visible.** A red "not connected, reconnecting" banner, automatic
  recovery, and nothing lost while offline.
- **Pairing:** the same machine is trusted; other devices pair once with a code shown by the
  backend.
- **Discovery:** the desktop app finds backends automatically; a browser opens the backend's
  address.
- **Multi-operator:** live state sync, conflict prompts that name who changed what, undo/redo
  where editing exists.
- **Keyboard:** full keyboard operation, ProPresenter-compatible shortcuts where they exist, and
  a "?" overlay listing shortcuts.
- **Accessibility:** labelled controls, visible focus, contrast suitable for a dark booth, and
  usable at 1366×768 and up.
- **Names:** never "ProPresenter"; network video is described as "IDN Open Protocol (compatible
  with NDI®)".

---

## Vesper (server)

The heart of the system: a headless server (Linux first, Windows supported) that runs the show. It renders slides, lyrics,
lower thirds, stage screens, timers, messages, props and video backgrounds on the GPU, mixes
program audio, and sends everything out over IDN. It also hosts site-wide modules: the
**Site Manager** (site identity, enrollment, management, monitoring, updates) and **Codex**
(content collections and settings backups).

**Features**
- **Shows and content:**
  - ProPresenter 7 library import
  - presentations with sections, arrangements, notes and song info
  - playlists with headers and Continuous Playlist
  - linked themes: songs follow their theme (editing the theme restyles every song that uses
    it), slides can override single properties and "Reset to theme"; imported songs stay copies
    until linked "keeping the look" (an import option or a song action: nothing on screen
    changes, later theme edits restyle them); template slides and their roles edited in the
    slide editor
- **GPU rendering:**
  - text with outlines, shadows, line masks, fit modes and per-word styling
  - shapes, images, video fills
  - layers (background, slide, announcements, props, messages, video input, audio)
  - looks with per-screen layer routing and template restyle
  - dissolve and cut transitions
- **Media and audio** (via `idn-ffmpeg` / `idn-audio`):
  - hardware video playback with loops, in/out, fades and retrigger rules
  - frame-accurate pacing and lip sync
  - audio layer and audio playlists
  - levels and meters
- **Live production:**
  - timers on slides, messages with fields, props
  - slide actions; stage display with stage messages
  - **captions layer** (`idn-captions`): live captions from the program audio, a live source or
    another captioner, per-look routing, stage screens, an IDN text source (`docs/CAPTIONS-PLAN.md`)
  - camera inputs (M4.3), announcements and macros (M4.3)
- **Outputs:**
  - IDN outputs with alpha, embedded audio, delay and overrides
  - screen configuration with live apply, test patterns, identify
  - streaming and recording (IDN gateways; done in batch 13, `docs/RECORD-STREAM-PLAN.md`)
  - OMT outputs and inputs (`idn-omt`; done in batch 13, `docs/OMT-PLAN.md`)
- **Modules:**
  - Site Manager
  - Codex
- **Connectors (M6):** Planning Center Services, ChurchCRM, Rock RMS, CCLI reporting, SongSelect
  import, parent pager.

**UI requirements**
- No UI of its own beyond a minimal local status page (health, version, pairing code, logs). All
  operation is through Oriel and the API.
- Logs and diagnostics must be readable remotely through Oriel.

**Required functionality (first release)**
- Runs unattended as a systemd service on Ubuntu LTS, starts at boot, recovers from crashes.
- Imports a ProPresenter library and plays a typical service end to end: lyrics with backgrounds,
  media, announcements, timers, stage display.
- At least 4 IDN outputs (audience, lower-third key, stage, spare) at 1080p30 with steady pacing.
- Lip sync within one frame on embedded audio.
- Never blocks live output on thumbnails, decoding, editing or network clients.
- Safe persistence (atomic writes, schema migration), plus backup and restore of the data and
  site identity.
- API coverage for everything Oriel does, documented for Stream Deck/Companion use; a Bitfocus
  Companion module (`integrations/companion/`) and Remote API compatibility for existing
  Stream Deck / Companion button profiles (off by default; `docs/CONTROL-SURFACES-PLAN.md`).

---

## Vesper Oriel (control app)

The operator's control surface. Volunteers build services, run slides, edit content, trigger
media and timers, configure screens and watch live previews. It is purely a remote control for
Vesper, and it also carries the site management views. Its layout follows the owner's design
canvas (redesign plan: `docs/ORIEL-REDESIGN-PLAN.md`): one compact header over full-screen
pages, a dark violet-accented palette and Inter everywhere.

**Features**
- **Show page:** sources and the playlist tree on the left; the playlist as one scrolling run of
  sections with LIVE / NEXT / DISABLED slides; on the right the output buttons, the live preview,
  Clear All and the coloured layer buttons, transport, and compact Timers / Messages / Stage
  tabs; the bottom bin with Media, Audio, Props and Macros.
- **Playlist building:** drag and drop, headers, context menus, undo/redo.
- **Library page (Codex):** collections, songs by shelf with the A–Z rail, media, themes (new,
  duplicate, rename, delete with used-by, edited in the slide editor; a song's theme and "Reset to
  theme") and props,
  search, uploads with resume, missing media and relink, storage, the item and collection
  trashes; songs viewed and edited in place.
- **Slide editor and Reflow** as full pages (typed lyrics, slide breaks, groups, live thumbnails).
- **Timers, Stage display, Screens & Looks** as full pages.
- **Site page:** devices (health, problems, version; identify, restart, logs, remove), join
  requests and Oriel computers waiting for approval, the site code and join mode, the Codex
  (storage, a manual encrypted export, nightly exports, importing an export), settings backups
  and restores (queued ones for show boxes), and Security (access level, unlock, users, paired
  computers, audit, this server's membership of another site). Updates come with the Launcher.
- **Settings page:** General (Oriel's own preferences, backed up to the Codex a few seconds after
  they change, with "Restore my Oriel settings"), Audio, Network & IDN, Keyboard, About.
- **Bible:**
  - **Bundled open translations** (KJV, ASV, WEB, BSB and other public-domain/open texts) stored
    in Codex, so it always works offline.
  - **Optional online providers through official APIs** (API.Bible, ESV API), with each church's
    own key and caching within each provider's terms.
  - Copyright/attribution lines added automatically.
  - No scraping of websites (e.g. BibleGateway).

**UI requirements**
- **Header (every page):** the Oriel menu (File, Edit, View, Import, Export, Settings, Help; no
  Open or Save: the server saves every edit), Show / Edit / Reflow, the page buttons Bible ·
  Library · Stage · Timers · Screens · Site, the Look picker, "?" and the connection pill.
  Every page but Show has a toolbar row with its name, its controls and Done or Close.
- **Slides:**
  - The live slide has an amber outline and a LIVE badge.
  - The next slide has a subtle outline.
  - Disabled slides are dimmed.
  - Group colour bars, thumbnails from the server.
- **Keyboard:** one table (the redesign plan's 3.2) feeds the key dispatcher, the menus, the "?"
  list and Settings ▸ Keyboard: Space / arrows / Page Up / Page Down step slides, F1–F6 clear,
  Ctrl+1 / Ctrl+2 the Audience / Stage screens, Ctrl+E / Ctrl+R the editors (Ctrl+S back to
  Show), Ctrl+L / B / Alt+T / Alt+S / Alt+M the pages, Ctrl+, Settings, Ctrl+Z / Ctrl+Y undo /
  redo. Keys are matched by their place on the keyboard.
- **Colours:** violet for selection, amber for live and warnings, red only for errors and
  offline, green for "on / healthy", grey for "off", blue for information.
- **Connection:** a red banner while offline (commands are never queued), a green one once back;
  on a show box, uploads, backups and exports go to the Codex box over a side connection after
  one question.
- **Admin actions** refused by the site's access level ask for the site's PIN or a sign-in.
- **Notices:** summarised, never flooding the screen.
- **Remote/tablet layout:** a simplified "stage manager" view (playlist, slides, next/previous,
  clear) for phones and tablets *(suggested)*.

**Required functionality (first release)**
- Run a full service from a playlist with keyboard and mouse: trigger, next/previous, clear,
  looks, media transport, timers, messages, props, stage messages.
- Build next Sunday's playlist and create or edit songs (Reflow), with undo.
- Configure screens and outputs on a headless server, end to end.
- Two operators at once without data loss.
- Windows installer, plus a browser fallback.

---

## Vesper Launcher

A small app on every machine that installs, launches, updates and monitors the Vesper products
there, with IDN's command-line tools bundled for setup and troubleshooting.

**Features**
- Install, update, remove and start/stop Vesper products; choose the machine's role (presentation
  server, display box, mixer, switcher, intercom station).
- Signed updates from the site's update channel; rollback to the previous version.
- Health: which apps run, their versions, CPU/GPU/disk, restart on crash.
- First-run setup: join the default site (shows the join request status).
- Bundled `idn-cli` tools: `idn doctor`, `find`, `recv`, `ptz`, `codec`.

**UI requirements**
- **Tray icon:** status colour (ok / updating / attention), quick launch, open window.
- **Small window:** a card per product showing state (running, stopped, update available) with
  Launch, Stop, Update and Logs buttons.
- **First-run wizard:** choose role(s), join the site, wait for approval, done. Three steps,
  volunteer-friendly.
- **Diagnostics page:** runs `idn doctor` and shows results in plain language, with "copy report"
  for support.
- Headless mode for servers: the same functions over the API, shown in Oriel's management views.

**Required functionality (first release)**
- Fresh install to a running, site-joined product in under 10 minutes for a volunteer.
- Safe, signed updates with automatic rollback if the new version fails to start.
- Works on Windows and Linux (control PCs, servers, display boxes).

---

## Vesper Codex (library)

The church's content and settings store. It hosts the content for Vesper/Oriel and signage as
**collections** (per campus, ministry or room) and stores the settings backups for every Vesper
product. There is **one Codex per site**: a module of the Vesper server on the site's server (the
Site Manager box), which other buildings reach over the church VPN. There are no replicas; show
boxes keep a local cache of what they have scheduled, and machines move files over IDN's file
transfer.

**Features**
- **Collections** (per campus, ministry or room). Each holds presentations, libraries (the
  sidebar shelves of presentations, as in ProPresenter), playlists, themes, media, media bins,
  global groups and the documents slide actions refer to (timers, messages, props, macros, clear
  groups, stage layouts); signage content (Mosaic) later. Screens, looks, show settings, masks and
  inputs stay with each box.
- **Show boxes cache what is scheduled:** media used by the box's playlists and bins is fetched
  ahead of time, anything else on demand, with a size cap. Live output always plays from local
  disk, never across the network.
- **Write-through edits:** a show box forwards content edits to the Codex; while the Codex can't
  be reached, edits are refused and the show keeps playing.
- Settings backup and restore for Vesper, Oriel, Mosaic, Cantor, Transept and Herald (sync of
  chosen settings later).
- **Protection without replicas:** a scheduled, encrypted export of the Codex to a folder the
  admin chooses (USB drive, NAS mount), with retention, plus a manual encrypted download.
- Media management: uploads, relinking missing media, deduplication, thumbnails, storage usage.
- Trash and a change log; per-document version history later *(suggested)*.
- SongSelect import and CCLI data with songs (M6).

**UI requirements** (inside Oriel)
- **Collections view:** list of collections with size, the boxes that use each, last change and
  status. Create, rename, assign to a box.
- **Library browser:** search across songs, presentations and media with filters and thumbnails;
  import (files, ProPresenter backups, SongSelect).
- **Backups view:** per product and per machine, with last backup, schedule, restore (with
  preview of what changes) and export to a file.
- **Sync status** per show box: clear indicators (in sync, syncing, offline, error) and a sync
  log.

**Required functionality (first release)**
- One Codex per site holding at least one collection that Vesper and Oriel use instead of a
  local data folder; a standalone server migrates its data folder into its own Codex.
- Show boxes cache their scheduled media and keep playing when the Codex is unreachable.
- Automatic settings backups for Vesper and Oriel, with one-click restore to a replacement box.
- Scheduled encrypted export of the Codex.
- LAN-only, with no internet dependency. Across buildings the church VPN must be routed without
  NAT, with the Site Manager's discovery port and Vesper's file-channel port open.

---

## Vesper Mosaic (displays and signage)

The display side of Vesper: small boxes behind projectors, TVs, confidence monitors and lobby
screens, from a single projector up to video walls, plus the church's digital signage, all
configured centrally in Oriel. Technical plan: `docs/MOSAIC-PLAN.md`.

**Decided (owner, 2026-11-24):**
- One product handles both a **single display** (one projector or TV) and **displays linked into
  a video wall**.
- **Narthex is folded in:** there is no separate signage or display software.
  - Signage is authored in Oriel.
  - It is rendered on a Vesper server as **channels**.
  - Mosaic displays show those channels.
  - The name "Narthex" may stay as the name of the signage feature.

**Decided (owner, 2026-11-25, option A):** a Mosaic box runs the **IDN Node with IDN's display
service** (`idn-display --service`, the Node's `apps.display`), not a Vesper server. **Vesper's
site server manages the boxes** (it finds them among the site's devices and drives each box's
display service with a token it issues for that box), and **Oriel is the UI**. Vesper does not
render on a Mosaic box. The Launcher's role for such a machine is "display box (Mosaic)": the
IDN Node with its display service switched on.
- **Displays connected to a Vesper server are Vesper's** (its own display outputs: KMS or
  windowed full screen, frame-accurate, with layouts, Fill/Key and corner pin), never the
  display service's: two programs on one screen fight. Mosaic manages screens only on machines
  that don't run a Vesper server; a Vesper box whose Node also has its display service on is
  flagged as a conflict. The Displays view shows the server's own outputs read-only, with a
  link to Configure Screens, so the room's whole picture is in one place.

### Displays

**Modes**
- **Single display.** One box behind one projector or TV shows one source or channel full screen.
- **Video wall, one box.** One box with several outputs: each screen is a tile of the wall (its
  crop of the picture, with bezels), set through the display service's wall tile settings.
- **Video wall, linked boxes.** Several boxes, one display each, are linked into one wall:
  - each box shows its own crop of the same source, with bezel compensation (the same tile
    settings);
  - the tiles of a wall stay in step through IDN's frame sync (the site's network clock and the
    source's timestamps, within one refresh of each other; one sync group, one feed);
  - the wall designer treats it the same as a one-box wall: a wall is a set of screens, each a
    tile.

**Features**
- Display endpoints on mini PCs or ARM boards running the IDN Node. They join the site, then
  appear in Oriel. **Setup requirement:** IDN's display service needs a desktop session (it
  doesn't run headless or on bare KMS): give the box a minimal desktop with auto-login to a light
  compositor.
- Central source assignment and live switching; name and group screens.
- **Channels:** site-wide named channels ("Main", "Lobby"), each pointing at a source; a screen
  follows a channel, and changing the channel's source moves every screen following it.
- Identify, test patterns, health monitoring (online, what
  each screen shows, the box's own health in Site ▸ Devices).
- Fallback when the feed goes: the last picture, then the site's logo after a few seconds (the
  display service's default), or black; each box keeps its feeds and settings, so it keeps
  showing while the site's server is down.
- Display power and scheduling via HDMI-CEC where supported *(suggested)*.

**UI requirements** (inside Oriel; the box itself has no UI)
- **Displays view** (Site ▸ Displays): a grid of all displays with a live thumbnail, name,
  source/channel and status; click to assign a source or channel; per-screen settings.
- **Channels list:** create, rename, set the source, delete.
- **Wall designer:** drag displays into a grid (outputs of one box or separate boxes alike), set
  bezels, preview the split.
- **On-screen setup:** the IDN Node's own (join pending, the box's name) and the display
  service's identify overlay. The box itself has no keyboard workflow.
- Bulk actions: identify all, test pattern all, power on/off by group.

### Signage (formerly Vesper Narthex)

Wallboard and digital signage for lobbies and hallways: scheduled announcements, events, video
loops and live data on unattended screens.

**How it fits together**
- **Authoring in Oriel:** a Signage page with the schedule, zone layouts, playlists and data feeds.
  Content lives in Codex collections like everything else.
- **Rendering on a Vesper server as channels:** for example a "Lobby" channel, with its schedule,
  zones and live data. This uses the existing renderer, media playback and feeds, and one channel
  feeds many screens.
- **Showing on Mosaic displays:** they follow the channel, as for any source.
  - Each box keeps showing its fallback (the last picture, then the site's logo) while the
    server is down; a cached schedule still on the box waits for the display service's
    fallback content (asked of the IDN maintainers).

**Features**
- Schedules by day, time and date range, with priorities and override slots ("emergency
  message").
- Content: slides, images, video loops, announcements, events.
- Live data from IDN feeds (Google Sheets, CSV/TSV, JSON, XML/RSS); clocks, countdowns to the
  next service, weather *(suggested)*.
- Zones and layouts per channel (main area, ticker, sidebar).
- Output to Mosaic displays or any IDN receiver; one channel to many screens, or a channel per
  screen.
- Event and announcement feeds from Planning Center, ChurchCRM or Rock (M6).
- Unattended for weeks, with automatic recovery.

**UI requirements** (in Oriel)
- **Schedule editor:** a calendar/timeline (week view) of what plays on which channel and when,
  with drag to create slots, recurring rules, and a "now playing" marker.
- **Layout designer:** a zone editor (reuses Oriel's canvas components) with feed bindings.
- **Playlist editor:** like Oriel's playlists, with durations and transitions per item.
- **Channels view:** every signage channel and the screens following it, with a live thumbnail,
  current item, next item and health.
- **Override button:** push an urgent message to all or chosen channels or screens immediately.
- **Feeds view:** add a feed URL or file, preview rows, map columns to fields.

### Required functionality (first release)
- A box boots, joins the site and is assigned a source or channel from Oriel within minutes.
- Stable full-screen playback for weeks, with reconnect after a network blip.
- A single display, and a wall of at least two linked boxes showing one picture in step.
- Channels and fallback content.
- A weekly signage schedule of slides and videos on at least 3 screens, unattended for 2 weeks.
- One data feed (Google Sheet or CSV) driving an events slide.
- Instant override message.

---

## Vesper Transept (video switcher)

A live production switcher for services and livestreams: up to 16 sources, program/preview,
transitions, keyed overlays (Vesper lyrics and lower thirds fed in directly), multiview, tally,
PTZ, captions, streaming and recording. Product UI and church workflow on IDN's `idn-switcher`.

**Features**
- Up to 16 inputs (IDN/NDI/OMT cameras, computers, colours), with Vesper fed in-process.
- Program/preview; cut, mix, wipes, fade through black, T-bar, fade to black.
- 4 keyed overlays (lyrics, lower thirds, PiP with crop and border).
- Multiview, tally, PTZ control and presets.
- Live captions on program.
- Streaming (RTMP/SRT) and recording, including 10-bit; stream inputs as site sources.
- Cues that recall Cantor scenes; titles from data feeds; HDR program.
- 1080p30 up to 4K60, sharing the GPU device with Vesper on the same box.

**UI requirements**
- **Main switcher view:**
  - multiview wall on top (program, preview and all inputs, with tally borders)
  - classic M/E row below: program bus, preview bus, CUT and AUTO buttons, a transition picker
    with duration, and an on-screen T-bar
- **Overlay (DSK) section:** 4 keyers, each with source, on-air button, auto-mix and live state.
  Vesper lyrics and lower thirds appear as ready-made sources.
- **PTZ panel:** joystick or pad, zoom and focus, preset buttons with thumbnails.
- **Stream/record panel:**
  - destinations (YouTube key or URL), Go Live / Stop
  - health (bitrate, dropped frames)
  - recording with disk space
- **Captions toggle** with a live caption preview.
- **Cues list:** a service rundown where each cue sets program, overlays and the Cantor scene.
- **Keyboard and control surfaces:** number keys for preview, Space for CUT, Enter for AUTO;
  Stream Deck and MIDI support *(suggested)*.
- **Volunteer mode:** a simplified layout (preview/program, 4–6 big source buttons, CUT/AUTO,
  lyrics on/off) *(suggested)*.

**Required functionality (first release)**
- 4–8 IDN camera inputs plus Vesper lyrics keyed over program.
- Cut and dissolve, multiview and tally.
- Stream to YouTube and record locally at 1080p30, reliably for a 2-hour service.

---

## Vesper Cantor (audio console)

A full mixing desk for front-of-house and monitors: channel strips, aux buses for in-ear and
wedge mixes, matrix and FX buses, scenes, noise suppression, MIDI surfaces, CLAP/VST3 plugins,
DAW and ASIO integration. Product UI on IDN's `idn-mixer` family.

**Features**
- Channel strips: trim, polarity, high-pass, gate, 4-band EQ, compressor, mute, fader, pan, sends.
- Buses: main, 16 aux, 8 matrix, 8 FX, monitor with solo (PFL).
- Scenes, recallable from Transept cues.
- Noise suppression (`idn-dsp`); CLAP/VST3 plugins (`idn-plugin-host`); MIDI control surfaces.
- DAW send/return (`idn-daw`), Windows ASIO driver (`idn-asio`); sound cards and stage boxes
  via `idn-audio`.
- Vesper program audio fed in directly; buses routed onward to Transept, OBS and stage monitors.
- 32–256-sample blocks, lossless audio, adaptive latency, click-free changes; routing matrix.

**UI requirements**
- **Console view:** a horizontal scroll of channel strips, each with meter, gain-reduction meter,
  fader, pan, mute, solo and name/colour. Bus masters on the right.
- **Channel detail:** a full-width editor for the selected channel (trim, HPF, gate, EQ curve
  with draggable bands, compressor graph, sends, plugin slots).
- **Sends-on-faders:** flip the faders to show sends to a chosen aux, for building monitor mixes.
- **Routing matrix:** a grid of sources to channels and buses to destinations, with meters.
- **Scenes:** list, recall, save, with safe-recall options per channel.
- **Personal monitor mixes:** a phone/tablet view where each musician controls only their own
  aux mix *(suggested)*.
- **Touch-friendly:** large fader targets for tablets, and keyboard nudging for precision.

**Required functionality (first release)**
- 32 inputs, a main mix plus 8 aux monitor mixes, with EQ and dynamics, at under 5 ms latency.
- Scenes, plus Vesper audio in-process.
- Bus outputs to sound cards and IDN; personal monitor mixes on phones *(suggested)*.

---

## Vesper Herald (intercom)

The production intercom for the tech and worship team: push-to-talk channels, calls,
messages and recordings, talkback into in-ear mixes. Built on IDN Voice.

**Features**
- Push-to-talk channels (director, camera, audio, stage); point-to-point calls; messages;
  recordings.
- Desktop app with talk keys and foot pedals; web client on any device.
- Talkback into in-ear mixes through Cantor (IEM feeds); talk sources.
- Control over MQTT; integration with Transept tally and cues *(suggested)*.
- Site-encrypted, LAN-only.

**UI requirements**
- **Station view:** one big button per channel (talk / listen, with latching or momentary
  modes), per-channel volume, and a clear indicator of who is talking right now.
- **Call list:** positions with online status; tap to call.
- **Messages:** short text messages and canned cues ("Stand by", "Go", "Wrap up").
- **Hardware:** talk keys and foot pedals mapped in settings; works with the window in the
  background.
- **Admin (in Oriel):** define channels, assign positions, route channels into Cantor IEM mixes,
  and recordings.

**Required functionality (first release)**
- 4 channels and up to 10 positions, with push-to-talk under 50 ms mouth-to-ear on the LAN.
- Desktop app and browser client; talkback into Cantor monitor mixes.

---

## Used as is
- **IDN OBS plugin:** IDN sources and outputs inside OBS Studio.
- **IDN virtual camera:** any IDN source as a webcam in Zoom or Teams on Windows.
