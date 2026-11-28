# Architecture (draft)

**Vesper** is the product family. The headless render/playout server is **Vesper** (crate `vesper-server`, binary `vesper`) and the control app is **Vesper Oriel** (short "Oriel" in its window and menus). Future products join the family as "Vesper <name>". Don't use "ProPresenter" in product names.

## Goals

- **Split server and control UI.** A headless render/playout server, plus one or more control apps connected over the network. Both can also run on the same machine.
- **Server on Linux first, Windows supported** (decided 2026-11-18). Linux (Ubuntu LTS, x86_64) is the primary target: VA-API hardware video, `apt`-managed dependencies, systemd. A Windows server must also work for churches where Linux isn't an option (wgpu supports D3D12). Either way it must run on a box with no keyboard, mouse or monitor attached. Every UI runs on Windows and Linux.
- **Multiple outputs.** Each output can go to the network (IDN Open Protocol, compatible with NDI®), a physically attached display, or both.
- **Network video in and out.** Network sources can be used as layers. Every screen can be sent over the network. The one network video transport is IDN Open Protocol, an open implementation compatible with NDI®; no proprietary runtime is used.
- **Familiar workflow.** The control UI follows ProPresenter's layout and workflow closely: library, playlist, slide grid, preview/live, layers, looks, stage display. Icons and artwork are our own.
- **Easy to hand out.** Churches get an installer, a data folder that's simple to back up, and nothing to configure by hand.
- **Imports ProPresenter 7 libraries** so existing churches can switch.

## Components

```
┌────────────────────────┐        HTTP + WebSocket         ┌──────────────────────────────┐
│ Vesper Oriel (Tauri)   │ ◄──────────────────────────────► │ Vesper (Rust daemon)         │
│  Win / Mac / Linux     │        mDNS discovery            │  library + show state        │
│  ProPresenter-style UI │                                  │  render engine (wgpu)        │
└────────────────────────┘                                  │  outputs: IDN / display      │
┌────────────────────────┐                                  │  inputs:  IDN / media files  │
│ browser (tablet/phone) │ ◄──── same API, served by ─────► │  API (axum)                  │
│ remote / stage view    │       the server itself          └──────────────────────────────┘
└────────────────────────┘
```

Rust workspace crates:

| Crate        | Role |
|--------------|------|
| `model`      | Our own data model (presentations, slides, playlists, themes, looks, screens). Serde types shared by everything. Linked themes (`theme_link`): slides that follow a theme's template slide with per-property overrides, resolved by the server whenever the theme changes (`docs/THEMES-PLAN.md`). |
| `protocol`   | API request/response/event types. TypeScript types are generated from these for the UI. |
| `vesper-server` | Vesper, the daemon (binary `vesper`): show state machine, render engine, output routing, API (`src/api/`, one module per area with its own routes), mDNS. Re-exports `vesper-codex` as `codex` (with `data`, `store`) and adds the server's side of it in `src/codex/`: the `/api/v1/codex` routes and write-through middleware, the `codex::Server`/`codex::Show` implementations and the content writes it plans, presence over the box-to-Codex link. |
| `vesper-codex` | Vesper Codex, the site's content store: loading the data folder (`data`) and atomic document writes (`store`), collections and the one serialized writer, change logs and trash, the media library (IDN's), uploads and the media worker, search, settings backups, encrypted exports, migrations, the box-to-Codex link, show boxes' mirrors, adopt, write-through. Reaches the server only through two traits: `codex::Server` (the server's state) and `codex::Show` (the show actor). |
| `vesper-site` | The server's site (re-exported by `vesper-server` as `site`, its `auth` as `auth`): its IDN Site Manager as a module, joins, devices, users, paired clients' keys, who a request comes from, the `/api/v1/site` routes. |
| `vesper-core` | What the server's crates share: API errors as HTTP responses, request/response helpers, document revisions (`ConfigError`, `check_revision`). |

The server's crates: `vesper-core` ← `vesper-site` ← `vesper-codex` ← `vesper-server`.
| `output`     | Frame sinks: IDN Open Protocol (compatible with NDI®; compiled in, see Licensing), PNG, ffmpeg pipe, OMT (`idn-omt`, compiled in); live IDN and OMT inputs. A verification receiver and the `idn_*` probe tools. |
| `pro-import` | Reads ProPresenter 7 files (`.pro`, playlists, `.probundle`) via protobuf (prost) and converts them into `model`. |
| `render`     | Headless wgpu renderer: layers, slides, still media and video frames (YUV → RGB on the GPU), per-layer transitions → premultiplied sRGB RGBA8 frames, with async readback per screen. |
| `media`      | Video and sound decoding through IDN's `idn-ffmpeg` `media` module (system FFmpeg, GPU first): one player per video over IDN's decode-ahead `VideoReader` with nearest-frame selection, media clocks, video thumbnails; sound read through IDN's `AudioReader` (48 kHz float) into lock-free rings. |
| `audio`      | The program audio bus: mixer (gain, equal-power fades, channel routing, soft clip, meters), one engine thread that drives the output (a sound card through IDN's `idn-audio`: ALSA over the system `libasound` on Linux, WASAPI on Windows; or a null output that keeps time), program-audio taps for network outputs. |
| `control`    | Tauri v2 app. Rust shell plus a web frontend (Svelte or Solid). Talks to the server **only** through the public API. |

The control app contains no show logic. Anything it can do, a browser, a Stream Deck/Companion module or a script can do through the same API.

### Moving the Codex into its own crate (done, 2026-11-24)

`vesper-server/src/codex/` was about 23 000 of the server's 54 000 lines. It moves into
`crates/vesper-codex` (package `vesper-codex`), so the graph becomes `vesper-core` ←
`vesper-site` ← `vesper-codex` ← `vesper-server`. No behaviour, wire or binding changes; the
server re-exports the crate as `codex` (and `data`, `store`), so paths such as
`vesper_server::codex::library` keep working. Every `crate::…` the Codex used, and what happens
to it:

| Server part the Codex used | Decision |
|---|---|
| `data` (collection content, `ShowData`, loading, `kind_dir`, `COLLECTION_KINDS`) | **Moves with the Codex.** It already depends on the Codex's media library and relink rules; `codex::load` builds `ShowData`. Re-exported as `crate::data`. |
| `store` (atomic document writes) | **Moves with the Codex** (it needs `data::kind_dir`; `config` still uses it for box documents). Re-exported as `crate::store`. |
| `config::{ConfigError, revision}` | Already `vesper_core::change`. `config::check_revision` **moves down into `vesper-core`** (re-exported by `config`). |
| `auth`, `site`, `lock` | Already `vesper-site` / `vesper-core`: the crate depends on them. |
| `Shared` (the server's state) | **Becomes a trait**, `vesper_codex::Server`: the Codex's parts the server holds (writer, uploads, library, link, mirror, search, backups, exports, adopt, membership), the documents (`data`, `codex`, `set_codex`, `collection_content`), the site, auth and events, and a few hooks below. `Shared` implements it. |
| `actor::ShowHandle` | **Becomes a trait**, `vesper_codex::Show` (swap in a collection update, `reloadData`), implemented by `ShowHandle`. `actor::CollectionUpdate` **moves with the Codex** (re-exported by `actor`). |
| `content` (content edit plans) | Stays. `content::ChangeEvent` **moves with the Codex** (`data`). The writer takes a finished plan; `Writer::content`/`docs` (which call `content::plan`/`showdocs::plan`) become a server-side extension trait, and restoring trashed content documents asks the server to plan their re-creation (`Server::recreate`). |
| `showdocs` (timers, messages, props, stage layouts) | Stays (as above). |
| `show::ShowState` (what is live, for the `liveShow` refusal) | Stays: `writer::live_items` moves to the server (`Server::live_items`). |
| `presence` | Stays, with `codex::site_presence` (presence and edit locks over the box-to-Codex link are presence's logic). The link and the mirror reach it through `Server::presence_serve` / `presence_pushed`; the message kinds move to the crate. |
| `api` (`AppState`, the route handlers, the request's caller) | **Stays as thin glue**: the `/api/v1/codex` route handlers (`codex/api.rs`, backups', exports' and adopt's routes) and the write-through middleware layer stay in the server. `api::Forwarded` **moves with the Codex**; the caller's forwarded collection is `Server::forwarded_collection`. |
| `thumbs::QUALITY` | **Moves with the Codex** (`blobs`, beside `THUMB_WIDTH`); `thumbs` re-exports it. |

Stages, each its own commit, each building and passing the tests: (1) in place, the Codex reaches
the server only through the traits above and the route handlers move out of its modules; (2) the
files move into the new crate (`git mv`); (3) documents. What stayed in the server is listed in
the table above (the `vesper-server` row): about 2 600 lines of glue in `src/codex/`.

## Server

### Show state
The server is authoritative. It holds what is live on each layer of each screen, along with timers, messages and the active look. Clients send commands, such as `trigger slide`, `clear layer` or `set look`. The server applies each command and broadcasts the resulting state change over WebSocket to every connected client. Any number of operators can be connected at once.

### Rendering
- A **screen** is a canvas with a fixed resolution and frame rate, for example Audience 1920×1080@30, Stage 1920×1080@30, or Lower Third key+fill.
- Each screen is composited from **layers**, bottom to top: video input, media, slide, announcements, props, messages. A **look** decides which layers appear on which screen.
- Compositing uses **wgpu**, offscreen and GPU-accelerated (Vulkan via Mesa RADV on the reference hardware). Text is laid out with `cosmic-text`. Glyph coverage and the vector text outline are rasterized with swash, then shadow blur and compositing run on the GPU. Blending is in linear light; output frames are premultiplied sRGB RGBA8 (see `crates/render`).
- Transitions (cut, dissolve and so on) run on the GPU per layer.
- **Baseline hardware:** a Ryzen H-series iGPU (e.g. Radeon 680M/780M class). Many boxes will have a dedicated GPU. Without a GPU, wgpu falls back to a software renderer (lavapipe/WARP), which is for CI and testing only.

### Outputs (per screen, many-to-many)
- **IDN Open Protocol sender (compatible with NDI®).** The only network video transport. Frames are read back from the GPU as premultiplied RGBA and handed to IDN Open Protocol, a separate, open-source (MIT/Apache-2.0) Rust library compiled into the server: there is no runtime to install. An output binding with transport `idn` (`{ "type": "idn", "sourceName", "groups" }`) appears to compatible receivers as `"HOST (sourceName)"`, with embedded program audio. Each screen can have several. Source names are unique among the server's IDN outputs. Details: `crates/output/README.md`; manual checks against third-party gear: `docs/INTEROP-CHECKLIST.md`. NDI® is a registered trademark of Vizrt NDI AB; neither this project nor the library is affiliated with or endorsed by Vizrt.
  - **Capacity today:** the library encodes in software, about **1.8 CPU cores per 1080p30 stream**. Plan on about **2 outputs per mini PC** until the library's encoder improvements land; measure with `cargo run --release -p output --example idn_soak`.
  - **Migration (schema 2):** bindings stored with the official-runtime transport `ndi` (schema 1, as older importers wrote them) are migrated to `idn` with the same source name and groups when the server loads them; see `model::migrate`. The file on disk is rewritten only when an operator edits that output.
- **Physical display.** A screen shown full screen on a monitor or projector attached to the box (`{ "type": "display", "selector", "mode" }`; the box's displays: `GET /api/v1/displays`). See `docs/DISPLAY-OUTPUTS-PLAN.md`.
  - **Linux:** the server drives the displays itself with kernel mode setting (DRM dumb buffers, page flips at vertical blank, from the screen's CPU frame, so the shared GPU device isn't involved): no desktop, compositor or auto-login. The server must be the card's DRM master, so a display box runs `vesper-display.service` and no desktop session on that card. Hot-plug is polled every 2 s; an unplugged display's output is `unavailable` and restarts when it's back.
  - **Windows** (and Linux desktops): borderless fullscreen windows, still to come.
- **OMT (Open Media Transport).** An MIT-licensed, royalty-free network video protocol that vMix 29 and OBS support, through IDN's `idn-omt` crate (compiled in). Outputs are `OutputTransport::Omt` bindings beside IDN ones; OMT sources are live sources named `omt:…` on the video-input layer and in fills. See `docs/OMT-PLAN.md`.
- **Recording and streaming** (batch 13, `docs/RECORD-STREAM-PLAN.md`): a screen with the program audio recorded to MP4/MKV in the data folder and streamed to RTMP(S) and SRT targets through IDN's gateway (`idn-ffmpeg`, the GPU's encoders), keys kept on the box.
- **Later:** Blackmagic DeckLink SDI.

**Deployment decision:** boxes that drive physical outputs run the server as a system service that drives the displays directly (`vesper-display.service`, kernel mode setting, no desktop). Network-only (IDN/OMT) boxes run it as a plain systemd service (`vesper.service`). The server user must be in the `render` and `video` groups (GPU access).

### Inputs
- **Media files:** video, images and audio. Video decodes through **FFmpeg from Ubuntu's `apt`** (a package dependency, never bundled), with **VA-API hardware decode** on AMD/Intel and software fallback. Frames are uploaded to wgpu textures. See *Media dependency policy*.
- **Network video receiver (IDN Open Protocol):** any compatible source on the network can be used as a video input layer (planned; today the receiver is used for verification only).
- **Audio:** media audio (video soundtracks, the audio layer) is mixed at 48 kHz and sent to a local sound card (IDN's `idn-audio`, which corrects the card's clock drift and reports when each block is heard and the card's health; on Linux the server user must be in the `audio` group) and/or embedded in IDN outputs. Without a device a null output keeps the audio clock running. A video with sound follows the sound it plays (the audio output position is its clock), so picture and sound stay within a frame.

### Storage
Everything lives in one data folder on the server. To back up a church, copy that folder.
Storage is plain JSON: one file per document (`<kind>` is the camelCase document kind), each
written atomically (temporary file, fsync, rename; `store::apply` for batches) and migrated with
`model::migrate` on load. A document written by a newer schema is never overwritten. Search uses
an in-memory index; there is no database.

```
data/
  portable/<kind>/<id>.json   box documents: screens, looks, show settings, masks, video and
                              audio inputs
  machine/<kind>/<id>.json    machine-local documents: output and input bindings, media root
                              bindings, audio device, font substitutions
  machine/server.json         server ID + name; clients.json only while clients paired before
                              site keys haven't swapped their old credential (token hashes only)
  machine/codex.json          this box's Codex role (host / client / dormant), its active
                              collection, the Codex it follows, and (on a show box) its own
                              Codex's dormant collection
  machine/codex-member/       the Codex's own identity in its site (on the Codex box; never
                              in a settings backup)
  machine/codex-export.json   scheduled Codex exports' settings (folder, time, retention);
                              codex-export-passphrase beside it: their stored passphrase (0600;
                              never answered back, never in a snapshot or an export)
  machine/site-manager/       the site's IDN Site Manager (site CA key, devices, users, named API
                              keys, its audit) on the site's server; site.json + site-audit.jsonl
                              beside it
  machine/site-member/        this server's identity as a machine of another server's site
                              (while it is in one)
  codex/                      only on the Codex box (the site's server): the site's Codex
    codex.json                {schemaVersion, id, createdAt}
    collections/<id>/collection.json        one collection (per campus, ministry or room)
    collections/<id>/portable/<kind>/...    its documents: presentations, libraries (shelves),
                                            playlists, themes, media assets, bins, media roots,
                                            global groups, timers, messages, props, macros,
                                            clear groups, stage layouts
    library/                  the Codex's media library: IDN's site media library (items with
                              lasting IDs and SHA-256s, a folder per collection), served to the
                              site's machines; media assets refer to items as {id, sha256}
    media/<sha256>.<ext>      the old managed media store (moved into library/ at start)
    backups/                  settings backups: a set per product and machine (each box's
                              settings, each Oriel's preferences), snapshots, blobs by SHA-256
    exports/                  scheduled encrypted exports' default folder (the admin sets
                              another: a USB drive, a NAS mount)
  codex-cache/                on a show box (a machine of a site whose server hosts the Codex):
    collections/<id>/         its copy of the collection it shows (documents byte for byte,
                              state.json {codexId, seq})
    media/                    IDN's library cache: the media its playlists and bins use, and
                              what was triggered (size-capped, 32 GiB by default)
  codex-import/               uploaded Codex exports waiting for their import (an hour)
  fonts/                      extra fonts (optional)
  cache/thumbnails/           generated slide thumbnails (safe to delete)
```

A standalone install from before Codex (documents in `portable/`, media in `media/`, as
`pro-import` writes them) becomes its own Codex on first start: the collection kinds and `media/`
are moved into `codex/` by a resumable migration (`codex/migration.json`), into one collection
named after the server; the managed media then move into the Codex's media library (IDN's,
`codex/library/`) by a second resumable migration (`codex/library-migration.json`), which
rewrites the media assets' locations as library items. Old-layout documents that appear later in `portable/` move into the active
collection the same way at the next start or `reloadData`; a document is never moved over one
written by a newer schema. `pro-import` refuses to write into a data folder that has moved into
Codex (it would recreate the box's `portable/` and `machine/`, with its identity, site membership,
screens and looks): import into a fresh folder. See `docs/CODEX-PLAN.md`.

Media is uploaded through the API, so control machines never need shared drives. The media files
themselves are hosted by IDN (owner, 2026-11-22): the Codex box runs IDN's site media library and
the Codex refers to its items; IDN keeps the files, their SHA-256s, the probes and the trash, and
the Codex marks the items its documents use so they can't be deleted by accident.

**Show boxes** (`codex::mirror`, `codex::link`): a server that joins a site whose server hosts the
Codex puts its own Codex to sleep (kept intact; leaving the site wakes it) and shows a copy of one
of the site's collections, kept in step over IDN's `channel/1` (the Codex pushes changes; the box
also polls; a resync compares SHA-256s). Media the box's playlists and bins use are copied to it
ahead of time through IDN's library cache; others when triggered. Live output plays only from local
disk: a file not copied yet behaves like missing media. Content edits on a show box are written
through to the Codex (forwarded over the link as the client sent them, run there as a local REST
write; the box's copy takes the change before it answers); while the Codex can't be reached they
answer `503` and the show keeps playing. Uploads and Codex management go to the Codex box (a show
box answers them with `409 codexElsewhere` naming it).
A show box offers its settings to the Codex on the same backup schedule as the Codex box (daily,
after changes, on demand), and takes a restore an admin asks for on the Codex (the restore waits
in the Codex's queue; the Codex tells the box at once, and the box's 30 s poll is the fallback).
Searches beyond the box's own collection are asked of the Codex. Servers with Codex announce the
`codex` capability; `GET /api/v1/codex` says the box's role.

**Encrypted export** (`codex::export`, M13): the Codex's documents (every collection with its
change log) and its settings backups, plus on the site's server the Site Manager's folder (IDN's
own site backup inside: always in scheduled exports, in a manual one only when asked,
`includeSite`, off by default; owner, 2026-11-23), in one IDN backup file (`idn_open_protocol::backup`, kind
`vesper-codex`: Argon2id and ChaCha20-Poly1305 with a passphrase; no crypto of Vesper's own).
**Never media:** the media folder needs its own backup (a NAS copy of `codex/library/`); after
an import the media are found again by SHA-256 once their files are back. A manual export asks
its own passphrase and downloads the file; scheduled exports run nightly into a folder the admin
sets, with a passphrase the admin stored, keeping 7 nightly and 4 weekly by default. An import is
previewed first and applied with explicit choices (new collections added, existing ones replaced
only when asked, their current copy into the collection trash first); its site part is staged
and put in place by the Site Manager at the next start. Servers with it announce `codexExport`.

### API
The control protocol is specified for client authors in `crates/protocol/README.md`; its types (and the generated TypeScript bindings) live in the `protocol` crate.

- **REST** (axum) for CRUD on library, playlists, media and configuration.
- **WebSocket** for live state, commands, and low-res preview frames (JPEG at a few fps for preview/live monitors in the UI).
- **mDNS** (`_vesper._tcp`) so control apps find servers on the LAN automatically.
- **Auth:** pair a client to a server with a code shown on the server, or by a site admin's one-click approval (the same steps at every server; at a server that isn't the site's server the pairing is relayed to it, and the code it shows is one the site's Site Manager made for it, given only to a machine the site marked a pairing relay: the site's server marks its Vesper boxes itself, as they join and at its start, never other machines; see `crates/vesper-site/src/relays.rs`); the client gets a **named site API key** (audience `app:vesper`, scope `control`, 10 years, a browser's 1; revoking it in the site is the control) that works on every Vesper server of the site. A key Vesper recorded as made for a pairing that is still unused a day after it was made is revoked; only recorded keys, never by name (owner, 2026-11-23: a key an admin named `Oriel – …` by hand is never touched). Every server checks keys locally on each REST request and WebSocket: the site's server with its Site Manager (`Handle::verify_token`), the others with their site membership (`TokenCheck::from_enrollment`), to which the site pushes revocations, so they hold offline. The site's device list marks the Codex's own membership `builtIn` (owner, 2026-11-23): Oriel shows it as "Codex (built-in)", part of Vesper, with no remove button, and the server refuses removing it (`409`). People sign in only for the site's admin actions at access level `users`; running the show never needs a sign-in. Clients paired before keys swap their old credential for a key once, without pairing again. Church LANs aren't trusted networks.
- **Later:** a Bitfocus Companion module, and possibly a compatibility shim for the ProPresenter remote API so existing Stream Deck setups keep working.

## Control app (Vesper Oriel)

Tauri v2 with a web frontend. Installers are small (a few MB) and run on Windows, macOS and Linux. Windows and Linux PCs can be control machines whatever OS the server runs. The same frontend build can be served by the server to browsers for remote and stage-manager views.

Main window layout, following ProPresenter: library/playlist sidebar | slide grid | preview + live + layer controls. Media bin along the bottom. Editor as a separate mode.

## Deployment modes

All rendering happens in the Vesper server; every UI is a remote front end. The same server code runs three ways (decided 2026-11-19):

- **Standalone:** one Vesper app with the server embedded in the binary, in the same process as the Oriel UI. Needs no IDN Node; the server's built-in Site Manager acts as a one-machine site.
- **Alongside:** the server and the UIs (Oriel, Cantor, Transept, …) as separate programs on one PC. Products exchange video and sound through IDN's same-machine shared-memory path instead of the network.
- **Distributed:** the server on its own box, the UIs and display boxes on other PCs, over the network.

## Media dependency policy

- On Linux, FFmpeg is a **system dependency** installed by `apt` (Ubuntu's build), loaded as shared libraries. We never vendor, bundle or statically link it, so Ubuntu, not this project, distributes the H.264/HEVC implementations. On Windows (decided 2026-11-23) the server ships a pinned **LGPL-only shared** FFmpeg build next to `vesper.exe`, linked dynamically and replaceable, with its licence and source notice (`docs/PACKAGING.md`).
- Hardware first: VA-API decode on the church boxes' iGPUs; for any future recording/streaming use VA-API encoders (`h264_vaapi`/`hevc_vaapi`), not x264/x265.
- Decoding may go through IDN's `idn-ffmpeg` adapter, which links the same system FFmpeg dynamically (decided 2026-11-19; this replaces the old rule that kept FFmpeg and the network video library strictly separate).
- Prefer royalty-free formats for anything we create: AV1/VP9 for recordings, MJPEG/JPEG for previews. Our network outputs (IDN, OMT) carry raw or their own codecs, not H.264.
- H.264/HEVC ("HX") camera streams from network sources go to the host's decoder; the network video library does not embed its own.

## Licensing

- The project license is **MIT/Apache-2.0**. No proprietary runtime or SDK is used, loaded, bundled or vendored.
- Network video is **IDN Open Protocol**, a separate MIT/Apache-2.0 library (`idn-open-protocol`) developed as a clean-room project and compiled in. It is used as a **git dependency** (`github.com/ADC-Industries/idn-open-protocol`, pinned to a release tag in every crate's `Cargo.toml`, recorded in `Cargo.lock`) and is never vendored or modified here. We only call it "compatible with NDI®" and carry the trademark notice: NDI® is a registered trademark of Vizrt NDI AB; neither this project nor the library is affiliated with or endorsed by Vizrt.
- Interoperability with ProPresenter comes from reading its file formats and public API, never from decompiling it or copying its code or assets.

## Milestones

1. ✅ **M0 – Data model + importer.** Workspace scaffold; `pro-import` reads a real ProPresenter backup and dumps our model as JSON. This proves the model covers real-world content.
2. ✅ **M1 – First pixels out** (OMT output added in batch 13 with `idn-omt`). Server renders a text slide on the GPU, headless, to one network video output. A CLI triggers slides. This proves wgpu headless + network video on Linux.
3. **M2 – Control app skeleton.** Discovery, library, playlist, slide grid, click to go live, live-state sync between two clients.
4. **M3 – Multi-screen.** Screens, looks, physical display outputs, video/image/audio media, transitions.
   - ✅ **M3a – Video and transitions.** `crates/media`: one player per video (decode-ahead thread, VA-API with per-stream software fallback, nearest-frame pacing, seamless loops); GPU YUV → RGB per stream; per-layer cut / dissolve in linear light; media transport. Soft loops, display outputs and the remaining transition styles are still to come.
   - ✅ **M3b – Audio.** `crates/audio` (mixer, engine thread, ALSA / null output, xrun counting) and audio decoding in `crates/media`; video soundtracks with the audio clock as the video clock (A/V within a frame across loops, seeks, pauses); the audio layer with audio bins and audio playlists (next / previous / shuffle / repeat); master, layer and per-output levels and mutes, meters; embedded audio on network outputs (planar float, matching timecodes, its own thread); Audio tab, meters and Settings ▸ Audio in the control app. Sound at playback rates other than 1 is still to come.
5. **M4 – Production features.** Network video input layer, stage display, timers, messages, props, announcements.
   - ✅ **M4.1 – Timers, messages, props, slide actions.** Timers run on the server (countdown, countdown to a time of day, elapsed; overrun / expiry; ticks once a second). Live data in rendered slides (`render::live`: data-link text with timer / clock formats, visibility conditions, colour triggers), re-rendered only when a shown value changes. Messages (token substitution, theme template or built-in banner; the last one shown is drawn), props (stacked, each with its own dissolve), slide actions run in order when a slide is triggered (skipped kinds counted). REST CRUD for timers, messages, props; props edited in the Edit view as one-slide documents; slide action edit ops. Stage layouts are recorded for M4.2.
   - ✅ **M4.2 – Stage display.** Stage screens render a stage layout with live data (current / next slide text following the playlist, notes, clock, timers, video countdown, slide pictures of the slide layer) and never the audience layers; the stage message (layout element or built-in top banner, optional flash); the layout per stage screen (`setStageLayout`, stored in the screen document; default: the first layout, else the built-in "Current / Next"); stage layout CRUD with built-in templates, edited in the Edit view like props (Data link insert). Oriel: Stage tab (screens, layout thumbnails, message box with presets), Screens ▸ Stage Screen ▸, Edit Layouts…. "Next" follows arrangements (the playlist item's, else the presentation's).
   - ✅ **Network video: IDN Open Protocol only.** The official runtime and its loader were removed; IDN Open Protocol (compatible with NDI®) is the one network transport for outputs and the verification receiver. Schema 2 migrates stored `ndi` bindings to `idn`.
6. **M5 – Editor + packaging.** Slide/theme editor, server `.deb` (depends on ffmpeg libs, mesa VA-API drivers, avahi) + systemd units, control-app installers for Windows/macOS/Linux, Companion module.
   - **Packaging** (`docs/PACKAGING.md`): the server's `.deb` with its systemd unit for network-only boxes, the Windows server (cross-built, with its installer and a startup task), Oriel's `.deb`/AppImage and Windows installer; the owner's Windows test checklist. The display boxes' unit (`vesper-display.service`) came with the display outputs. Still to come: code signing, macOS.
7. **M6 – Church management connectors.** Vesper integrates with church management systems; it never becomes one. Connectors are read-only where possible and store no member data in Vesper.
   - **Planning Center Services:** import service plans (order of service) as playlists, with songs matched to the library, segment headers and arrangement/key hints.
   - **ChurchCRM:** service and event data, and announcements for signage.
   - **Rock RMS:** service plans and content channels, events and announcements, and check-in (parent pager).
   - **CCLI usage reporting:** Vesper logs every song shown live (CCLI number, title, date, and whether lyrics were projected, streamed or recorded), and produces the church's licence report: automatic submission through CCLI's reporting integration where available, plus a CSV/printable export for manual entry.
   - **SongSelect import (files and paste):** CCLI's SongSelect API partner program is retired, so Vesper imports what the church's subscriber downloads: lyrics `.txt`, ChordPro (Premium) and SongShow Plus `.usr` files, several at once (Import ▸ Song from SongSelect…), or SongSelect lyrics pasted as text (also recognised when pasted into Reflow). Sections become groups; title, writers, CCLI number, copyright and key become the song's metadata, so its copyright line shows under the words with the church's licence number; a song already in the library (same CCLI number) can be skipped, replaced in place or kept twice (`model::songselect`, `POST /api/v1/songselect/parse`). Settings ▸ CCLI holds the church's licence number (offering the one imported files carry), the copyright line on or off and its slides (`/api/v1/settings/ccli`). In the desktop app, an optional watcher (per computer, off by default) notices a SongSelect file landing in the Downloads folder, recognised by its content (`model::songselect::recognise`), and offers to import it through the same preview; it never imports by itself. No scraping or automated access to CCLI's sites.
   - **Report submission depends on CCLI's terms.** Automatic report submission needs CCLI partner access; until that's arranged, the fallback is the report export.
   - Shared features built on these: **parent pager** messages from child check-in (on the messages layer), **events/announcements feeds** for signage, **volunteer schedules** on stage screens.

## Product family roadmap

- **Signage** (formerly Vesper Narthex; owner, 2026-11-24): folded into Vesper and Mosaic, not a separate product. Authored in Oriel, rendered on a Vesper server as channels, shown on Mosaic displays (see `docs/PRODUCTS.md`). After M4.3 and M5.
- **Vesper Transept** (video switcher): product UI and workflow on IDN's `idn-switcher` engine (IDN roadmap R.15).
- **Vesper Cantor** (audio mixing console): product UI and workflow on IDN's `idn-mixer` engine (IDN roadmap R.16).
- **Vesper Codex** (library): hosts the content for Vesper/Oriel and signage (presentations, songs, media, themes) as **collections**, e.g. per campus, ministry or room (the sidebar shelves of presentations stay **libraries**, as in ProPresenter). It also stores the **settings backups** for Vesper, Oriel, Mosaic, Cantor, Transept and Herald. Codex is a **module of the Vesper server**; there is **one Codex per site**, on the site's server (the Site Manager box), which other buildings reach over the church VPN (routed, without NAT). There are no replicas: show boxes cache what they have scheduled and always play from local disk, edits are written through to the Codex, and a scheduled encrypted export protects it (documents and settings backups; the media folder needs its own backup). Machines move files over IDN's file channel between site machines. Plan: `docs/CODEX-PLAN.md`. Everything works on the LAN without the internet (internet outages are common); LAN outages are rare and are not a design priority, since the systems are remote-controlled anyway.
- `idn-cli` is bundled with Vesper Launcher as support tools (`idn doctor`, `find`, `recv`, `ptz`). `idn-industrial` is out of scope.
- **Vesper Herald** (intercom): product UI and workflow on IDN Voice (`idn-voice`).
- **Vesper Launcher:** installs, launches, updates and monitors the Vesper apps on each machine.
- **IDN components Vesper adopts:**
  - `idn-ffmpeg` and `idn-audio` replace Vesper's own media decode and audio device I/O, so every product shares IDN's internal data transfer (system FFmpeg policy unchanged);
  - `idn-shq-gpu` for GPU SpeedHQ decode on Vesper's shared wgpu device;
  - `idn-omt` for OMT interoperability;
  - `idn-vcam` is used as is (Windows virtual camera);
  - `idn-panel` is replaced by Vesper's own management views.
- **Platform direction:** IDN is built into the backends (the UIs are remote controls). Every Vesper server is always in a site, with exactly one Site Manager per site, a module of one Vesper server, with management and monitoring inside the software. A server on its own runs its own site; it can join another Vesper server's site as one of its machines (by code and/or approval), and its own Site Manager then stays off until it leaves. A server whose own site still has other machines can't join another (they'd be orphaned). Pairing at a server that isn't the site's server is relayed to the site's server: that server shows a pairing code the site's Site Manager made for it, and typed right the pairing is approved at once; otherwise an admin approves it there. Vesper products auto-join a default site (join approval to be designed). Who may administer the site is the site setting `access`: `open` (the default), `pin` (one admin PIN) or `users` (users with roles). With `open`, **any paired Oriel can approve new devices** into the site, remove machines and change the site's settings: pairing with the server is the only gate (see `crates/protocol/README.md`, "Site").
- **Vesper Mosaic** (displays): display receivers built on IDN's `idn-display`. Small boxes behind projectors, TVs and confidence monitors show chosen IDN sources full screen, configured centrally; video walls and multi-screen layouts. Covers Vesper's physical display outputs and the lobby (signage) screens. A Mosaic box runs the IDN Node with IDN's display service (owner, 2026-11-25), not a Vesper server; the site's Vesper server manages the boxes (the `mosaic` module: screens, feeds, settings, site-wide channels) and Oriel is the UI (Site ▸ Displays). It supports a single display and displays linked into a video wall (one box with several outputs, or several boxes, each screen a tile). Plan: `docs/MOSAIC-PLAN.md`.
- IDN pieces folded into products: Cantor also covers `idn-dsp`, `idn-plugin-host`, `idn-daw` and `idn-asio`; IDN live captions (`idn-captions`) are integrated into Vesper and Transept; the IDN OBS plugin (`idn-obs`) is used as is; IDN Remote is not part of Vesper.

## Open questions

- Svelte vs Solid for the frontend.
