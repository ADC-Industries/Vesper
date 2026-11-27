# `protocol`: the control protocol

How a client (the control app, a browser remote, a Stream Deck / Companion script) talks to the server. Every type on the wire is defined in this crate. The TypeScript types in [`bindings/`](bindings/) are generated from it, so the UI and the server cannot drift apart.

- **REST** (`/api/v1/...`) for documents (libraries, presentations, playlists, thumbnails, media), and for commands as `POST`.
- **One WebSocket** (`/api/v1/ws`) for live state, events, commands and live preview frames.
- **mDNS** so clients find servers on the LAN.
- **Pairing** so that only approved clients can control the show.

The server is **Vesper** (binary `vesper`) and the control app is **Oriel**. The `product_id!` macro in `src/lib.rs` spells the server's lowercase identifier in one place (it sets the mDNS type `_vesper._tcp`).

## Conventions

- JSON only. Fields are `camelCase`. IDs are UUID strings. Times are seconds (`f64`). Timestamps are RFC 3339 UTC.
- Enums with data are tagged with `"type"`, e.g. `{"type": "triggerSlide", ...}`. Plain enums are camelCase strings.
- Optional fields are omitted rather than sent as `null`. `ShowState` is the exception: its shape is fixed, so an empty part is `null` or `[]` and never missing.
- **Versioning.** `PROTOCOL_VERSION = 1` is a major version. It changes only when a change is incompatible. New fields, commands, messages and enum values are compatible changes, so a client must ignore anything it doesn't know. Model enums also carry an `"unknown"` value for that purpose.
- Model documents (`Presentation`, `PlaylistNode`, `Look`, `Screen`, `ClearGroup`, `MediaAsset`) are served in their stored form. See the `model` crate.

## Discovery

The server advertises `_vesper._tcp.local.` (DNS-SD) on the API port (default **5555**). Its TXT record holds:

| key | value |
|---|---|
| `id` | server ID (stable UUID): the key to remember a server by |
| `name` | display name (default: host name) |
| `version` | server software version |
| `proto` | protocol major version |

From Rust, `protocol::discover::discover(timeout)` (feature `discover`) returns the servers it finds, and `discover::Browser` streams found/lost events. The server's advertisement shares UDP 5353 with Avahi or Bonjour. Start the server with `--no-mdns` to turn it off. To list servers from a shell: `cargo run -p vesper-server --example ws_probe -- --discover`.

## Authentication and pairing

Church LANs aren't trusted networks. Every Vesper server is in an IDN site (see [Site](#site)), and a paired client's credential is a **named site API key**: one key per client, valid on every Vesper server of the site, listed in and revoked from the site's Site Manager.

1. **The server machine itself** (127.0.0.1 or ::1) is trusted without a key, so a control app on the same box just works. `vesper --no-trust-localhost` turns this off. A web page from elsewhere open in a browser there isn't: a request with an `Origin` other than the desktop app's (`tauri://localhost`, `http(s)://tauri.localhost`) or a page of the machine itself (`localhost`, `127.0.0.1`, `[::1]`, any port) needs a key like any other client; so does a change (anything but `GET`/`HEAD`) that a browser marks as sent by another page (`Sec-Fetch-Site: cross-site` or `same-site`) without such an `Origin`, and any request whose `Referer` names a page elsewhere. (The desktop app's `<img>` loads carry neither `Origin` nor `Referer`: plain reads keep the machine's trust.) Nor is a request addressed to any other host name, even one that resolves to the server machine (a page served as `http://oriel.test:5621/` reads its own origin without sending `Origin`, as after DNS rebinding): the `Host` header must be `localhost`, `127.0.0.1` or `[::1]` (any port; the desktop app's `tauri.localhost` too), for REST and the WebSocket alike, or the request is a remote client's.
2. **Any other client pairs once**, in one of two ways, the same at every server of the site:
   - **The code** the server shows: in its log, in `GET /api/v1/pairing/code` and in `GET /api/v1/server` (the last two only answer requests from the server machine, so a local UI can display it). At the site's server it is the server's own: it rotates after every use (it pairs one client, even if two send it at once) and every 10 minutes, and it can also change after repeated wrong tries (10 wrong codes from any addresses), so read it again if it was refused. At the site's other servers it is a code the site's Site Manager makes for that server (see below).
     ```http
     POST /api/v1/pair
     {"code": "482913", "clientName": "Booth laptop"}

     200 {"token": "idn1.eyJ2Ijox…", "clientId": "3f9a0c12d4e5"}
     ```
   - **A site admin's approval:** without `code`, the pairing waits for an admin to approve it on the Site page (`GET /api/v1/site/pairings`, see [Site](#site)):
     ```http
     POST /api/v1/pair
     {"clientName": "Booth laptop"}

     202 {"pairingId": "2d02f28b…32 hex…", "expiresInSeconds": 600}
     ```
     The client asks `GET /api/v1/pair/{pairingId}` (no key needed; the id is secret) every second or so: `202 PairPending` while it waits, `200 PairResponse` once approved (given once; a key approved at the site's server and not fetched before the pairing ends is revoked, since it reached no one), `403 forbidden` when refused (the message says why), `404 notFound` when it ended unanswered after 10 minutes, or a minute after its client last asked (or its server lost it). At most 3 pairings wait from one address (an IPv6 address's /64) at once, 64 in all: another answers `429 rateLimited`. A client that stops waiting (the operator goes back, or types the code again after `codeWrong`) takes the pairing back with `DELETE /api/v1/pair/{pairingId}` (no key needed): `204`, and it is gone from the site's waiting list (at the site's server it ends with `key_request_cancelled`, as on expiry; a relayed one is taken back at the site's server); `404` when it is unknown or already answered (an answer stays to be fetched).

   The site's Site Manager makes the key: audience `app:vesper` on any machine of the site, scope `control`, named `Oriel – <clientName>` (60 characters at most), living as long as IDN allows (10 years), or a year for a web browser (`"browser": true` in the request). Revoking it is the control. The token is shown once; `clientId` is the key's id. At most 5 pair attempts (with or without a code) per minute per IP address; past that, `429 rateLimited`. Without a site (the server's own Site Manager is off on the command line and it isn't in another site), pairing answers `503 unavailable`.

   **At a server of the site that isn't the site's server** (a relay), the pairing is relayed to the site's server; the relay keeps nothing, and the key works on every server of the site, also once the relay is gone. The code a relay shows is made by the site's Site Manager for that relay: six digits, good for 5 minutes, until it is used once, or until 5 wrong tries burn it; the relay shows the next one at once after a pairing with a code, and when it ends (`GET /api/v1/pairing/code` answers `503 unavailable` while the relay has none from its site yet). The relay passes the typed code on and never judges it: the site's server checks it, and **the right code is approved at once**, without anyone's click, so `POST /api/v1/pair` answers `200 PairResponse` (the relay waits up to 3 seconds for the site's answer). A wrong, used or expired code isn't refused: the pairing waits for an admin's approval at the site's server, like one without a code (`202`), and its `PairPending` says `"codeWrong": true` (in the first answer, or soon after when the site's server answers late), so the client can offer to type the code again. A relayed client's key lives 10 years even in a browser (the relayed request can't say what kind of client it is).

   **Only a pairing relay shows a code** (IDN v0.3.4-Beta11): the site gives its codes only to a machine it marked a relay (`SiteDevice.relay`); through any other machine of the site `GET /api/v1/pairing/code` stays `503` and every pairing waits for an admin's approval at the site's server. The site's server marks its **Vesper boxes** itself: a machine enrolled as an app whose name is the one every Vesper server joins with, `<name> (Vesper)` (for one approved in Vesper under another name, the name it asked with). It marks one as it joins (by code, approval here or in IDN's admin panel), and at its start every enrolled Vesper box not marked yet (an existing site after the upgrade). Never another machine (an IDN display box, a camera PC, another program's app; nor the Codex's own membership): the mark lets a machine's pairings be approved by its code with nobody's click. Each mark is in the Site Manager's audit (`relay`, by the `server machine`). An admin can take a mark off any machine or put one on a Vesper box (`POST /api/v1/site/devices/{fingerprint}/relay`; any other machine answers `409`); the site's server never marks a machine again once it marked it or an admin set it (`site.json` keeps which). Leaving the site or being removed ends the mark.
3. **Send the key on every request**, as `Authorization: Bearer <key>`. For the WebSocket, use the query parameter `?token=<key>`, because browsers can't set headers on a WebSocket.
4. **Checking** is local on every server, on every request and WebSocket upgrade: the site's signature, the audience, the scope `control`, the time, and the key not revoked. The site's server checks with its Site Manager; the site's other servers with their site membership, to which the site pushes every revocation, so a revoked key stops even on a server that can't reach the site's server. An open WebSocket is checked again every 2 seconds and closed with code `4003` once its key is revoked.
5. **Revoking:** `GET /api/v1/site/api-keys` lists the site's keys (with when and where each was last used); `DELETE /api/v1/site/api-keys/{id}` (a site admin, see [Site](#site)) revokes one. It stops on every server of the site at once. `DELETE /api/v1/clients/{id}` (server machine only) revokes a key by its id too.
6. **Credentials from before site keys.** A client paired before keys still sends its old 43-character token. It works, and `GET /api/v1/server` answers it with `"upgradeCredential": true`: the client calls `POST /api/v1/pair/upgrade` with it once (body `{"browser": true}` for a browser, optional) and gets its site key (`PairResponse`, named after the client's old name), stores it and uses it from then on. Nobody pairs again. The old credential keeps working until the key is first used, then it is dropped; asked again before that (the answer lost), the key handed over earlier is revoked and a new one made. `GET /api/v1/clients` (server machine only) lists the clients still to swap; once none is left, the old store (`<data>/machine/clients.json`) is removed and no longer used. The swap needs no one's approval, so it is made only at the site's server; at the site's other servers the old credential keeps working (`501` to the swap).

People sign in only for the site's admin actions at access level `users` (see [Site](#site)); running the show never needs a sign-in, at any access level.

Without a valid key, everything except `GET /api/v1/server`, `POST /api/v1/pair`, and `GET` and `DELETE /api/v1/pair/{pairingId}` answers `401 unauthorized`. A paired client that calls a route reserved for the server machine gets `403 forbidden`. `GET /api/v1/server` reports `auth` as `local`, `paired` or `unpaired`, so a client can tell when it needs to pair.

## REST

| Route | Response |
|---|---|
| `GET /api/v1/server` | `ServerInfo`: `{id, name, version, protocolVersion, capabilities, auth, pairingCode?, upgradeCredential?}` |
| `POST /api/v1/pair` | `PairRequest {code?, clientName, browser?, launcher?}` (`launcher`: a Vesper Launcher's machine ID, see [Updates](#updates)) → `PairResponse` (a named site API key), or `202 PairPending` waiting for an admin's approval |
| `GET /api/v1/pair/{pairingId}` | a waiting pairing: `202 PairPending`, `200 PairResponse` (once), `403` refused, `404` ended |
| `DELETE /api/v1/pair/{pairingId}` | the client takes a waiting pairing back: `204`; `404` unknown or answered |
| `POST /api/v1/pair/upgrade` | `PairResponse`: a credential from before site keys swapped for its key, once |
| `GET /api/v1/pairing/code` | `PairingCode` (server machine only) |
| `GET /api/v1/clients`, `DELETE /api/v1/clients/{id}` | `ClientInfo[]` of the clients still on a credential from before site keys; revoke one (with the site key it was swapped for, if not used yet), or a site key by its id: `204` (server machine only) |
| `GET /api/v1/state` | `StateSnapshot`: `{revision, state}` |
| `POST /api/v1/commands` | any `Command`. Returns `CommandResponse {ok, revision?, error?}` with the error's HTTP status |
| `GET /api/v1/libraries`, `POST` | `LibrarySummary[]`; create → `201 DocumentRevision`. See [Editing content](#editing-content) |
| `PUT`, `DELETE /api/v1/libraries/{id}` | rename (`If-Match`) → `DocumentRevision`; delete (only when empty) → `DeletedDocuments` |
| `GET /api/v1/libraries/{id}/presentations` | `PresentationSummary[]` (sorted by name) |
| `GET /api/v1/presentations` | `PresentationSummary[]` (every library). A summary carries the presentation's `kind` (`song`: song metadata or arrangements; `scripture`: a passage; `media`: no text but media on its slides; else `text`) and, when known, `authors`, `copyrightYear`, `ccliNumber` (song metadata) and `theme` / `themeName` (the theme its slides follow, else the one they were made from) |
| `GET /api/v1/presentations/{id}` | `Presentation` (the full document: sections, slides, elements; `ETag` = revision) |
| `POST /api/v1/presentations` | `CreatePresentation` → `201 DocumentRevision` |
| `POST /api/v1/presentations/{id}/edits` | `PresentationEdit` → `PresentationEditResult` (optional `If-Match`: applied only to that revision) |
| `POST /api/v1/chordpro/parse`, `GET /api/v1/presentations/{id}/chordpro` | `ChordProParse` → `ChordProParsed` (a new, unsaved presentation); a presentation as ChordPro text. See [Chords](#chords) |
| `POST /api/v1/presentations/{id}/duplicate`, `/move` | `DuplicatePresentation` → `201`, `MovePresentation` (`If-Match`) → `DocumentRevision` |
| `DELETE /api/v1/presentations/{id}` | `DeletedDocuments` |
| `GET /api/v1/groups` | `GlobalGroup[]` (group pickers) |
| `POST /api/v1/groups`, `PUT`, `DELETE /api/v1/groups/{id}` | `CreateGlobalGroup {name?, color?, hotkey?, after?, document?}` → `201`; `UpdateGlobalGroup {name?, color?, hotkey?, after?}` (`If-Match`; `null` clears colour / hotkey, `after: null` = first); delete (into the trash) → `DeletedDocuments` |
| `GET /api/v1/presentations/{id}/slides/{slideId}/thumbnail?w=320` | JPEG, see [Thumbnails](#thumbnails) |
| `POST /api/v1/presentations/{id}/thumbnails?w=320` | `202 ThumbnailPrefetch {slides, cached, queued}` |
| `GET /api/v1/presentations/{id}/slides/{slideId}/render?w=960&hide=<id>,<id>` | PNG, see [Editor render](#editor-render) |
| `GET /api/v1/playlists` | `PlaylistTreeNode[]` (folders and playlists, nested, no items) |
| `GET /api/v1/playlists/{id}` | `PlaylistDetail` (items with `displayName` and the presentation's summary) |
| `POST /api/v1/playlists`, `PUT`, `DELETE /api/v1/playlists/{id}` | `CreatePlaylistNode` → `201`; `UpdatePlaylistNode` (rename / move, `If-Match`); delete (a folder takes its subtree) → `DeletedDocuments` |
| `POST /api/v1/playlists/{id}/edits`, `/duplicate` | `PlaylistEdit` → `PlaylistEditResult`; `DuplicatePlaylist` → `201` |
| `GET /api/v1/looks`, `/clear-groups` | `Look[]`, `ClearGroup[]` |
| `POST /api/v1/looks`; `GET`, `PUT`, `DELETE /api/v1/looks/{id}`; `POST …/duplicate` | `LookEntry` (the `Look` plus a `revision`; `ETag`, `If-Match` as for screens); create (partial body; a free "Look N" name, last in the list) and duplicate (`DuplicateLook {name?}`, default "<name> copy", right after the original) → `201`; delete → `204`. See [Looks](#looks) |
| `GET`, `POST /api/v1/masks`; `GET`, `PUT`, `DELETE /api/v1/masks/{id}` (`?detach=true`); `POST …/duplicate` | `MaskEntry[]` / `MaskEntry` (the `Mask` plus a `revision`), same rules as looks; delete → `200 MaskDeleted {looks, screens}`. See [Looks](#looks) |
| `GET`, `POST /api/v1/video-inputs`; `GET`, `PUT`, `DELETE /api/v1/video-inputs/{id}` | `VideoInputEntry[]` / `VideoInputEntry` (the `VideoInput`, this box's binding `source?` and a `revision`); `PUT`/`DELETE` take `If-Match`; delete → `204`. See [Live sources](#live-sources) |
| `GET`, `POST /api/v1/timers`; `GET`, `PUT`, `DELETE /api/v1/timers/{id}` | `TimerEntry[]` / `TimerEntry` (each `Timer` plus a `revision`; `ETag`, `If-Match` as for screens); create → `201`, delete → `204`. See [Timers, messages and props](#timers-messages-and-props) |
| `GET`, `POST /api/v1/messages`; `GET`, `PUT`, `DELETE /api/v1/messages/{id}` | `MessageEntry[]` / `MessageEntry`, same rules |
| `GET`, `POST /api/v1/props`; `GET`, `PUT`, `DELETE /api/v1/props/{id}` | `PropEntry[]` / `PropEntry`, same rules; `POST` takes `fromSlide: {presentation, slide}` |
| `GET /api/v1/props/{id}/thumbnail?w=320` | PNG of the prop alone (alpha), `ETag` |
| `GET /api/v1/props/{id}/document`, `POST /api/v1/props/{id}/edits`, `GET /api/v1/props/{id}/slides/{slideId}/render?w=` | the prop as a one-slide `Presentation` for the Edit view; `PresentationEdit` → `PresentationEditResult`; editor render PNG |
| `GET`, `POST /api/v1/stage-layouts`; `GET`, `PUT`, `DELETE /api/v1/stage-layouts/{id}` | `StageLayoutEntry[]` / `StageLayoutEntry`, same rules; `POST` takes `template` (see [Stage display](#stage-display)) |
| `GET /api/v1/stage-layouts/{id}/document`, `POST …/edits`, `GET …/slides/{slideId}/render?w=`, `GET …/thumbnail?w=` | as for props: the one-slide document, edit ops, editor render, PNG of the layout as designed (`ETag`) |
| `GET /api/v1/stage-layout-templates`, `GET /api/v1/stage-layout-templates/{id}/thumbnail?w=` | `StageLayoutTemplate[]` `{id, name}` (`currentNext`, `lyricsClockTimer`, `notes`); a template's PNG |
| `GET /api/v1/screens`, `POST /api/v1/screens` | `ScreenEntry[]` (each `Screen` plus a `revision`); create → `201 ScreenEntry`. See [Screen configuration](#screen-configuration) |
| `GET`, `PUT`, `DELETE /api/v1/screens/{id}` | `ScreenEntry` (`ETag`); `DELETE` → `ScreenDeleted {outputs, looks}` |
| `PUT /api/v1/screens/{id}/color`, `PUT /api/v1/screens/{id}/corner-pin` | body `ColorCorrection` / `CornerPin` (missing fields = defaults; `{}` resets); saved and applied live, no `If-Match` (the last change wins) → `ScreenEntry`. See [Screen configuration](#screen-configuration) (capability `screenAdjust`) |
| `GET /api/v1/outputs`, `POST /api/v1/outputs` | `OutputEntry[]` (each `OutputBinding` plus a `revision`); create → `201 OutputEntry` |
| `GET`, `PUT`, `DELETE /api/v1/outputs/{id}` | `OutputEntry` (`ETag`); `DELETE` → `204` |
| `GET /api/v1/output-transports` | `OutputTransportInfo[]`: `{kind, label, supported, available, version, status, detail}` per transport (`idn`, `omt`, `display`, `sdi`); `idn` (IDN Open Protocol, compatible with NDI®) and `omt` (Open Media Transport) are always built in and report the library `version` |
| `GET /api/v1/themes`, `POST /api/v1/themes` | `ThemeSummary[]` (each with its template slides `{id, name, slide}` and `usedBy`: presentations following it); `CreateTheme {name?, document?}` → `201 DocumentRevision`. See [Themes](#themes) |
| `GET`, `PUT`, `DELETE /api/v1/themes/{id}`; `POST …/duplicate` | `ThemeEntry {theme, revision}` (`ETag`); `UpdateTheme {name?, secondLanguage?}` (`If-Match`; omitted fields kept; `secondLanguage`: the [second-language style](#dual-language), batch 16); delete (`If-Match`, `?reassign=<theme>` or `?unlink=true`) → `DeletedDocuments`; `DuplicateTheme {name?}` → `201` |
| `GET /api/v1/themes/{id}/document`, `POST …/edits`, `GET …/slides/{slideId}/render?w=`, `GET …/slides/{slideId}/thumbnail?w=`, `GET …/used-by` | as for props: the theme as a presentation, edit ops, editor render, a template slide's PNG (`ETag`); `DocumentUse[]` (presentations following it, then messages using it) |
| `POST /api/v1/displays/{id}/identify` | a display no output uses shows its name, connector and mode full screen for 5 s → `204`; `404` for an unknown display, `409` when it isn't connected, an output uses it (identify its screen with `identifyScreens` instead) or it is already showing its name (capability `screenLayouts`) |
| `GET /api/v1/displays` | `DisplayInfo[]`: the monitors and projectors attached to this box, connected or not: `{id (connector, "HDMI-A-1"), index, name (EDID model, else the connector), manufacturer?, connected, modes: [{width, height, refreshMhz}] (largest first), preferred?, source? ("kms": driven directly on a box without a desktop; "window": a monitor of this computer's desktop, shown as a window; "fake": tests), position? ({x, y}, its top left on the desktop, window displays), primary? (true: the desktop's main monitor), boundTo: [outputId]}`. On a box with displays on more than one graphics card (an iGPU and a graphics card), a KMS display's `id` carries its card (`card1-HDMI-A-1`), since each card numbers its connectors from 1; a selector saved with the bare connector name still finds the one display it can mean (the only one of that name, else the one with the selector's model, else the only connected one). A window display's `id` is the OS monitor name (`\\.\DISPLAY2` on Windows, the output name under X11/Wayland) and its only mode is the desktop's current one (no mode switch). Also in `state.displays`, refreshed within about 2 s when a display is plugged in or out (capability `displays`). A display output binding's `transport.fullscreen: false` shows the screen in a plain movable window instead of covering the monitor (window displays only) |
| `GET /api/v1/media/{id}` | `MediaAssetInfo {asset, online, problem?}` |
| `GET /api/v1/media/{id}/file` | the file itself (supports `Range` and `If-Modified-Since`) |
| `GET /api/v1/media/{id}/used-by`, `GET /api/v1/props/{id}/used-by` | `DocumentUse[] {kind, id, name}`: the documents of the active collection that use the asset or the prop (a media element or action, a playlist item, a bin's item, a theme's asset, a prop action, a macro, …), presentations first, then playlists, props, macros, bins, themes, others; by name within a kind; `404` for one that isn't there |
| `GET /api/v1/media/{id}/thumbnail?w=320` | JPEG: an image decoded and scaled to `w` (never upscaled); for a video, its frame at about 10 % in (cached in memory); a generic placeholder for audio and for videos whose file is missing. `ETag`, `304`; `404` when an image file is missing |
| `GET /api/v1/media-bins` | `MediaBinNode[]`: the bin tree. Folders (from the bins' folder paths, `id: null`) hold sub-folders, then bins. Each node has `name`, `path`, `kind` (`folder` / `manual` / `watched`), `browser` (`media` / `audio`), `itemCount` |
| `GET /api/v1/media-bins/{id}` | `MediaBinDetail`: the bin's items `{id, name, kind (image/video/audio), asset, availability (online/missing), enabled, duration}` (`duration`: seconds of a video / audio file, `null` for stills or unknown) |
| `POST /api/v1/media-bins`, `PUT`, `DELETE /api/v1/media-bins/{id}` | `CreateMediaBin {name?, folder?, browser?, after?, document?}` → `201`; `UpdateMediaBin {name?, folder?, after?}` (rename, move to another folder, reorder; `If-Match`); delete (into the trash; its media stays) → `DeletedDocuments`. See "Media bins and global groups" under Editing content |
| `POST /api/v1/media-bins/{id}/edits` | `MediaBinEdit` → `MediaBinEditResult` (a manual bin's items) |
| `GET /api/v1/audio-devices` | `AudioDeviceInfo[]`: the machine's playback devices `{name, description, available, problem?, channels?}` (`default`, then `hw:<card>,<device>` for every ALSA device; `problem` says e.g. that the server user is not in the `audio` group) |
| `GET`, `PUT /api/v1/audio-device` | the machine's `AudioDeviceConfig {device, channelMap?, masterGainDb?}` (`device: ""` = no local audio). `PUT` stores it and applies it at once (the device is reopened, the master gain set); `masterGainDb` must be −96 … +12 |
| `GET`, `PUT /api/v1/captions/config` | the machine's `CaptionsConfig {enabled?, source?, language?, model?, words?, keepMusic?, maxLines?, style?, audience?, stage?, network?}` (see [Live captions](#live-captions)). `PUT` validates (`badRequest`), stores and applies it at once |
| `POST /api/v1/captions/lines` | `PushCaption {text, final?}`: a caption line from another captioner (`204`; `conflict` while captions are off; at most 500 characters) |
| `GET /api/v1/captions/models` | `CaptionModel[] {name, sizeMb, note, downloaded}`: the speech models offered and which are on the server |
| `GET /api/v1/diagnostics` | `DiagnosticsReport` (fonts, missing media, unsupported features, load issues) |
| `GET /api/v1/server/details` | `ServerDetails {name, version, os, cpu?, cpus, gpu?, uptimeSeconds, products}`: this Vesper's own numbers (G9; `products`: `Vesper`, plus `Codex` and `Site Manager` when this box runs them). Other machines: `GET /site/devices` (`SiteDevice.machine`, as their IDN Nodes report it) |
| `GET /api/v1/server/stats` | `ServerStats {uptimeSeconds, process: {rssBytes?, threads?, openFiles?, heapInUseBytes?, heapFreeBytes?, rustHeapBytes?}, wsClients, screens: ScreenPacing[], videos: VideoStats[], audioXruns, caches: CacheSize[]}`: this server's health counters since it started (only going up; compare two readings): per running screen `ticks`, `lateTicks` (frames the clock skipped), `lateRenders` (video/transition frames finished after they were due), `renders`, `maxRenderMs`, `p95RenderMs`, `framesDropped` (its outputs, busy); per live video `shown`, `skipped`, `late`, `errors`; `process` from `/proc/self` (Linux; absent elsewhere), the heap's bytes in use and held free from glibc's `mallinfo2` (Linux with glibc; absent elsewhere; when `rustHeapBytes` is present they count only the C libraries' memory: FFmpeg, whisper, GPU drivers), `rustHeapBytes` the memory the server's Rust allocator (mimalloc, the default build) has committed (absent on a build without it); `caches` (`{name, entries, bytes}`, bytes 0 when not counted): the renderer's caches (published every few seconds while it works: decoded images, glyphs, slide layers, uploaded images, video textures) and the thumbnails' memory caches. Read by the soak harness (`docs/SOAK-PLAN.md`) |
| `GET /api/v1/fonts` | `FontList {families: [{family, faces: [{weight, italic, postscriptName}]}], substitutions: [{family?, postscriptName?, replacementFamily, installed}], fallbackFamily}`: what the server can render (font pickers) |
| `GET /api/v1/macros` | `Macro[]`: the active collection's macros (stored form, by sort key then name); `triggerMacro` runs one (capability `macros`) |
| `GET`, `PUT /api/v1/remote-compat` | `RemoteCompatStatus {enabled, port, listening, listeningPort?, problem?}`; `PUT` takes `UpdateRemoteCompat {enabled?, port?}`, stores it on the box and applies it at once. See [Remote API compatibility](#remote-api-compatibility) (capability `remoteCompat`) |

**Capabilities.** `GET /api/v1/server`'s `capabilities` lists what the server offers, so a client can hide what an older server lacks (and ignores flags it doesn't know):

| flag | |
|---|---|
| `ws` | the WebSocket (`/api/v1/ws`) |
| `commands` | commands, also as `POST /api/v1/commands` |
| `thumbnails` | slide thumbnails ([Thumbnails](#thumbnails)) |
| `preview` | live preview frames ([Live preview](#live-preview)) |
| `pairing` | pairing and site keys ([Authentication and pairing](#authentication-and-pairing)) |
| `mdns` | announced over mDNS ([Discovery](#discovery)); absent while it isn't (`vesper --no-mdns`, or the announcement failed) |
| `mediaFiles` | media files over REST (`GET /api/v1/media/{id}/file`) |
| `reload` | the `reloadData` command |
| `mediaBins` | `GET /api/v1/media-bins` |
| `screenToggles` | `setScreensEnabled` |
| `screenAdjust` | a screen's `color` and `cornerPin`, and their routes `PUT /api/v1/screens/{id}/color`, `/corner-pin` |
| `fillKey` | an output's `target` can be `fill` or `key` (a Fill/Key pair for a hardware keyer) |
| `screenLayouts` | a screen's `arrangement` (Single / Mirror / Grouped / Edge Blend), an output's `tile` and `region`, `placeholder` outputs, `POST /api/v1/displays/{id}/identify` ([Screen configuration](#screen-configuration)) |
| `macros` | `GET /api/v1/macros` and the `triggerMacro` command |
| `remoteCompat` | the Remote API compatibility layer and its setting (`/api/v1/remote-compat`) |
| `streaming` | recording and streaming a screen: `/api/v1/streaming…`, `/api/v1/recording/…` ([Recording and streaming](#recording-and-streaming)) |
| `updates` | the site's update channel and the Vesper Launchers' reports (`/api/v1/updates/…`, [Updates](#updates)) |
| `mosaic` | the site's display boxes, their screens and channels (`/api/v1/mosaic/…`, [Mosaic](#mosaic)) |
| `signage` | the site's signage channels, playlists, feeds and override (`/api/v1/signage/…`, [Signage](#signage)) |
| `translate` | offline machine translation of slides into a second language: the box's models and previews (`/api/v1/translate/…`, [Machine translation](#machine-translation)) |
| `ping` | the WebSocket heartbeat (`ping` / `pong` messages) |
| `screenConfig` | screens and output bindings over REST ([Screen configuration](#screen-configuration)) |
| `identifyScreens` | `identifyScreens` |
| `testPatterns` | `setTestPattern` |
| `editing` | content editing ([Editing content](#editing-content)) |
| `site` | the IDN site and its Site Manager ([Site](#site)) |
| `codex` | Vesper Codex: collections, change log and trash, media library and uploads, missing media, storage, search, settings backups and restore, show boxes ([Codex](#codex)). Every Vesper server with the module says it, a show box too; what the box is to the site's Codex is `GET /api/v1/codex`'s `role` |
| `codexExport` | the Codex's encrypted export and import ([Export and import](#codex-export)); servers with `codex` but without it exist |
| `themeEditing` | themes are created, edited, duplicated and deleted, and slides follow them ([Themes](#themes)) |
| `lookEditing` | looks are created, edited, duplicated, reordered and deleted, and play their own transition when made live ([Looks](#looks)) |
| `masks` | masks are documents (`/masks`) that looks draw over audience screens ([Looks](#looks)) |
| `scripture` | Bible translations, references, passages, search and presentations from a passage ([Scripture](#scripture)) |
| `presence` | who has what open, and edit locks ([Presence and edit locks](#presence-and-edit-locks)) |
| `presenceBrowsing` | presence places `show` and `timers`; props and timers take edit locks ([Presence and edit locks](#presence-and-edit-locks)) |
| `displays` | display outputs: `GET /api/v1/displays`, `state.displays`, and `display` output bindings with a mode ([Screen configuration](#screen-configuration)); older servers refuse `display` bindings |

**CORS.** Every `/api/v1` route answers cross-origin requests from any origin and allows the `Authorization` and `Content-Type` headers (the packaged desktop app's pages come from `tauri://localhost`). Preflight `OPTIONS` requests need no token. Exposed to the page: `ETag`, `Content-Range`, `Accept-Ranges`, `Content-Disposition`, `X-Codex-Export-Note` and `X-Codex-Media-Included` (an export's file name and note).

**The control UI.** `vesper --ui-dir <dir>` (default: `apps/control/dist` under the working directory, when it exists) serves the built control app at `/`, with unknown non-API paths falling back to `index.html`, so a browser on the LAN can open `http://<server>:5555/`. The UI's files need no token; the app pairs itself.

Errors come back as `{"error": {"code": "notFound", "message": "presentation … not found"}}`. Validation errors also name the request field, `{"error": {"code": "badRequest", "message": "…", "field": "transport.sourceName"}}`, and an edit conflict carries the current document: `{"error": {"code": "conflict", …}, "current": {…ScreenEntry…}}` (`current: null` when it was deleted).

| code | HTTP | meaning |
|---|---|---|
| `badRequest` | 400 | malformed JSON, bad arguments |
| `unauthorized` | 401 | no or bad key, a revoked key, wrong pairing code |
| `forbidden` | 403 | route reserved for the server machine |
| `notFound` | 404 | unknown presentation, slide, look, … |
| `conflict` | 409 | not possible in the current state (e.g. `next` before any trigger), an `If-Match` mismatch, or a stored document from a newer server version |
| `liveShow` | 409 | something is live on the box that the request would take off the screens; send it again with `confirm: true` (see [Codex](#codex)) |
| `locked` | 423 | someone else holds the document's edit lock ([Presence and edit locks](#presence-and-edit-locks)); the message names them |
| `rateLimited` | 429 | too many pair attempts, uploads or upload chunks at once |
| `insufficientStorage` | 507 | a media upload would leave less disk space free than the server keeps (5 % plus 2 GiB) |
| `incompatibleProtocol` | 400 | client and server majors differ |
| `wsOnly` | 400 | command needs the WebSocket (previews) |
| `notImplemented` | 501 | defined in the protocol, not implemented by this server yet |
| `unavailable` | 503 | renderer gone or server shutting down; the Codex's media library isn't open (an upload's finish); on a show box, a content edit (or a search beyond its collection, or its assignment) while the site's Codex can't be reached |
| `codexElsewhere` | 409 | on a show box: uploads and Codex management belong to the box that holds the site's Codex; the body's `codex` (`CodexBox`) names it |
| `internal` | 500 | a bug or an I/O error |

## WebSocket

Connect to `ws://<host>:5555/api/v1/ws?protocolVersion=1&token=<key>`. Omit `token` on the server machine itself. The server closes the socket with code `4003` when the key is revoked while it is open (`4001`: an incompatible protocol version).

```text
client                                                     server
  ── connect ─────────────────────────────────────────────►
  ◄── {"type":"hello","server":{"id":"…","name":"Booth","version":"0.1.0","protocolVersion":1}}
  ◄── {"type":"state","revision":7,"state":{…}}
  ◄── (recent notices)
  ── {"type":"command","id":1,"command":{"type":"next"}} ─►
  ◄── {"type":"state","revision":8,"state":{…}}             (to every client)
  ◄── {"type":"result","id":1,"ok":true,"revision":8}       (to the sender)
```

- **Version check.** Send `protocolVersion` in the URL, or as the first message: `{"type":"hello","protocolVersion":1,"clientName":"Booth"}`. If the major differs, the server sends `{"type":"error","error":{"code":"incompatibleProtocol",…}}` and closes the socket with close code **4001**.
- **State.** The server sends the full `ShowState` on connect and again after every change, with a `revision` that only ever goes up. A slow client may skip revisions: only the newest state matters, and there are no diffs in v1. Screen frame counters and measured fps are refreshed with every state, and at least every 10 s while they change.
- **Results.** Every command gets a `result` with the same `id`, even one the server cannot read: a command `type` it does not know (a newer client) is answered `notImplemented`, bad fields `badRequest`. Only a message that is not a command with an `id` gets an id-less `{"type":"error",…}`. A successful result carries the `revision` of the state that includes the command's effect. That state is always sent *before* the result, so when the client reads a result, its state is already current. A command that changes nothing (e.g. `next` at the very end) returns the current revision.
- **Events.** `{"type":"documentChanged","kind":"presentation","id":"…","change":"changed"}` means: refetch that document over REST. The server sends it when its data is reloaded (`reloadData`), after every screen configuration edit (kinds `screen`, `outputBinding`, `look`, `mask`) and after every content edit (kinds `presentation`, `library`, `playlistNode`); `change` is `added`, `changed` or `removed`. Content edits add `revision` (the document's new revision), `clientId` (who made it: the edit's `clientId` or the `X-Client-Id` header) and, for edit ops, `ops` (the ops as applied), so a client holding the previous revision can patch its copy and a client can skip its own edits. A collection's documents carry `collection` (see [Codex](#codex)); `codexChanged` says which Codex lists to refetch.
- **Notices.** `{"type":"notice","level":"warning","message":"12 media files missing in 'Morning Light'","key":"missingMedia:…"}` carries something an operator should see. `level` is `info`, `warning` or `error`. Notices with a `key` are *rolling*: a later notice with the same key **replaces** the earlier one (clients update the toast instead of adding one), and a newly connected client only gets the newest of each key. Keys in use:

  | key | when |
  |---|---|
  | `missingMedia:<presentation id>` | a presentation with missing media files is opened (thumbnail prefetch) or triggered: one summary, once per presentation per data load |
  | `fonts` | the set of fonts not matched exactly grows: count plus the most used few |
  | `unsupported` | a new unsupported feature is met while rendering |
  | `notes` | a new render note (template not found, render failure, …) |
  | `lagged` | this client was too slow and missed events (`documentChanged`, `codexChanged`, …): it reloads what it shows (lists, the open documents) as after a reconnect |

  Output errors and load problems have no key. The full lists are always in `GET /api/v1/diagnostics`.
- **Heartbeat.** `{"type":"ping","id":5}` is answered at once with `{"type":"pong","id":5}`. Browsers cannot see WebSocket ping frames, so clients use this to notice a dead link.
- **Ordering.** Each connection's commands run in the order sent. Commands from all clients go through a single owner of the show state and are applied one at a time.
- The server sends WebSocket pings every 20 s and drops a client it has heard nothing from (not even the pong every WebSocket client answers a ping with) for 60 s. Pings go out ahead of everything else, so a client busy receiving previews on a slow link still gets them. It never accepts binary frames from clients.

### Commands

| `type` | fields | |
|---|---|---|
| `triggerSlide` | `presentation`, `slide` (ID) **or** `index`, `playlistItem?: {playlist, item}`, `step?`, `transition?` | slide goes live; its background media too (image or video). `step` picks the occurrence of a repeated slide (its place in the play order, see Navigation). `transition` overrides the slide's / presentation's for this change, see [Transitions](#transitions) |
| `next`, `prev` | | next/previous enabled slide (see below) |
| `triggerPlaylistItem` | `playlist`, `item` | a presentation starts at its first enabled slide; media goes to the media layer; an audio file goes to the audio layer (and, from an audio playlist, the playlist plays on: see [Audio](#audio)) |
| `clearLayer` | `layer` (`slide`, `media`, `announcements`, …) | |
| `restoreLayer` | `layer` (`media`, `slide`, `announcements`, `props`, `messages`, `audio`) | brings back what the last clear took off the layer while it is still empty (`state.restorableLayers` lists those layers): `clearLayer`, `clearAll`, clear groups, clear actions and `mediaStop` keep it; new content on the layer drops it. Slides come back without running their actions (the navigation position goes back to them); videos and audio start again from their in point; what was deleted since is left out. `conflict` when the layer is not empty or has nothing kept |
| `clearAll` | | every layer |
| `clearGroup` | `id` | runs a clear group's targets |
| `setLook` | `look` (ID or `null` = every layer everywhere) | The choice (also a slide's look action) is kept on the box: after a restart that look is live again, `null` included; a look deleted since starts as every layer. Before any choice: `ShowSettings.startupLook`, else every layer. |
| `triggerMedia` | `asset`, `layer?`, `transition?` | an image or video goes to the media layer, a foreground video to the slide layer (`layer`: `media` / `slide`; omitted = the bin item's layer), with the display and playback settings of the first bin item holding the asset. Triggering the video already playing there follows its retrigger setting (`automatic`: keeps playing unless it ended; `always`: restarts; `never`: ignored). An audio file goes to the audio layer (`layer` omitted or `audio`), cross-fading with what played there |
| `setScreensEnabled` | `role` (`audience` / `stage`), `enabled` | switches every screen of that role on or off. A disabled screen keeps its outputs running and sends black (transparent on screens with alpha). Layers stay live underneath |
| `identifyScreens` | `enabled` | draws a large label (screen name, role, size, rate, outputs) over every running screen, above everything, until turned off or for 60 s. Shown in `state.identify` |
| `setTestPattern` | `screen`, `pattern` (`null` = off) | replaces the screen's picture with a test pattern: `{"type":"colorBars"}` (75 % SMPTE-style), `grid` (grid, circle, centre cross, 93 % / 90 % safe areas), `gradient` (ramps), `{"type":"solid","color":{r,g,b,a}}`, `alphaChecker` (opaque / transparent squares and alpha ramps, for key outputs), `motion` (a bar crossing once a second and a binary frame counter, new every frame). Shown in `state.screens[].testPattern`; not stored |
| `setOption` | `continuousPlaylist?: bool` | show options; only the fields given change. Not persisted: a restart goes back to the defaults |
| `setClockFormat` | `use24Hour?: bool`, `seconds?: bool` | the box-wide clock format (`state.clockFormat`); only the fields given change. Clock data links and clock message tokens that don't set `use24Hour` follow it, and a short time gets seconds with `seconds`. Kept on the box across restarts (`machine/clock-format.json`) |
| `subscribePreview` | `screen`, `fps` (≤ 15), `width` (≤ 1920) | WebSocket only, see [Live preview](#live-preview) |
| `unsubscribePreview` | `screen` (ID or `null` = all) | WebSocket only |
| `prefetchThumbnails` | `presentation`, `width?` | warms the thumbnail cache |
| `reloadData` | | rereads the data folder and sends `documentChanged`; changed screens and output bindings are applied live |
| `timerStart`, `timerStop`, `timerReset` | `timer` | start / resume (an expired timer starts over), pause, back to the configuration (stopped) |
| `timerSet` | `timer`, `seconds` **or** `timeOfDay` | sets the current value (not stored); `timeOfDay` (seconds after local midnight): a countdown-to-time timer counts to it, any other timer gets the seconds until it |
| `timerAdjust` | `timer`, `deltaSeconds` | adds (or subtracts) seconds, in any state |
| `propShow`, `propHide`, `propToggle` | `prop` | props stack on the props layer, each dissolving with its own transition (else the layer default) |
| `triggerMacro` | `macro` | runs a macro's enabled actions in order, as a slide's macro action does (media actions go to their layer, nested macros at most 4 deep, missing targets skipped and counted in `state.diagnostics.skippedActions`); `notFound` for an unknown macro |
| `messageShow` | `message`, `tokens?: [{name, value}]` | shows a message with token values (defaults for the rest); showing one that is up brings it to the top with the new values |
| `messageHide` | `message?` | hides one message, or every message |
| `setStageLayout` | `screen`, `layout` (`null` = the default) | the layout a stage screen shows; stored in the screen document (`kind.layout`). See [Stage display](#stage-display) |
| `stageMessage` | `text`, `flash?: bool` | a message to the performers on every stage screen (never on audience screens); `flash` flashes it for 4 s |
| `stageMessageClear` | | takes the stage message down |
| `mediaPlay`, `mediaPause`, `mediaToggle`, `mediaRestart`, `mediaStop` | `layer?` | transport of a video or the audio layer (see below); `conflict` when nothing is live |
| `mediaSetLoop` | `loop: bool`, `layer?` | loop on or off for what plays (`state.transport.loop`): on, it plays again and again; off, it plays to its out point and ends (its end behaviour applies). It goes on from where it is, paused or not; a video that ended plays again. `conflict` when nothing plays |
| `audioNext`, `audioPrev` | | the next / previous item of the audio playlist playing (`audioPrev` restarts the item once it has played 3 s) |
| `setAudioOptions` | `shuffle?`, `repeat?` | audio playlist options (not stored) |
| `setVolume` | `target` (`master` / `layer` / `output`), `layer?`, `output?`, `db` | a level, dB (−96 … +12). `layer`: `audio`, `media` (the background video's sound), `slide` (a foreground video's and the live slide's video elements') or `videoInput` (the live source's sound); `output`: an output binding ID (its embedded audio). Not stored |
| `setMute` | `target`, `layer?`, `output?`, `muted` | mutes the master bus, a layer's sound or an output's embedded audio |
| `playTestTone` | `frequency?` (Hz, default 1000), `seconds?` (default 2, ≤ 30) | a −18 dBFS sine on the master bus |
| `mediaSeek`, `mediaSkip` | `seconds`, `layer?` | jump to a position (from the in point) / by ± seconds |
| `triggerVideoInput` | `input` | puts a logical video input live through this box's `InputBinding` (see Live sources); `notFound` for an unknown input, `conflict` for one not bound here (or bound to a capture device) |
| `triggerLiveSource` | `source` (`HOST (name)` from `GET /sources`, or `ip:port`; an OMT source `omt:HOST (name)` / `omt:host:port`), `transition?` | puts a live network source on the video-input layer (see [Live sources](#live-sources)) |
| `setTransitionOverride` | `layer`, `transition` (`null` = back to the show default) | replaces a visual layer's default transition until set back (not stored) |

**Media transport.** Transport commands act on the video of `layer` (`media` = background, `slide` = foreground video), on the audio layer (`audio`) or, without `layer`, on `state.transport`: the foreground video when one is up, else the background video, else the audio layer. `mediaPlay` on a video that ended plays it again from the start; `mediaStop` clears its layer (dissolving over the video's fade-out when it has one). Positions count from the start of the played range (the trigger's in point); `state.transport.position` is refreshed at most 4 times a second while a video plays, so a scrub bar should advance on the local clock in between.

<a id="audio"></a>**Audio.** Everything audible is mixed at 48 kHz into one master bus: the background video's soundtrack, a foreground video's soundtrack, the live slide's video elements (on the slide layer's level, each at its fill's `audio`), the sound of the live source on the video-input layer (on the `videoInput` layer level; resampled to 48 kHz with drift correction and paired with its pictures by timecode: within a frame of them, later by a sound card's latency) and the audio layer, each at its trigger's level and channel routing (`AudioSettings`) and its layer's level (`setVolume`). The bus plays on the machine's audio device (`PUT /api/v1/audio-device`) and is embedded into every IDN output whose binding has `embedAudio`, with the same timecodes as the video. Without a usable device a null output keeps the audio clock running (`state.audio.outputState` `null` / `error`). A video with sound follows its sound (the audio clock is the video's clock), so picture and sound stay within a frame of each other across loops, seeks and pauses. The audio layer plays one file at a time; a new one cross-fades with the old (its fade-in, else the show's default media fade). Items of an **audio playlist** play one after another (in a shuffled order with `shuffle`; starting over after the last with `repeat`); `state.audio.playlist` names the item playing. **Meters:** `{"type":"audioLevels","levels":[-12,-14],"xruns":0}` messages carry the master bus peak per channel (dBFS, whole dB, `-60` = silence) at most 10 times a second, only while a value changes.

<a id="transitions"></a>**Transitions.** A layer change plays the most specific transition set: the command's `transition` → the slide's → the presentation's (slide layer) or the media trigger's own (media layer) → the layer's `setTransitionOverride` → the show default per layer (`ShowSettings.defaultTransitions`; built in: 0.5 s dissolve). A `Transition` is `{kind, duration?, easing?}`. Rendered kinds: `cut`, `dissolve`, `push {direction}` (both sides move in `direction`), `slide {direction}` (the new content slides in over the old, which stays), `wipe {direction}` (a hard edge moving in `direction` reveals the new content; a diagonal grows from the opposite corner, `center` opens a box from the middle) and `zoom {direction?: "in" | "out"}` (scales about the centre while cross-fading). Directions are `left`, `right`, `up`, `down`, `upLeft`, `upRight`, `downLeft`, `downRight`, `center` (push and slide treat `center` as `left`). `easing`: `linear` (default), `easeIn`, `easeOut`, `easeInOut` (cubic). Vendor and unknown kinds (and fade-through-colour for now) play as a dissolve of the same length; servers before batch 8 read the new kinds as unknown (a dissolve) and ignore `easing`. Dissolves blend in linear light; push, slide and wipe never overlap their two sides (a transparent slide never shows the other through it); every kind applies to every visual layer (media, slide, announcements, props, messages). Triggering again mid-transition starts the next one from the picture on screen. Making a look live plays the look's own `transition` (any of these kinds, with its easing) on the layers whose picture it changes (none: a cut; see [Looks](#looks)). Content that changes otherwise (an edit of the live slide, a reload) cuts. `state.transitions` lists transitions in progress as of each state (progress is not streamed).

<a id="timers"></a>**Timers.** Kinds: `countdown` (`seconds`), `countdownToTime` (`secondsAfterMidnight` read with its `period`: `am` / `pm` turn a 12-hour value into 24-hour; counts to the next occurrence, tomorrow's when the time has passed; while stopped it shows the time left from now), `elapsed` (`start`, optional `end`). States: `stopped`, `running`, `paused`, `overrun` (a countdown with `allowOverrun` below zero, still counting), `expired` (holds 0, or `end`). `state.timers[]` carries `{id, name, kind, allowOverrun, state, currentSeconds, display, warningSeconds}` (`display` in the default `h:mm:ss` format, hours hidden while zero; countdowns round up, elapsed timers down; `warningSeconds`: the timer document's "turn amber with this much time left", `null` = none). A change of a timer's state is broadcast at once (within 0.2 s of expiry); while a timer counts, `{"type":"timerTick","timers":[{id, state, currentSeconds, display}]}` messages carry the values of the counting timers once a second (not more often); a stopped `countdownToTime` timer counts too (its value follows the clock). Set and adjust values are kept within ±99 hours. Adjusting a stopped `countdownToTime` timer moves its target, so it keeps following the clock.

<a id="actions"></a>**Slide actions.** Triggering a slide (`triggerSlide`, `next` / `prev`, a playlist item) applies its media, then runs its other enabled actions in list order: timer (start / stop / reset / restart / set / adjust), message (show with token values / hide), prop (show / hide), look, clear layer, clear group, stage layout (recorded in `state.stageLayouts`: the stage screens switch to it) and macro (its actions, nested at most 4 deep). Disabled actions are skipped; kinds the server cannot run (video input, trigger, unsupported or unknown imported kinds) and actions whose target is missing are skipped and counted in `state.diagnostics.skippedActions`. Action delays are not honoured yet.

**Navigation.** `next` and `prev` walk the live presentation in its **play order** and skip disabled slides. The play order is the arrangement of the playlist item the slide was triggered from (`setItem {arrangement}`), else the presentation's selected arrangement (`setArrangements {selected}`), else document order: repeated sections play again, sections the arrangement leaves out are skipped (an arrangement that plays nothing counts as document order). The server remembers which occurrence of a repeated slide is live; `triggerSlide` takes the slide's first occurrence at or after the current position in the same presentation and playlist item (wrapping), else its first one, so clicking a chorus after the second verse continues with the second chorus; with `step` (0-based place in the play order) it takes exactly that occurrence (ignored when that step holds another slide, e.g. after an edit). Every `SlidePosition` (`state.layers.slide`, `.announcements`, `state.cursor`) carries its `step` (absent when the slide isn't in the play order, or was deleted while live; in document order it equals `slideIndex`), so a client can mark the live occurrence, not every copy of the slide. A slide the arrangement leaves out (triggered by hand) sits just before the first entry that comes after it in document order: `next` goes there, `prev` to the entry before. If the slide was triggered from a playlist (`playlistItem`, or `triggerPlaylistItem`), `next` past the last slide moves on to the first enabled slide of the next enabled presentation item in that playlist, unless the `continuousPlaylist` option is off (`setOption`): then `next` stops at the last slide. Headers, placeholders and media items are skipped. `prev` before the first slide goes to the *last* enabled slide of the previous presentation item. `state.cursor` always shows what `next` would trigger, or `null` when there is nothing left.

Every command also works as `POST /api/v1/commands` with the same JSON, which suits Stream Deck and Companion buttons:

```sh
curl -X POST http://booth:5555/api/v1/commands -H "Authorization: Bearer $TOKEN" \
     -H 'Content-Type: application/json' -d '{"type":"next"}'
# {"ok":true,"revision":42}
```

### ShowState

```jsonc
{
  "layers": {
    "videoInput": {"name": "Cam 1", "source": "BOOTH (Cam 1)"},  // or null; status in sourcesChanged
    "media": {"asset": "…", "name": "Clouds", "kind": "video", "online": true,
              "playback": {"state": "playing", "position": 12.25, "duration": 30.0, "loop": true,
                           "rate": 1.0, "hw": true}},  // stills: playing, 0, null
    "foreground": null,                       // a foreground video on the slide layer
    "slide": {"presentation": "…", "presentationName": "Morning Light", "slide": "…", "slideIndex": 3,
              "section": "…", "sectionName": "Chorus", "playlist": {"playlist": "…", "item": "…"},
              "step": 5},                     // place in the play order (the second chorus)
    "announcements": null,
    "props": [{"prop": "…", "name": "Logo"}],  // in the order shown; later ones on top
    "messages": [{"message": "…", "name": "Nursery", "text": "Parent of A42, please come",
                  "tokens": [{"name": "child", "value": "A42"}], "visible": true}],  // the last one is drawn
    "audio": {"asset": "…", "name": "Prelude", "kind": "audio", "online": true,
              "playback": {"state": "playing", "position": 31.5, "duration": 184.2, "loop": false,
                           "rate": 1.0, "hw": false}}   // the audio layer
  },
  "activeLook": "…",                          // null = every layer on every screen
  "transitions": [{"layer": "slide", "kind": {"type": "dissolve"}, "progress": 0.1, "duration": 0.5}],
  "transport": {"layer": "media", "asset": "…", "name": "Clouds", "state": "playing", "position": 12.25,
                "duration": 30.0, "loop": true, "rate": 1.0, "hw": true},  // videos first, else the audio layer
  "timers": [{"id": "…", "name": "Countdown", "kind": {"type": "countdown", "seconds": 300},
              "allowOverrun": false, "state": "running", "currentSeconds": 287.25, "display": "04:48"}],
  "stageLayouts": [{"screen": null, "layout": "…"}],   // chosen by stage layout actions / setStageLayout
  "stageScreens": [{"screen": "…", "name": "Stage", "layout": "…", "layoutName": "Current / Next"}],
                                                       // every stage screen and the layout it shows (null = built-in)
  "stageMessage": {"text": "Wrap up", "flash": true}, // null = none
  "screens": [{"id": "…", "name": "Audience", "role": "audience", "enabled": true, "width": 1920, "height": 1080,
               "frameRate": {"num": 30, "den": 1}, "measuredFps": 30.0, "lastRenderMs": 12.5,
               "testPattern": null,                    // setTestPattern
               "outputs": [{"name": "IDN: Audience", "kind": "idn", "origin": "binding", "binding": "…",
                            "width": 1920, "height": 1080, "frameRate": {"num": 30, "den": 1}, "delayFrames": 0,
                            "status": "ok", "detail": null, "framesSent": 9000, "framesDropped": 0, "errors": 0,
                            "embedAudio": true, "audioDb": 0.0, "audioMuted": false}]}],
                                                       // running (enabled) screens only
  "screensEnabled": {"audience": true, "stage": true},    // setScreensEnabled
  "identify": false,                                      // identifyScreens
  "options": {"continuousPlaylist": true},                // setOption
  "clockFormat": {"use24Hour": false, "seconds": false},  // setClockFormat
  "displays": [{"id": "HDMI-A-1", "index": 0, "name": "Example Monitor", "connected": true,
                "modes": [{"width": 1920, "height": 1080, "refreshMhz": 60000}],
                "preferred": {"width": 1920, "height": 1080, "refreshMhz": 60000}, "boundTo": ["…"]}],
  "cursor": {"presentation": "…", "slideIndex": 4, …},     // what `next` would trigger
  "build": {"layer": "slide", "presentation": "…", "slide": "…", "step": 1, "steps": 3},
                                                          // the live slide's object builds (null: none)
  "captions": {"enabled": true, "status": "running",      // off / starting / downloading / running / error
               "detail": null, "progress": null,          // why / what it waits for; download %
               "downloaded": null, "downloadSize": null,  // download bytes so far / size (if known)
               "model": "base.en", "device": "CPU (6 threads)", "load": 0.34,
               "lines": [{"line": 4, "text": "Welcome, everyone.", "final": false}]},
                                                          // live captions (see Live captions)
  "restorableLayers": ["slide"],                          // empty layers restoreLayer can bring back
  "diagnostics": {"fontSubstitutions": 1, "fontMisses": 0, "missingMedia": 0, "unsupported": 2,
                  "notes": 0, "loadIssues": 0, "skippedActions": 0},
  "audio": {"device": "hw:1,0", "outputState": "ok", "detail": null,   // null / error: null output
            "masterDb": 0.0, "muted": false, "levels": [-12, -14], "xruns": 0,
            "layers": {"audio": {"db": 0.0, "muted": false}, "media": {"db": 0.0, "muted": false},
                       "slide": {"db": 0.0, "muted": false}, "videoInput": {"db": 0.0, "muted": false}},
            "shuffle": false, "repeat": false, "playlist": {"playlist": "…", "item": "…"}}
}
```

## Screen configuration

The server is headless: screens and their outputs are set up through the API (the control app's Screen Configuration dialog). Changes are saved and applied **live**: no restart, and screens and outputs that a change does not touch keep running without dropping a frame.

- **Documents.** A screen is a portable `Screen` (`portable/screen/<id>.json`): `name`, `kind` (`{"type":"audience"}` / `{"type":"stage"}`), `size` (whole, even pixels, each side 64–8192, at most 7680 × 4320 in all), `frameRate` (rational, 1–120 fps: 24000/1001, 24, 25, 30000/1001, 30, 50, 60000/1001, 60), `alpha` (`opaque` / `straight` / `premultiplied`), `background` (colour; transparent = none), `enabled` (a disabled screen keeps its settings but does not run), `sortKey`. An output is a machine-local `OutputBinding` (`machine/outputBinding/<id>.json`): `screen`, `name`, `transport` (`{"type":"idn","sourceName":"…","groups":[…]}` for an IDN Open Protocol output, compatible with NDI®; `{"type":"omt","name":"…"}` for an OMT (Open Media Transport) source `HOST (name)`, with alpha and `embedAudio` like IDN (names unique among the OMT outputs); `{"type":"display","selector":{"connector":"HDMI-A-1","name":"…"},"mode":{"width":1920,"height":1080,"refreshMhz":60000}}` shows the screen full screen on an attached display (see below); `sdi` is not supported yet; `{"type":"placeholder"}` holds the slot of an output whose hardware isn't there yet), `enabled`, `resolutionOverride` / `frameRateOverride` (absent = follow the screen), `alphaOverride`, `videoDelay` (0–30 frames), `target` (`full`, or `fill` / `key` for a Fill/Key pair, see below), `tile` / `region` (the part of the screen it shows, see Screen layouts below). Fields at their default are omitted, as everywhere in the model.
- **Colour correction and corner pin** (`Screen.color`, `Screen.cornerPin`; capability `screenAdjust`) adjust the screen's whole picture after compositing and before it goes to any output (IDN, displays, PNG), so every output of the screen gets them. `color` is `{brightness (−1…1, 0), contrast (0…2, 1), gamma (0.1…4, 1), saturation (0…2, 1), red, green, blue (gains 0…2, 1), blackLevel (−0.5…0.5, 0)}`, applied to the sRGB-encoded straight colour in the order gains, saturation (Rec. 709 luma), contrast about mid grey, brightness, black level (`b + c·(1−b)`), clamp, gamma (`c^(1/gamma)`); alpha is never changed. `cornerPin` is `{enabled, topLeft, topRight, bottomRight, bottomLeft}`, each `{x, y}` in screen pixels, **positive = away from the screen's centre**: the picture is warped in perspective to the quad the moved corners make; outside it the output is black (opaque screens) or transparent (screens with alpha). Offsets are at most the screen's width / height and, while enabled, the corners must make a convex shape (`400` with `field` `color.<name>`, `cornerPin.<corner>` or `cornerPin`). Defaults are not stored; settings that change nothing cost nothing (frames bit for bit as without them). Change them with the full `PUT /screens/{id}` or with the narrow `PUT /screens/{id}/color` / `/corner-pin` (no `If-Match`: meant for sliders); either way the next frame has them and no output restarts.
- **Create** with `POST` and a partial document: the server fills in the ID and defaults (a screen gets "Screen N", 1920 × 1080 at 30 fps, opaque, last in order; an output's `name` defaults to its source name).
- **Optimistic concurrency.** Every entry carries a `revision` (a content hash; also the `ETag`). Send it back as `If-Match: "<revision>"` with `PUT` and `DELETE`. If someone changed or deleted the document meanwhile, the answer is `409 conflict` with `current` (the document now, or `null`), so a UI can offer *Reload* or *Overwrite* (resend with `current.revision`). Without `If-Match` the write is unconditional.
- **Validation** happens before anything is written; errors are `400` with `error.field`. IDN source names must be unique among the server's IDN outputs (case-insensitive: receivers list them as `"HOST (name)"`), names non-empty (at most 64 characters).
- **Schema 2 (migration).** Schema-1 documents with the old `ndi` transport (output bindings) or source (input bindings) are read as `idn` with the same fields, in memory, when the server loads its data folder; the files are rewritten only when an operator edits them. A request body with `"schemaVersion": 1` (or none) is migrated the same way.
- **Schema 3 (media library locations).** A document holding a media asset location in the Codex's media library (`{"type": "library", id, sha256}`: a media asset, or a trash entry with one) is written with `"schemaVersion": 3`; every other document stays at 2, so older servers keep reading it. A server before schema 3 refuses (and never overwrites) a schema-3 document, instead of reading the location as unknown and losing it on its next save. A document that no longer holds one is written as 2 again.
- **Persistence** is atomic (temporary file, fsync, rename) and never overwrites a document written by a newer server version (`409`).
- **Deleting a screen** deletes its output bindings and removes it from every look that lists it (`ScreenDeleted` says which). A new screen needs no look changes: a look that does not list a screen shows every layer on it.
- **Screen layouts** (capability `screenLayouts`). `Screen.arrangement` (absent = Single) is `{kind, columns, rows, bezel, overlap, blend}`: `kind` `single` / `mirror` (every output shows the whole screen; the two differ only in intent), `grouped` (the screen spans a grid of displays) or `edgeBlend` (it spans overlapping projectors); `columns`, `rows` 1–8 (1 × 1 unless grouped / edge blend); `bezel` `{x, y}` (grouped: canvas px hidden behind the frames between two tiles, 0–2000); `overlap` `{x, y}` (edge blend: canvas px two neighbours both show, smaller than a tile); `blend` `{gamma (1–3, 2.2), curve (1–4, 2)}`. The screen's `size` is the whole wall: a tile is `(size − (n−1)·gap) / n`, `gap` = bezel or −overlap, at least 16 px (`400`, `field` `arrangement.<name>`). An output's machine-local `tile` (row-major index, `0` = top left) picks the tile it shows: the worker crops it, scales it to the output's size (its `resolutionOverride`, else the tile's size, which `OutputState.width/height` report) and, for edge blend, fades every inner edge over the overlap: at `t` (0 at the tile's edge, 1 at the overlap's far side) the light is `½(2t)^curve` below the middle and `1 − ½(2(1−t))^curve` above, so two neighbours add up to full light, and the sRGB value is multiplied by `light^(1/gamma)`; corners multiply; alpha is untouched. An output's `region` (`{x, y, width, height}` canvas px, inside the screen, at least 16 px) is an explicit crop that wins over the tile's. A `tile` is checked when it is set or changed (`400` `field` `tile`: the screen isn't grouped / edge blend, or has fewer tiles); a screen re-arranged to fewer tiles later leaves that output `unavailable` ("tile 4 is outside the 2×1 arrangement"). A `{"type":"placeholder"}` transport is an output slot without hardware yet: stored, listed with `kind` `placeholder`, `status` `unavailable` and `detail` "No hardware yet", never run. Changing a screen's arrangement or size rebuilds only the outputs whose crop changed.
- **Display outputs** (capability `displays`). `selector.connector` is the `id` of a display in `GET /api/v1/displays` (`selector.name`, the monitor's model, finds it again if it moves to another connector); `mode` is one of that display's `modes` (absent = its preferred mode; a mode it doesn't offer falls back to the preferred one). A display serves one output (`badRequest` on `transport.selector` otherwise; `choose a display` without a selector). The screen is scaled to the mode, letterboxed. A display that isn't plugged in keeps its binding and shows `unavailable` (“HDMI-A-1 is not connected”) until it is plugged back in, then starts by itself. On Linux the server drives the displays directly (kernel mode setting, no desktop); with a desktop session on the same graphics card the output is `unavailable`. `GET /api/v1/output-transports` reports `display` as available with its backend in `status` (not available, "no desktop session", for a Windows server running in session 0 as the default startup task). Name default: the monitor's model, else the connector.
- **Fill/Key pairs** (capability `fillKey`). For a hardware keyer, a screen with alpha is sent as two outputs of that screen: one with `target: "fill"` (the colour over black: straight, i.e. unshaped, or premultiplied, i.e. shaped, when the output's alpha type, its `alphaOverride` else the screen's `alpha`, is `premultiplied`) and one with `target: "key"` (the alpha as a grey picture, white = opaque). Both frames are opaque (no alpha is sent with them) and come from the same frame of the screen at the same tick; each output has its own display (or transport), status, counters and delay (keep the two the same). On displays driven directly (Linux, kernel mode setting) the fill and key of one screen on one graphics card are page-flipped together; two connectors are not guaranteed to scan out in step, so the key can be up to one refresh apart from the fill on a change. An opaque screen's key is solid white. IDN outputs carry alpha themselves (prefer one output with `straight` alpha), but a `key` IDN output is allowed. Output names in the state end with ` (fill)` / ` (key)`. An unknown `target` is `400` with `field` `target`.
- **Status** of each output is in `state.screens[].outputs[]`, matched by `binding` (the output binding ID). An output that cannot run (for example a transport that is not available) keeps its configuration and shows `unavailable` with the reason; `GET /api/v1/output-transports` says up front which transports work on this server.
- **Video delay** keeps up to `videoDelay + 1` frame references per output; memory grows only with frames that actually differ (at most about 8.3 MB per 1080p frame, ≈ 257 MB for 30 frames of constantly changing 1080p).

## Looks

A look (`Look`, a box document like screens) decides per **audience** screen which layers are drawn (`screens[].layers`: a missing field means shown), whether the slide's text is re-rendered through a theme's template slide (`templateOverride`), which mask is drawn on top (`mask`) and how a slide's second language is shown (`language`, batch 16: `auto`, `primary`, `second`, `stacked`, `sideBySide`; see [Dual-language slides](#dual-language)). A screen the look doesn't list shows every layer. Capability `lookEditing`:

- `POST /looks` (partial body), `PUT /looks/{id}` (the whole look, `If-Match`), `POST /looks/{id}/duplicate`, `DELETE /looks/{id}` (`If-Match`). Each answers a `LookEntry` (`201` for create and duplicate, `204` for delete) and sends `documentChanged {kind: "look"}`. Reorder with a `PUT` carrying a new `sortKey` (`GET /looks` is sorted by it).
- Validation (`400` with `field`): a name (unique, case-insensitive, at most 64 characters); entries only for existing audience screens (`screens.N`: a stage screen or an unknown one is refused, unless the stored look already had that very entry), each screen once; a theme override names an existing theme and one of its template slides (`screens.N.templateOverride`); `transition` lasts 0–10 s. Entries that change nothing are dropped.
- **Live look.** The live look is a saved look: a saved edit of it changes the screens at once and plays its transition. It can't be deleted (`409`: make another look live first). Slide actions naming a deleted look are skipped (`state.diagnostics.skippedActions`).
- **Transition.** `setLook` (and a slide's look action) plays the look's `transition` on every layer whose picture changes because of the look, using the same transition kinds as layer changes; a look without one cuts. A layer change at the same moment plays its own transition.
- **Masks** (capability `masks`). A `Mask` is a slide (`size` = its canvas; shapes, pictures, text: ordinary elements) drawn over a screen **after all its layers**, scaled to the screen: `mode: "overlay"` (default) draws it as authored; `mode: "keyhole"` shows the screen only through the slide's shapes (their alpha), everywhere else is `color` (default opaque black; a see-through colour cuts the picture out to alpha: with `a: 0` the screen is transparent outside the shapes, which an output with alpha carries); `feather` (canvas px, 0–200) softens the whole mask's edge. A look names one per screen (`screens[].mask`, which must exist). `GET`, `POST /masks`, `GET`, `PUT`, `DELETE /masks/{id}`, `POST …/duplicate` work like looks (`MaskEntry`, `DuplicateLook` body, `documentChanged {kind: "mask"}`). A **stage screen** takes its own mask, `Screen.mask` (set with `PUT /screens/{id}`; `400`, `field: mask`, on an audience screen or for a mask that doesn't exist), drawn over its layout the same way. Deleting a mask that looks or stage screens use is refused (`409`, `current: {looks, screens}`) unless `?detach=true`: they forget it in the same write (`200 MaskDeleted {looks, screens}`). A mask change cuts.

## Editing content

Presentations and playlists are edited with **id-addressed operations**, not whole-document `PUT`s, so two operators editing different slides never conflict.

```http
POST /api/v1/presentations/{id}/edits
{"clientId": "tab-7f3a", "baseRevision": "9c1e…", "ops": [
  {"type": "setSlideText", "slide": "…", "text": "Morning light\nover the quiet hill", "expectText": "Morning light"},
  {"type": "splitSlide", "slide": "…", "atParagraph": 1}
]}

200 {"revision": "4b7d…", "applied": [ …primitive ops… ], "inverse": [ …ops that undo the batch… ]}
```

- Ops apply to the **current** revision (`baseRevision` is informational). With `If-Match: "<revision>"` the batch applies only when the presentation is still at that revision (else `409 conflict` with `current: {revision}`). A batch is all or nothing. It fails with `409 conflict` only when a target no longer exists or a precondition fails (`expectText`); `current` is an `EditConflict` `{revision, opIndex, slide?, element?, text?}` (for `expectText`: the element's text now). Malformed ops (duplicate IDs, unknown fields) are `400`.
- `applied` lists primitive ops: `duplicateSlide` becomes `addSlide`; `splitSlide` becomes `replaceTextBox` + `addSlide` (+ `setSlideTranslation` when the primary text has a second-language text); `mergeSlides` becomes `replaceTextBox` + `deleteSlide` (+ `setSlideTranslation` before the delete when the second slide's primary text has one); `setTextStyle` with a range becomes `replaceTextBox`; `addSlide` without a slide gets the server-made slide; `setSlideText` without `element` gets the primary text element (or `addElement` + `setSlideText` when the slide has no text).
- `inverse` undoes the batch exactly (only `modifiedAt` differs). Clients keep their own undo stack: undo = send `inverse`; the answer's `inverse` is the redo.
- Patch fields: omitted = unchanged; for model fields that are optional, `null` clears. `after` names the sibling to insert / move after; omitted = first.

| op | fields |
|---|---|
| `setPresentation` | `name?`, `songInfo?`, `scripture?`, `transition?`, `canvasSize?`, `notes?`, `category?`, `language?` (the presentation's own language, see below; `null` clears) |
| `addSection`, `deleteSection`, `moveSection`, `setSection` | `{after?, section}`, `{section}`, `{section, after?}`, `{section, name?, color?, hotkey?, globalGroup?}` |
| `addSlide`, `deleteSlide`, `moveSlide`, `duplicateSlide`, `setSlide` | `{section, after?, slide?}`, `{slide}`, `{slide, toSection, after?}`, `{slide}`, `{slide, label?, enabled?, notes?, transition?, background?, hotkey?, builds?}` (`builds`: the whole build order, see [Object builds](#object-builds)) |
| `setSlideText` | `{slide, element?, text, expectText?}`: plain text, `\n` between paragraphs |
| `splitSlide`, `mergeSlides` | `{slide, atParagraph}` (Insert Slide Break), `{first, second}` |
| `addElement`, `deleteElement`, `reorderElement`, `setElement` | `{slide, element, index?}`, `{slide, element}`, `{slide, element, index}`, `{slide, element, patch}` (name, bounds, rotation, opacity, flip, hidden, locked, aspectLocked, fill, stroke, shadow, feather, scroll, placeholder: the slot role `{slot, accepts?}`, `null` clears it; `visibility`: the whole condition list, `[]` = always shown; `dataLink`: the live data link, `null` clears it) |
| `setTextStyle` | `{slide, element, range?, character?, paragraph?, clear?}`: without `range` the box defaults; `range` is `{start, end}` in Unicode characters of the plain text (`\n` counts one); `clear` lists field names to unset |
| `setTextBox`, `replaceTextBox` | `{slide, element, verticalAlign?, fit?, margins?, outline?, textShadow?, lineMask?}`, `{slide, element, textBox}` |
| `setArrangements` | `{arrangements, selected?}` |
| `applyTheme`, `resetToTheme`, `unlinkTheme`, `replaceSlide` | `{theme, template?, slides?}`, `{slide, element?, properties?}`, `{slides?}`, `{slide}` (see [Themes](#themes)) |
| `addSlideAction`, `removeSlideAction`, `moveSlideAction`, `setSlideAction` | `{slide, after?, action}`, `{slide, action}` (ID), `{slide, action, after?}`, `{slide, action}` (the whole new action; its `id` names the one replaced) |
| `setSlideTranslation` | `{slide, translation}`: the slide's whole second-language layer (`null` removes it; see [Dual-language slides](#dual-language)) |

**Scrolling text** (`Element.scroll`, batch 8): `{enabled?, direction?: "left" | "right" | "up" | "down", speed? (canvas px/s, 120), start?: "auto" | "offscreen" | "onscreen", repeat? (true), gap? (canvas px between copies, ink to ink; 120)}`. The text moves through its box on each screen's clock from when the slide went live (fill, stroke and shadow stay put), clipped to the box and drawn above the slide's other elements; `auto` starts on screen when the text fits the box, else just off it. Thumbnails and `/render` show it still, where it is laid out. Imported scrollers (`ticker` elements) fill it. `setElement` refuses (400) a `speed` outside 0–10000 px/s or a `gap` outside 0–50000 px.

**Video elements** (batch 8; sound batch 11): an element whose enabled media fill (`fill.kind: {type: "media", asset, display?, playback?, audio?}`) is a video asset plays it in its box on the screens from when the slide goes live. On the slide layer it plays with its sound (`audio`: `AudioSettings`, level / mute / routing, omitted = as is; rate 1 only) on the slide layer's level, one player for every screen, the picture following the sound; its sound fades with the slide layer's transition. On props, messages, announcements and a look's template override it plays muted. `playback.repeat` (loop, count, once holding the last frame), `inPoint` (start offset) and `outPoint` apply; `display` fits, fills or stretches it, clipped to the box. Until the video is open (off the render thread), the box shows nothing. Thumbnails and `/render` show no picture for it.

**Stroke styles** (`Stroke.style`): `solid`, `squareDash` (dotted), `shortDash`, `longDash`, `custom` (`pattern`) and, since batch 8, `double` (two lines a third of the width each, a third apart; drawn on rectangles, rounded rectangles and ellipses, solid on other shapes; older servers draw it solid).

**`setSlideText`** (Reflow) edits the given element, else the slide's **primary text element**: the first text element with a text placeholder slot, else the largest text box without a data link. A slide without one gets a new text element in the presentation's prevailing style (the nearest slide's primary text box, emptied). The box keeps `defaultStyle` and `defaultParagraph`. Old and new lines are matched by content: the unchanged lines at the start and at the end keep their paragraphs whole (runs such as superscripts and colours, style, chords); the changed lines between them take, in order, the styles of the old lines they replace (more new lines than old: the last replaced one's, else the neighbouring old paragraph's) and become a single run in the style of that old paragraph's first run.

<a id="dual-language"></a>**Dual-language slides** (batch 16, `model::language`). A slide can carry a second language: `Slide.translation` = `SlideTranslation {language?, texts?, layout?}`, where `language` is a BCP 47 code (`"es"`), `texts` holds `ElementTranslation {element, paragraphs?}` (the second text of a text element of the slide, paragraphs like a text box's; at most 64) and `layout` the slide's preferred `LanguageDisplay` where a screen says `auto` (a parallel Bible passage asks for `sideBySide`). It is a layer beside the elements: templates, linked themes, Reflow's primary text, builds and stage displays keep using the primary text; duplicating a slide or applying a theme keeps each second text with its element (a leftover merged into the template's last text slot takes its second text along). Set it with `setSlideTranslation {slide, translation}` (the whole layer; its inverse is the old layer). **Drawing:** each audience screen's look entry has `language` (`LanguageDisplay`): `auto` (default: the slide's `layout`, else `stacked`), `primary` (as authored), `second` (the second text in the primary's place and style), `stacked` (the second text under the primary, in the same box) or `sideBySide` (the box split in two, primary left, second right; the right half has a stable derived element ID, keeps the element's visibility conditions and builds with it). Stacked and side-by-side second text uses the slide's theme's `secondLanguage` style (`SecondLanguageStyle {sizeScale? (0.8), italic? (true), color? (pale gold), fontFamily?, gap? (24 canvas px)}`; the slide's `templateOrigin` theme, else its presentation's, else the defaults). A look's template override (lower thirds) restyles the text after the second language is added; side by side is drawn stacked there (a template takes one text per slot). Slides without a translation are drawn as before; older servers ignore the fields. <a id="presentation-language"></a>**A presentation's own language** (lane L, 2026-11-26): `Presentation.language` (optional) is the BCP 47 code of the language its text is written in (`"en"`, `"es"`, `"pt-BR"`); absent = not given (older files load without it). `setPresentation {language}` sets it (`null` clears): it is stored in its usual case (`pt_br` → `pt-BR`: language lower case, a 4-letter script title case, a 2-letter region upper case), and a value that is not a language code (a 2–8 letter language subtag, then subtags of 1–8 letters or digits, at most 35 characters) is `400`; a document created with `POST /presentations {document}` has such a value dropped. Imports fill it where the source says: ChordPro `{meta: language xx}`, a Bible passage (its translation's `language`), a PowerPoint deck (`dc:language` in its properties, else the language most of its slides' text is marked with, `a:rPr lang`, by characters); the ProPresenter importer leaves it unset (its documents carry no language). The ChordPro export writes `{meta: language xx}` when it is set. Oriel's Translate dialog takes it first as the source language. A slide's second language stays its own (`SlideTranslation.language`). `SlideTranslation.machine` (`MachineTranslation {engine, model}`) marks a kept machine translation ([Machine translation](#machine-translation)); clients drop it when someone edits the second text.

**`splitSlide`** moves the primary text's paragraphs from `atParagraph` on to a new slide inserted right after, a copy of the slide (same elements, geometry and styling; fresh IDs; no label, hotkey, notes or actions). The primary text's second-language text splits with it, by paragraph (at the same paragraph when both have as many, else in proportion): each slide keeps its part. **`mergeSlides`** appends the second slide's primary paragraphs, and their second-language text, to the first slide's.

**Live.** Editing the live slide (or its presentation) re-renders the screens at once. Thumbnails of edited slides get a new `ETag` (their content hash changed). A live slide that is **deleted** stays on screen until the next trigger or clear; `state.layers.slide` keeps naming it, and the cursor moves to the slide that took its place (`next` goes there).

**Playlist items** (`POST /api/v1/playlists/{id}/edits`, same rules): `addItem {after?, item}` (presentation, `header {name, color}`, media, …), `removeItem {item}`, `moveItem {item, after?}`, `setItem {item, arrangement?, enabled?, name?, color?}` (a header's `name` / `color` are its own).

**Whole documents** (libraries, presentations, playlists and folders) are created, renamed, moved and deleted with the routes in the REST table. Updates, moves and deletes take `If-Match: "<revision>"` (`409` with `current: {revision}` on a mismatch). A library is deleted only when empty. Deletes answer with the deleted documents; sending one back as `{"document": …}` to the create route restores it with its ID (undo). Every write is atomic and never replaces a document written by a newer server version.

### Media bins and global groups

Media bins (`MediaBin`) and global groups (`GlobalGroup`) are collection documents like playlists:
every write goes through the Codex writer (`documentChanged {kind: mediaBin | globalGroup,
collection}`, the change log, deletes into the item trash and restorable with their IDs) and is
forwarded to the Codex from a show box.

- **Bins.** A bin's `folder` is a path (`["Motion"]`); folders are not documents (a folder exists
  while a bin is in it). Order: `after` names a bin of the same folder and browser (media or
  audio); omitted = last on create, `null` = first on update. Moving to another folder without
  `after` puts the bin last there. Names and folder segments follow the content rules (trimmed,
  1–200 characters, no control characters); folders nest at most 8 deep.
- **Bin items** (`POST /api/v1/media-bins/{id}/edits`, the playlist rules): `addItem {after?,
  item: MediaBinItem {id, name?, enabled?, trigger: {asset, layer, …}}}` (the asset must be in
  the collection: `400`), `removeItem {item}`, `moveItem {item, after?}`, `setItem {item,
  enabled?, name?}` (`name: null` = the media's name). The answer has `applied` and `inverse`.
  A watched folder's bin refuses item edits (`400`): its items follow the folder.
- **Global groups**: `name`, `color`, `hotkey`, order. Sections linked to a group keep their own
  name and colour (their snapshot); editing or deleting the group doesn't rewrite songs.
- **Bin playlists** are playlists with `purpose: media` (or `audio`): the playlist routes.

## Live sources

Network video sources (IDN Open Protocol, compatible with NDI®, and OMT) shown on the
**video-input layer**, the bottom one (plan in `docs/LIVE-SOURCES-PLAN.md`, OMT in
`docs/OMT-PLAN.md`). Sources are per box, not in a collection.

- **OMT sources** are named `omt:` + the OMT name or address (`omt:STUDIO (Program)`,
  `omt:192.0.2.10:6400`; `omt://host:port` is taken as it is) wherever a source is named by a
  string: `triggerLiveSource`, a live fill's `source`, the thumbnail's `source`. They are listed
  with `omt: true` (mDNS lookups every 5 s; one missing from two lookups drops off; a site's
  discovery server lists IDN sources only). Pictures, alpha (straight, or premultiplied when the
  sender marks it), sound, status and thumbnails work as for IDN sources; a thumbnail proxy asks
  the source for previews (an eighth of each side; one picture in 15 kept), and a source without
  them sends its whole pictures. An OMT output whose alpha type is `premultiplied` sends its
  colour premultiplied and says so. An `InputBinding` source `{type: "omt",
  name}` binds a logical input to one.

- `GET /sources` → `{ sources: LiveSource[] }`: every source on the network (mDNS, or the
  site's discovery server while this box is enrolled in another server's site) and every source
  received by address. `LiveSource`: `name` (`HOST (name)`), `address?`, `idn` (an IDN device,
  else another compatible sender), `omt?` (an OMT source), `listed` (on the network now), `status` (`idle`,
  `connecting`, `live`, `lost`, `unsupported`), `width?`, `height?`, `frameRate?`, `message?`
  (why lost or unsupported). The registry starts on the first request.
- `sourcesChanged {sources}` on the WebSocket: the whole list after every change (a source
  appears or goes, a status or picture size changes).
- `GET /sources/thumbnail?source=…&w=…` (w 16–640, default 320): the newest picture as JPEG;
  `204` while it connects. A source not in use is read through its low-bandwidth stream, one
  picture in 15, until no thumbnail was asked for 10 s. Poll it every few seconds.
- `triggerLiveSource {source, transition?}` puts a source live; `clearLayer videoInput`,
  `clearAll`, clear groups and `restoreLayer videoInput` work as for media; looks hide it with
  `layers.videoInput: false`. `state.layers.videoInput` is `{input?, name, source}`.
- One receiver per source in use, shared by every screen, asking for pictures no larger than
  the largest screen. Screens draw its newest picture at their own rate. A lost source keeps
  reconnecting; its last picture holds 3 s, then the layer draws nothing until pictures come
  back.
- `triggerVideoInput {input}`: a logical input (`VideoInput` document of this box) goes live
  through its `InputBinding` (`source: {type: "idn", sourceName}` or `{type: "omt", name}`); the state's `input` and
  `name` (the input's name) are set. Playlist video-input items and slide video-input actions
  do the same with their display settings and transition.
- **Logical inputs and their bindings:** `GET /video-inputs` → `VideoInputEntry[]` (in order):
  the `VideoInput` (`id`, `name`, `sortKey`, `color?`, `alpha`, `audio`) plus `source?` (this
  box's binding, `{type: "idn", sourceName}` or `{type: "omt", name}`; absent = not bound here) and a `revision` of
  both. `POST /video-inputs` (partial; no name → "Camera N"; `source?`) → `201`; `GET`, `PUT`
  (the whole input plus `source`, absent or `null` unbinds; `If-Match`), `DELETE` (the input
  and its binding; `If-Match`) `/video-inputs/{id}`. Names 1–64 characters (trimmed), a
  source 1–512. `documentChanged {kind: "videoInput" | "inputBinding"}`. A live input follows
  a new binding at once and leaves the layer when unbound or deleted; uses of a deleted input
  (playlist items, fills) answer `notFound` when triggered. Visibility conditions "video
  input X is live" follow the layer (the input put live, or an input bound to the source put
  live by name).
- **A source as an element fill:** `FillKind` `{type: "liveSource", source?, input?, display?}`
  draws a source inside a slide element's box (a camera box), like a video fill. `input` (a
  logical input, resolved through this box's binding) wins over `source` (`HOST (name)` or
  `ip:port`). Older servers read the tag as `unknown` and draw nothing.
- **Alpha:** a source with an alpha plane (keyed graphics, another Vesper's alpha output)
  composites over what is below; `display.alpha` (and the input's `alpha`): `opaque` ignores
  it, `straight` / `premultiplied` override what the source says.
- Interlaced sources are woven (two fields → one frame); H.264 / HEVC sources (NDI|HX
  cameras) are decoded with the system FFmpeg (`unsupported` only where it has no decoder for
  the codec). Their sound, uncompressed or AAC (NDI|HX cameras, decoded with the same FFmpeg),
  plays on the program bus (the `videoInput` layer level), paired with the pictures by
  timecode.

## Remote API compatibility

Existing Stream Deck / Companion button profiles call the presentation remote REST API's
endpoints (`/version`, `/v1/...`). Vesper answers the commonly used ones on a port of its own,
**off by default**: `PUT /api/v1/remote-compat {"enabled": true, "port": 50001}` (Oriel:
Settings ▸ Network & IDN ▸ Control surfaces). The setting is kept on the box
(`machine/remote-compat.json`) and applied at once (changes sent at the same time are applied one after the other); a port
that can't be opened is reported in `problem` (with `listening: false`) and tried again at the
next change or start. Port `0` takes
a free one (`listeningPort`).

- **Auth is the API's:** requests from the server machine are trusted (same `Host` rule as
  `/api/v1`); every other client sends a paired site key as `Authorization: Bearer <key>` or
  `?token=<key>`, else `401`. Old profiles that send no key work only from the server machine.
  Its triggers are plain `GET`s, which any web page can send (`<img src=…>`), so a request a
  browser makes for a page (`Sec-Fetch-Site` other than `none`, or any `Origin` or `Referer`) is
  never the server machine here: it needs a key too. Button panels and scripts send none of
  these; a URL typed into the address bar (`Sec-Fetch-Site: none`) still works.
- Commands run like `POST /api/v1/commands`; triggers answer `204`, errors the API's JSON error
  with its status (`404` unknown id, `409` nothing live, `400` a bad layer or operation).
- `{id}` is a UUID, a name (case-insensitive) or a 0-based index in the list's order (sort key,
  then name). Lists answer `{"id": {"uuid", "name", "index"}, …}` objects.

| Endpoint | Vesper |
|---|---|
| `GET /version` | `{name, platform, os_version, host_description: "Vesper <version>", api_version: "v1"}` |
| `GET /v1/trigger/next`, `/v1/trigger/previous`; `/v1/presentation/{active\|focused}/{next\|previous}/trigger` | `next`, `prev` |
| `GET /v1/presentation/{active\|focused\|id}/trigger`, `…/{index}/trigger` | `triggerSlide` (first slide / `index`; the live presentation keeps its playlist item) |
| `GET /v1/presentation/active`, `/v1/presentation/slide_index`, `/v1/status/slide` | the live presentation; its slide index; current and next slide `{uuid, text, notes}` |
| `GET /v1/playlists`, `/v1/playlist/{id}/trigger`, `/v1/playlist/{id}/{index}/trigger` | playlists (folders left out); `triggerPlaylistItem` (first presentation or media item / `index`-th item) |
| `GET /v1/clear/layer/{layer}` | `clearLayer` (`audio`, `props`, `messages`, `announcements`, `slide`, `media`, `video_input`) |
| `GET /v1/clear/groups`, `/v1/clear/group/{id}/trigger` | clear groups; `clearGroup` |
| `GET /v1/timers`, `/v1/timers/current`, `/v1/timers/{start\|stop\|reset}`, `/v1/timer/{id}/{start\|stop\|reset}`; `PUT /v1/timer/{id}/increment` (body: seconds); `GET /v1/timer/system_time` | timers, their values (`time`, `state`); `timerStart` / `timerStop` / `timerReset` (every timer or one); `timerAdjust`; Unix time |
| `GET /v1/messages`; `GET`, `POST /v1/message/{id}/trigger` (body `[{name, text: {text}}]`); `GET /v1/message/{id}/clear` | messages; `messageShow` with token values; `messageHide` |
| `GET /v1/looks`, `/v1/look/current`, `/v1/look/{id}/trigger` | looks; the live look; `setLook` |
| `GET /v1/props`, `/v1/prop/{id}/trigger`, `/v1/prop/{id}/clear` | props (`is_active`); `propShow`; `propHide` |
| `GET /v1/macros`, `/v1/macro/{id}/trigger` | macros; `triggerMacro` |
| `GET`, `PUT`, `DELETE /v1/stage/message` (PUT body: a JSON string) | the stage message; `stageMessage`; `stageMessageClear` |

Everything else (transport, media and audio playlists, library documents, thumbnails, the status
stream) answers `404`. Vesper's own API (`/api/v1`) covers all of it; the Companion module in
`integrations/companion/` uses that.

## Presence and edit locks

Who has which presentation, theme, stage layout, prop, timer, playlist or collection open, where,
and who is editing it (capability `presence`; the places `show` and `timers` with capability
`presenceBrowsing`; plan in `docs/PRESENCE-PLAN.md`).

- **Presence.** Over the WebSocket a client sends what it has open, whenever that changes and
  after every (re)connect: `{"type":"presence","clientId":"…","computer":"Booth laptop","open":[{"kind":"presentation","id":"…","place":"edit"}]}`.
  The list replaces the one sent before (at most 32 entries; more is an id-less `badRequest`
  error). `place` is `edit` (slide editor), `reflow`, `library` (the Library page; with kind
  `collection`, the collection it shows), `show` (the Show page, browsing: its playlist
  (`playlistNode`) and presentation) or `timers` (the Timers page: the timer it shows); an
  unknown place reads as `other` (capability `presenceBrowsing`; an older server refuses the
  whole message, so a client sends `show` and `timers` only with that flag). The server lists
  every place to every client: a client must read a place it doesn't know as `other`. Oriel
  releases before `presenceBrowsing` don't, and name a viewer on the Show or Timers page
  "undefined" in the who's-viewing list (cosmetic; updating Oriel fixes it). Each entry may
  name its `collection` (the collection the client shows; Oriel sends it, see *Per
  collection* below). `clientId` is the one the
  client sends with its edits (`X-Client-Id`); `computer` defaults to the hello's
  `clientName`. Each WebSocket connection that has reported is a **session**; it ends when the
  socket closes or the heartbeat gives up on it (60 s of silence). Connections that never
  report aren't listed.
- `GET /api/v1/presence` → `PresenceList {clients, locks}`: `PresenceClient {session, clientId?,
  name, user?, computer?, open, server?}` (`name`: the client's key name, or `server machine`; `user`: the
  site user it signed in as) and `DocumentLock {kind, id, clientId, name, user?, computer?,
  since, server?, collection?}`. Every change is sent to every client as `{"type":"presenceChanged","presence":{…}}`
  (the whole list).
- **Edit locks are hard.** These writes check the document's lock: `POST
  /presentations/{id}/edits`, `/move` and `DELETE /presentations/{id}`; `PUT`, `DELETE
  /themes/{id}` and `POST /themes/{id}/edits`; `PUT`, `DELETE /stage-layouts/{id}` and `POST
  /stage-layouts/{id}/edits`; `PUT`, `DELETE /props/{id}` and `POST /props/{id}/edits`; `PUT`,
  `DELETE /timers/{id}` (a timer's settings: starting or stopping it is a command, never
  locked). Playlists are never locked (their edits are small operations the operator makes
  during a service; presence only). The writer is the `X-Client-Id` header, else the edit body's
  `clientId`. When another client holds the lock the write answers **`423 locked`** (the
  message names the holder; nothing is changed). Otherwise it goes ahead and, when the writer
  has a live session with the document open in a place that edits it (`edit`, `reflow`,
  `library`, `timers`; never `show`), takes the lock (the first edit locks; opening alone
  doesn't). A writer without a live session (a script, no `clientId`) never takes a lock.
  Reading, duplicating and other documents are never locked.
- `POST /api/v1/presence/locks` `TakeLock {kind, id, takeOver?}` with `X-Client-Id` (`400`
  without it) → `DocumentLock`: takes the lock (for a client with a live session; `409`
  otherwise). Held by another client: `423`, unless `takeOver: true`, which moves it to the
  caller at once (no admin needed). `DELETE /api/v1/presence/locks/{kind}/{id}` with
  `X-Client-Id` releases the caller's lock (`204`).
- **A lock ends** when its holder's presence no longer lists the document in a place that edits
  it, or when the holder's
  last session ends. Nothing is stored: a restart frees every lock.
- **Site-wide.** On a site with a Codex, a show box reports its clients to the Codex box and
  shows the site's list: `GET /presence` and `presenceChanged` on any box list every client
  of the site, and `PresenceClient.server` / `DocumentLock.server` name the Vesper a client of
  another box is connected to (absent for the box's own). Locks are held on the Codex box: a
  lock taken through one box refuses writes through every other (`423`), and `POST` /
  `DELETE /presence/locks` on a show box act on the Codex's locks. A show box that can't reach
  the Codex shows its own clients only (its writes answer `503` then). Over the link:
  `vesper.codex.presence`, `vesper.codex.lock`, `vesper.codex.presence_changed` (each list
  stamped with the Codex's presence generation: a show box never goes back to an older one).
  Across versions: a box's report and lock request and the Codex's answers say `allPlaces:
  true` (they read every place, an unknown one as `other`); a peer that didn't say so (a
  release before the places `show` and `timers`) gets clients cut down to `edit`, `reflow` and
  `library` (a timer's settings as `library`, the Show page left out), so a site mid-upgrade
  keeps its site-wide presence and locks.
- **Per collection** (F08-1). Two collections can hold the same document IDs (a show box that
  re-uploads its collection after leaving and rejoining the site makes a second one). A lock is
  in the collection its write or take is for (a show box's forwarded collection on the Codex,
  else the box's active collection) and says so in `DocumentLock.collection`; locks of two
  collections never block each other. `OpenDocument.collection` (optional) names the
  collection the client has the document open in: only an open entry in the lock's collection
  takes or keeps a lock. A client shows only the entries and locks of the collection it shows
  (Oriel's badges, "*X* is editing" and lock banners). An entry or lock without a collection
  (an older client, show box or server) matches the document in any collection.
- The client that lost a lock learns it from `presenceChanged`; its writes since get `423`, so
  it keeps what it hadn't saved and offers it (Oriel keeps the typing and offers "Take back").
  Locks come before `If-Match` and `expectText`, which apply as before.
- **Per box for now:** each box keeps its own clients' presence and locks, and checks them
  before a show box forwards a write (the Codex box checks forwarded writes against its own
  clients' locks; a forwarded write never takes one). Site-wide presence over the
  box-to-Codex link is planned.

## Object builds

A slide's `builds` (`Build[]`, omitted when empty) is its build order: `{element, mode?
("in" | "out", default in), start? ("onNext" default, "withPrevious", "afterPrevious",
"withSlide"), delay? (s), transition? (the shared `Transition`; omitted = cut)}`. The order
splits into steps: step 0 plays with the slide (`withSlide` builds, and `withPrevious` /
`afterPrevious` ones before the first `onNext`); each `onNext` build opens the next step.
`next` takes the live slide's next build step before it moves to the next slide (no layer
transition: only the elements move); `prev` takes one back (settled, no animation), and at step
0 goes to the previous slide. Triggering a slide starts it at step 0; each layer (slide,
announcements) keeps its own progress, so a slide triggered on the other layer leaves it alone.
`next` / `prev` step the builds of the live slide the position is on, and the number of steps
always follows the slide as it is now (an edit of the live slide's builds counts at once).
`state.build` reports `{layer, presentation, slide, step, steps}` for that slide while it has
builds; `state.cursor` (the stage's next slide) is always the next *slide*. Elements drawn by an `in` build that has
not played are hidden, `out` builds hide theirs once played; `dissolve` fades, `push` / `slide`
move the element from or past the canvas edge, `zoom` scales and fades it, `wipe` plays as a
dissolve. Thumbnails and the editor render show every element. `setSlide {builds}` replaces the
order (at most 1000 builds, else `400`). A build that names an element the slide doesn't have
(its element was deleted) is kept but ignored: it takes no step and draws nothing, it can be
sent back with the rest of the order (and an undo that restores it applies), and it plays again
when the element comes back (an undone delete). Older servers keep `builds` in the
document and show every element.

## Themes

Themes are **linked** (design: `docs/THEMES-PLAN.md`): a slide made from a theme can **follow** it, so editing the theme restyles every song that uses it, while a slide can still override single properties.

- **The link.** A slide follows its theme when it has `templateOrigin` (`{theme, slide}`: the template slide) **and** `themeLink` (`{overrides?}`: `background`, `transition` and `removed:<template element id>` that it sets itself). Its elements that come from the template have `fromTheme` (`{element, overrides?}`: the template element and the properties the slide overrides: `name`, `bounds`, `rotation`, `opacity`, `flip`, `hidden`, `locked`, `aspectLocked`, `shape`, `fill`, `stroke`, `shadow`, `feather`, `scroll`, `visibility`, `dataLink`, `text.paragraphs` (a decoration's text), `text.defaultStyle`, `text.defaultParagraph`, `text.verticalAlign`, `text.fit`, `text.fontScaleFactor`, `text.margins`, `text.textShadow`, `text.outline`, `text.lineMask`, `text.fill`). A slot's text and picture are the slide's content, never a property. Other elements are the slide's own. Slides without `themeLink` (every imported presentation) are plain copies, as before.
- **Overrides are kept by the server**: after every edit of a following slide, each element's `overrides` lists the properties that differ from its template element (one set back to the theme's value follows the theme again); a deleted theme element is recorded on the slide. Clients only read them (the editor marks them).
- **Applying**: `applyTheme {theme, template?, slides?}` (all slides when `slides` is omitted) makes slides follow the theme: each gets `template`, else the template slide named like the one it was made from, else the theme's first. The template is applied (content kept, the previous theme's elements go; a slide without a text slot pours its largest text box into the template's), then linked. `resetToTheme {slide, element?, properties?}` clears overrides (the slide's own and every element's; one element's; or only the named properties) and restyles. `unlinkTheme {slides?}` keeps the look as a plain copy. `linkTheme {theme?, slides?}` (batch 9) links slides that don't follow a theme **keeping their look** (imported songs): each follows its `templateOrigin` template slide when that fits, else the template slide (of `theme` when given) with the most matching elements and then the fewest overrides; elements are paired bottom-up with the template's (same slot, else same name and kind) as far as the z-order allows, everything that differs is recorded as an override, and the server checks that resolving gives the slide back, so nothing on screen changes. Slides that can't be linked that way stay as they are (only the linked ones appear in `applied`). They are applied as `replaceSlide {slide}` (the whole new slide; its inverse is the old one), and the server's override bookkeeping after an edit is one more `replaceSlide` per slide in `applied`.
- **Editing a theme** works like a prop: `GET /themes/{id}/document` is a presentation with the theme's ID, name and canvas size and one section whose slides are the template slides, each labelled with its template's name (`setSlide {label}` renames it). `POST /themes/{id}/edits` takes the presentation edit ops, including adding, duplicating (a new template slide), moving and deleting slides; sections, `splitSlide`, `mergeSlides`, arrangements and the theme ops are refused (`400`), and so is deleting the last template slide. **Every change of a theme** (edit, rename) re-resolves, in the same write, every slide that follows it: template elements in the theme's order with the slide's content and overrides, new template elements added (empty slots), slides' own elements on top; a slide whose template slide was deleted keeps its look, unlinked. Each presentation that changes gets its own `documentChanged` (no `ops`: read it again), and its thumbnails new `ETag`s.
- **Template slides and roles** (batch 9): a template slide's roles are its slots (`placeholder.slot`: `lyrics`, `title`, `scripture`, `reference`, …); `setElement {patch: {placeholder}}` gives a box a role or clears it (`null`: decoration). A slot whose role is cleared keeps each following song's text (recorded as a `text.paragraphs` override) instead of showing the template's sample. `GET /themes/{id}/used-by?template=<template slide id>` lists only the presentations with a slide following that template slide (and messages using it), so a client can say who a delete affects; following slides keep their look, unlinked, when their template slide is deleted (undoing the delete brings the template slide back, not the links).
- **New themes** (`POST /themes` without `document`) get three template slides, Lyrics, Title and Scripture (with a Reference slot), white Arial on black, on the show's default canvas size; the name defaults to "Theme" (numbered when taken). `POST /themes/{id}/duplicate` copies it with fresh IDs ("*name* copy"); nothing follows the copy.
- **Deleting** a theme that presentations follow is refused (`409` with `current: {usedBy: DocumentUse[]}`) unless `?reassign=<theme>` (they follow that theme, template slides matched by name, else its first) or `?unlink=true` (they keep their look as copies). Deleted themes go to the item trash and can be restored from it. Messages naming a deleted theme fall back to the built-in banner.
- `documentChanged` uses kind `theme`. The `themeEditing` capability says the server has all of this.

## Timers, messages and props

Timer, message and prop documents are edited with whole-document `PUT`s, like screens: create with `POST` and a partial document (the server fills in the ID, a name such as "Timer 2", the sort key and defaults; a new timer without a `kind` counts down 5 minutes), update with `PUT` and `If-Match: "<revision>"` (`409` with `current` on a mismatch), delete with `DELETE` (`204`). Validation errors are `400` with `error.field` (a countdown runs 0 s – 99 h, a time of day is 0 – 86399, token names are unique, non-empty, without leading or trailing spaces and without `{`, `}` or `$`; inner spaces are allowed, as in `${Child Number}`). `documentChanged` events use the kinds `timer`, `message` and `prop`, with the new `revision` and the `clientId` (`X-Client-Id`). Deleting a timer, message or prop that is in use drops it from the show (its layer clears).

- **Messages.** `text` holds `${name}` placeholders (`{name}` works too). Each token has a `kind`: `text` (`defaultValue`; `messageShow` gives values), `timer` (`timer`, `format`: the timer's value), `clock` (`format`). The text goes into the first text slot of the message's `template` (`{theme, slide}`: a template slide of a theme; its other text slots are emptied), else its first text element; without a template the server draws a built-in lower-third banner (translucent black bar, white centred text). Messages appear on screens whose look shows the messages layer; the last one shown is drawn.
- **Props.** A prop is one slide on its own canvas (`size`). `POST /api/v1/props` with `fromSlide: {presentation, slide}` copies that slide's elements (fresh IDs, no actions) and canvas size. In the Edit view a prop is edited as a one-slide presentation: `GET /api/v1/props/{id}/document` (the wrapper: the prop's ID, name and canvas size, one section with the prop's slide), `POST /api/v1/props/{id}/edits` (the presentation edit ops; ops that add, delete, move or split slides or sections, arrangements and slide actions are refused with `400`), `GET /api/v1/props/{id}/slides/{slideId}/render` (the editor picture).
- **Live data.** Screens draw data-link elements with live values: timers (with their format and colour triggers; a link without colour triggers turns amber while its countdown has `warningSeconds` or fewer left), the clock (in the box-wide format where the link leaves it open, see `setClockFormat`), the current / next slide's text, notes, label and section, slide counts, playlist item names, the stage message, video / audio countdowns, the live presentation's name (`presentationName {source?: "presentation" | "announcements"}`) and notes (`presentationNotes`), the song's copyright line (`ccliText`: from its `songInfo` and the show's `ccli` settings, the `layout` tokens or, without one, title / author / `© year publisher` / `CCLI Song #` / `CCLI Licence #` one per line), and (on stage screens) slide pictures. **The copyright line on its own:** with the show's `ccli.enabled` and the song's `songInfo.display`, the slide layer draws the CCLI template (`ccli.template`, laid out at the show's `defaultCanvasSize` and scaled to the song's canvas; without one a built-in line of small white text in the bottom-left corner) over the song's slides that `ccli.showOn` names (`firstSlide`, the default; `lastSlide`, `firstAndLast`, `allSlides`), counted in the play order (an arrangement's repeats apart), on every audience screen that shows the slide layer (over a look's template override too); its `ccliText` elements show the line. A timer link's `show` (`value`, default; `elapsed`: a countdown's duration minus its value; `remaining`: a count-up timer's end minus its value; empty where there is none) picks the time it shows. A `songArtwork` link is a picture: the element is filled with the live song's `songInfo.artwork` (a media asset, fitted), or drawn without its fill when there is none. Visibility conditions are evaluated (timer `hasTimeRemaining` / `hasExpired` / `isRunning` / `notRunning`, `elementHasText`, `videoInput {input, active?}` (the input on the video-input layer), and since batch 12 `message {message?, active?}` (a message, or any, on the messages layer), `stageMessage {active?}`, `layer {layer, active?}` (a show layer has content), `playlistItem {which?, active?}` (the live item's playlist has a current / next item or header), `videoCountdown {state?}` and `audioCountdown {state?}` with `hasTimeRemaining` / `hasExpired` / `isRunning` / `notRunning` / `looping` / `notLooping` (nothing live counts as expired, not running, not looping); `active` defaults to true). Older servers read the new links and conditions as `unknown` (authored text, always shown). A screen re-renders when a value it shows changes: at most once a second for clocks and timers (a timer showing tenths changes its screens once a second), at once when the slide changes. Thumbnails and editor renders show the authored (design-time) text and every element.

## Stage display

A screen with `kind: {"type": "stage"}` is a **stage screen**: it shows a stage layout to the performers and **never** the audience layers (background media, slides, props, audience messages, announcements). `setScreensEnabled` with `role: "stage"` (Ctrl+2 in Oriel) switches every stage screen to black and back.

- **Stage layouts** are one-slide documents (`StageLayout {id, name, sortKey, size, slide}`) whose elements carry data links: current / next slide text, notes, clock, timers, video countdown, playlist item names, the stage message, slide pictures. They are created, edited and deleted like props (REST table above; in the Edit view as a one-slide document; the Edit view's insert toolbar has a **Data link** menu). `POST /api/v1/stage-layouts` with `"template": "currentNext" | "lyricsClockTimer" | "notes" | "musician"` starts from a built-in template (1920 × 1080); without one the layout is blank. `documentChanged` uses kind `stageLayout`.
- **Which layout a stage screen shows** (`state.stageScreens[].layout`) is the most specific of: a choice for that screen in `state.stageLayouts` (a stage-layout slide action, or `setStageLayout`), a choice for every stage screen (an action without a screen), the screen document's stored `kind.layout`, the first stage layout in sort order, and the built-in "Current / Next" layout (`layout: null`) when the data folder has none. `setStageLayout` stores its choice in the screen document (portable: it survives a restart and travels with the data folder; `documentChanged` kind `screen` follows); slide actions only change the running show. Choices naming a deleted layout are skipped.
- **Next slide** is the slide `next` would trigger (`state.cursor`): the play order (the arrangement, see Navigation), skipping disabled slides, on into the next presentation item of the playlist the slide was triggered from.
- **Slide pictures** (`slideImage` links, current or next) show the **slide layer only**: the slide drawn on black at the element's size, with its live data, without background media, props or messages. They change with the slide. The element's own text is not drawn.
- **Stage message** (`stageMessage`, `stageMessageClear`; `state.stageMessage`): shown in the layout's stage-message elements (`message` links), else in a built-in banner along the top (large white text on an opaque black bar). With `flash`, the banner (or a message element whose link has `flash`, in its `flashColor`, default red) turns red and back every 0.4 s for 4 s. Audience messages are not shown on stage screens.
- Stage screens cut (no dissolves). Clocks and timers re-render them at most once a second; a slide change, a message or a layout change at once.

## Chords

Batch 16. Songs carry chords for the band; audience screens never show them.

- **On the lyrics.** A paragraph has `chords?: ChordMark[]`, each `{at?, chord}`: the chord as written (`"G/B"`, `"Am7"`, `"N.C."`) and the character it sits over (`at`: an index in Unicode characters into the paragraph's plain text, default 0; the text's length = after the last character). Chords are not text: `plainText`, search, the slide grid and data links that show slide text ignore them. A text box draws them above the words only when its `chords` display is on (`TextBox.chords?: ChordDisplay {show?, color?, size?}`: colour default warm yellow, size a fraction of the line's text size, default 0.7); every line with chords then gets a chord row above it, each chord left-aligned over its character and pushed right past the chord before it when they would overlap. `setSlideText` keeps a changed line's chords on their words (characters before and after the changed part keep theirs; a chord inside it keeps its offset, at most on the last new character; when lines are inserted, deleted, split or joined, the changed block is remapped as one text, so each chord stays on the line holding its character and an inserted line has none); `replaceTextBox` takes the paragraphs as sent.
- **The song's key.** `SongInfo.originalKey` is the key the chords are written in; `SongInfo.transpose?` (semitones, default 0) is how far the band plays it from the written chords (the song's key control). The ProPresenter importer reads chords from the text's `chord` custom attributes, the chord display from `chordPro` (shown, its colour), the written key from `music.originalMusicKey` / `music.original` and `transpose` from the user key (`music.user`).
- **Musician stage view.** Two data links: `chordChart {which?: "current" | "next"}` shows that slide's lyric lines (the text `slideText` would show) **with their chords above the words** (the element's own text style; chords drawn in its `text.chords` style, shown even when it is off), and `songKey` the live song's key line (`"Key A"`, `"Key A · Capo 2 (G shapes)"`, `"Capo 2"`, or empty). Both follow the stage layout's **chord view** (`StageLayout.chords?: ChordView {transpose?, capo?, accidentals?: "auto" | "sharps" | "flats", notation?: "letters" | "nashville"}`): the chords shown are the written ones moved by the song's `transpose` + the view's `transpose` − `capo` (a guitarist's layout with a capo reads the shapes; a keys player's layout without one reads concert pitch); the key line names the concert key and, with a capo, the shapes' key. "What's next" is `sectionName {which: "nextSection"}`. The built-in template `musician` ("Musician (Chords)": key line, current section, the current slide's chord chart, the next section's name and the next slide's chart, clock and timer) starts such a layout; set the view with `PUT /stage-layouts/{id}` (`chords`). A `next` chart that falls into the next playlist item uses the live song's key.
- **ChordPro.** `POST /api/v1/chordpro/parse` (`ChordProParse {text, name?, linesPerSlide?, like?, split?}` → `ChordProParsed {presentation, sections, slides, chordLines, secondLanguageSlides, unpairedStanzas, capo?}`, `400` when the text has no lyric lines) reads a ChordPro song (`[C]Morning [G/B]light` lines; `{title}`, `{artist}`, `{composer}`, `{key}`, `{capo}`, `{ccli}`, `{copyright}`, `{year}`, `{album}`; `{start_of_verse}` / `{sov}`, `{soc}`, `{sob}`, `start_of_<kind>`, `{chorus}` repeats; `{comment: Chorus}` or a heading line names a section; chord lines above lyric lines are read by column; tabs and grids are skipped) into a new, unsaved presentation: sections named like other imports ("Verse 1", "Verse 2", "Chorus", "Bridge", linked to the global group of that name), a slide per stanza (or per `linesPerSlide` lines), text styled like `like`'s, an arrangement when sections repeat. Create it with `POST /api/v1/presentations {document, library}`. `GET /api/v1/presentations/{id}/chordpro` writes a presentation as ChordPro (`text/plain`, attachment `<name>.cho`: metadata, sections in document order, the slides' primary text with chords inline). **Two languages** (`docs/DUAL-LANGUAGE-PLAN.md` §2): `split` (`SecondSplit`: `{type: "none"}` (default), `"alternating"` (lines alternate in each stanza), `"line"` (a `---` line: first language above, second below), `"inline", separator` (`first / second` inside each line, outside the chords), `"comments"` (`{ci: …}` / `{comment_italic: …}` lines inside a stanza), or `"auto"` (the file's `{meta: second_language_split …}`, else a `---` line, else none)) takes each stanza's second language out into its slide's second text (`Slide.translation`, language from `{meta: second_language xx}`, a language code like `language`; `{meta: language xx}` is the presentation's own `language`); chords stay on the first language, and are dropped from second-language lines. Only italic comments are the second language in the comments split: `{c: …}`, `{cb: …}` and `{highlight: …}` notes are skipped. A stanza split into several slides (`linesPerSlide`) splits its second lines with its lyric lines when they pair line by line (a line of chords alone takes none); otherwise all of them stay on the stanza's first slide, counted in `unpairedStanzas`. The export writes slides' second texts as `{ci: …}` lines after their own lines (a slide's empty paragraphs are not written as blank lines inside it: leading and trailing ones are dropped, inner ones split it into stanzas, each with its own `{ci: …}` lines when they pair), with `{meta: second_language xx}` and `{meta: second_language_split comments}` in the header, so `auto` reads it back the same way; the presentation's `language` is written as `{meta: language xx}`. The edit op `applyChordPro {text}` puts a ChordPro text's chords on the song's lines with the same words (ignoring case and punctuation), see the edit ops.
- **Editor render** with `chords=1` (`…/slides/{slideId}/render?w=&chords=1`) draws every text box's chords (the Edit view's chord view).
- **Transposition rules** (`model::chords`): a chord is a root `A`–`G` with `#` / `b`, a suffix kept as written (`m`, `7`, `sus4`, `add9`, `maj7`, `6/9`, `dim`, `+`) and an optional bass note after the last `/`; anything else is shown unchanged. Spelling: `sharps`, `flats` or `auto` (by the key shown: flats in F, Bb, Eb, Ab, Db and their relative minors, Gb / F# as written; without a key, as written). Nashville numbers (`1`, `5/7`, `6m7`, `b7`) are relative to the written key and need one (letters without).

## Live captions

Speech turned into caption text on the server (IDN's `idn-captions`: whisper.cpp on the server's
CPU; nothing leaves the box but the text), drawn on the **captions layer** (`captions`, the
top-most visual layer, above messages and under a screen's mask) and optionally sent on the network
(`docs/CAPTIONS-PLAN.md`).

- **Configuration** (`GET`/`PUT /api/v1/captions/config`, a machine document): `enabled`;
  `source` `{type: "program"}` (the program audio bus, default), `{type: "liveSource", name}` (a
  network source's sound; its receiver is kept open while captions listen) or `{type: "external"}`
  (no speech engine: lines come from `POST /api/v1/captions/lines`); `language` (`en` default;
  another language needs the multilingual `large-v3-turbo`, and `auto` picks it); `model` (`auto`
  default: the server measures itself once and takes the largest model that keeps up, else a name
  from `GET /captions/models`); `words` (names to spell right); `keepMusic` (caption sung lines too;
  off by default since speech recognition invents words over music); `maxLines` (1–4, default 2);
  `style {font?, size? (56, px at 1080 lines), color?, background? (alpha 0: no band), position?
  ("bottom" / "top"), margin? (40)}`; `audience` (default true: the layer on audience screens, per
  look with `LayerVisibility.captions`), `stage` (also on stage screens, over their layout),
  `network` (each line also on an IDN Open Protocol (compatible with NDI®) text-only source called
  "Captions", `text/1`). The engine restarts only when the source, model, language, words or
  `keepMusic` change.
- **Models** are fetched once, on first use, from whisper.cpp's files at a pinned commit (size and
  SHA-256 checked) into the data folder's `cache/whisper/`; `state.captions.status` is
  `downloading` meanwhile, with `progress` (0–100, when the size is known), `downloaded` (bytes so
  far) and `downloadSize` (bytes, when known), and `detail` naming the model. With `model: "auto"`
  the models it measures are downloaded the same way (IDN v0.3.3's `auto_choose_with`). Servers
  before IDN v0.3.3 leave `downloaded` / `downloadSize` out, and give no progress while `auto`
  measures. An offline server takes the file copied there by hand.
- **Lines.** A draft is corrected in place until it is final; the screens show the last
  `maxLines` lines (`state.captions.lines`, the last may be a draft) and clear them after 8 s
  without a new line (the state keeps them). `clearLayer {layer: "captions"}` takes them off the
  screens; captions keep running. `clearAll` and clear groups leave captions alone.

## Thumbnails

`GET /api/v1/presentations/{id}/slides/{slideId}/thumbnail?w=320` returns a JPEG of the slide drawn over its background: the most recent background media action at or before the slide. `w` defaults to 320 and is clamped to 16..1920; the height follows the presentation's aspect ratio.

- The response has a strong `ETag` (a content hash of the slide, its background file, the width and the server version) and `Cache-Control: no-cache`. Send `If-None-Match` and the server answers `304` without rendering anything.
- The server caches thumbnails in memory (LRU) and on disk, under `<data>/cache/thumbnails/<presentation>/<slide>-<w>-<hash>.jpg`. A cached thumbnail comes back in well under a millisecond.
- Thumbnails render on the server's render thread at low priority: a live change always goes first.
- When a presentation is opened, call `POST /api/v1/presentations/{id}/thumbnails?w=320` (or the `prefetchThumbnails` command) so the whole grid is warm. The thumbnail requests that follow are cache hits.

## Editor render

`GET /api/v1/presentations/{id}/slides/{slideId}/render?w=960` is the Edit view's picture of a slide: the slide alone (its background colour, **no** background media), transparent where it draws nothing, as a PNG with straight alpha. `w` defaults to 960 and is clamped to 16..3840; the height follows the canvas aspect ratio. `hide` (comma-separated element IDs) leaves elements out, so an editor can draw the ones being dragged or typed into itself. Nothing is cached (`Cache-Control: no-store`); it renders at interactive priority, behind any pending live change. Add a query parameter that changes with the slide (a content hash) so the browser does not reuse an older picture.

## Live preview

After `{"type":"subscribePreview","screen":"…","fps":10,"width":480}`, the server pushes **binary** WebSocket messages for that screen. It sends one right away, then at most `fps` per second, and only when the picture changed. A slow client drops frames instead of building a queue. The server encodes only while someone is subscribed, and every subscriber of the same screen and width shares one encode.

```text
offset  size  field
     0    16  screen ID: the UUID's 16 bytes (same byte order as its text form)
    16     8  frame counter, u64 little-endian (grows with every new picture)
    24     4  width,  u32 little-endian
    28     4  height, u32 little-endian
    32     …  JPEG (FF D8 …)
```

The frame counter starts again from 0 when the screen's output is re-created (its size, alpha or background changed), without a reconnect: on one socket a screen's frames come in order, so a counter lower than the last one is a new picture, and only a repeat of the same counter can be skipped.

```ts
ws.binaryType = "arraybuffer";
ws.onmessage = (e) => {
  if (typeof e.data === "string") return handleJson(JSON.parse(e.data));
  const v = new DataView(e.data);
  const hex = [...new Uint8Array(e.data, 0, 16)].map((b) => b.toString(16).padStart(2, "0")).join("");
  const screen = `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
  const frame = v.getBigUint64(16, true), width = v.getUint32(24, true), height = v.getUint32(28, true);
  const jpeg = new Blob([e.data.slice(32)], { type: "image/jpeg" });
  showPreview(screen, URL.createObjectURL(jpeg), width, height, frame);
};
```

In Rust: `protocol::PreviewFrameHeader::decode(&bytes)`.

## Site

Every Vesper server is always in an IDN site. A server on its own runs its own: the site's **IDN Site Manager** runs in it as a module, so a standalone install is a one-machine site. There is **exactly one Site Manager per site**: a server can instead join another Vesper server's site as one of its machines (below), and its own Site Manager then stops and stays off until it leaves. Machines (other Vesper boxes, IDN Nodes, display boxes) join the site; its users and roles are shared with the site's other tools. Ports follow IDN's port map: discovery **5959**, the Site Manager's own HTTPS API **29202** (`--site-discovery-port`, `--site-api-port`). Its files live in `<data>/machine/site-manager/` (the site CA key: part of the backup), `machine/site.json` and `machine/site-audit.jsonl`. A damaged `machine/site.json` (or `machine/clients.json`) doesn't stop the server: it is moved aside (`….damaged-<time>`) and the server starts with access `users`, no sign-ins and no PIN (the site itself is the Site Manager's folder), and says so in its log and in `GET /api/v1/site` (`settingsDamaged: true`; Oriel shows it as a warning on the Site page). It fails closed: until the access is set again (`PUT /api/v1/site/settings` from the server machine or a signed-in admin user, even to the same `users`), a paired client that hasn't signed in only reads, even on a site with no admin user (a site on `pin` normally has none); this survives a restart.

- **Joining another server's site** (admins): `POST /api/v1/site/join {server, code?}` with the site server's discovery address (`host` or `host:port`, port 5959 by default) and, where the site's join mode takes one, a join code from that server (`code_or_approval`: a code joins at once, without one the join waits for an operator's approval there, comparing the code `GET /site/membership` shows; `code_and_approval`: a code, then approval). `POST /site/join {code}` types a code into a join already under way. Refused with `409` while this server's own site still has other machines (they would be left without a site: remove them or move them first), and while it is already in another site. Once in, this server's own Site Manager stops: `enabled` reads `false`, and turning it on again answers `409` until the server leaves. Its paired clients' keys were its own site's: they pair again, once, through the usual steps (approved at the new site's server). `GET /api/v1/site/membership` (also `SiteStatus.membership`) shows where it stands (`state`: `none`, `starting`, `noSite`, `waitingForWindow`, `waitingForApproval` with the comparison `code`, `codeNeeded`, `codeAccepted`, `wrongCode` with `triesLeft`, `codeExpired`, `enrolled` with `connected`, `denied`, `removed`, `error`), with `siteChanged` `membership` when it changes. `POST /api/v1/site/leave` leaves (or stops joining): the server tells the other site, which revokes it and lists it no more (`device_left` there; told only while connected, else the site lists it until it is removed there), and runs its own site again (the same one it had). Removed by that site, the server runs its own site again by itself and forgets that site (`membership` reads `none`): it never asks to join it again by itself, so an admin there can't pull it back in; joining again is a new `POST /site/join`.
- **Turning the Site Manager off** (`PUT /api/v1/site/settings {"enabled": false}`) is refused (`409`) unless the server is in another site (where joining has done it already). `vesper --no-site-manager` keeps it off for testing: a server in no site then can't pair clients.
- **Pairing** (see [Authentication and pairing](#authentication-and-pairing)) makes each client's named site API key with the Site Manager; every server of the site checks keys locally. Pairings waiting for approval (asked without a code at the site's server, ids `p<n>`; relayed by the site's other servers, ids `k<n>`) are listed by `GET /api/v1/site/pairings` (`PairingRequest`: the client, the server it asked at, its address or that server's fingerprint, since when; `launcher`: a Vesper Launcher's machine ID, whose approval makes that machine's Launcher key, see [Updates](#updates)) and answered by an admin (at access `users`, a signed-in one): `POST /site/pairings/{id}/approve`, `POST /site/pairings/{id}/deny {reason?}` (the reason is told to the client). A relayed pairing typed with the code the site gave its relay doesn't wait: the site's server approves it at once (its `key_requested` event has `detail.codeOk: true`; the Site Manager audits the approval as `key_request/approve`, with the machine it came through). A relayed request ends when its server disconnects or after 10 minutes (`key_request_cancelled`). `GET /api/v1/site/api-keys` lists the site's named keys (`SiteApiKey`: name, audience, scope, made when and by whom, `user (via machine)` for a relayed pairing, expiry, revoked, last used and where; never the key); `DELETE /api/v1/site/api-keys/{id}` revokes one (admins): it stops on every machine at once. A key Vesper recorded, when it made it, as made for an Oriel pairing that is still unused 24 hours after it was made is revoked by the site's server (checked hourly; audited as `api_key/revoke_unused` by the `server machine`): it most likely never reached anyone. Only recorded keys are swept, never by name: a key an admin made in the Site Manager is never touched, even one named `Oriel – …` by hand.

Join modes, roles and event names keep IDN's spelling (`code_or_approval`, `operator`, `join_code_wrong`); everything else follows the conventions above.

- **Join mode** (one per site): `approval` (the machine asks, an operator compares the 6-digit code both screens show and approves), `code_or_approval` (**default**: a machine that types a join code joins at once; one without waits for approval) or `code_and_approval` (a code, then approval). Approval needs the approval window open (`POST /site/pairing`).
- **Join codes:** 6 digits shown `"482 913"`, single use, 10 minutes, burned after 5 wrong tries (and an address is refused for 10 minutes after 10 failures). `POST /site/join-code` makes the site code and takes back earlier ones made here; this server shows it (`GET /site/join-code`) until it is used, expires or burns.
- **Access** (`SiteAccess`, the setting `access`) decides who may do the site's admin actions (approve / deny, the approval window, join codes, the join mode, Remove / allow, restart, users, the password, tokens, the settings). Reading the site's state (status, lists, events) needs only pairing; the audit and logs too, except with `users`, where they need a sign-in (any role). The server machine itself is always the site's `admin`.
  - `open` (**default**): every paired client acts as the site's `admin`. No PIN, no sign-in. This means **any paired Oriel can approve new devices** into the site (and remove them, issue join codes, change the settings): pairing with the server is the only gate. Choose `pin` or `users` where that is too open.
  - `pin`: one admin PIN, 4–8 digits, stored hashed (Argon2id). A paired client unlocks with it (`POST /site/unlock {pin}`) and acts as `admin` until it locks: on demand (`DELETE /site/unlock` or `DELETE /site/session`), after 30 minutes without an admin action (each one starts the time again; `you.unlockSecondsLeft`), or when the server restarts. Locked, it reads as a viewer. Wrong PINs: `401`; after 5 in a row the client is locked out (`429`) for 1 minute, doubling each time up to 15 minutes. A new PIN locks every other unlocked client.
  - `users`: site users with roles (`admin`, `operator`, `viewer`), shared with the site's other tools. A paired client acts as the user it signed in as (`POST /site/session {user, password}`, kept per client across restarts; `409` with another access level); without signing in it can only read. IDN's role rules apply: joins, devices (Remove, restart), users, the password and the settings are the admins'; operators may identify machines and fetch logs. Switching to `users` needs an admin user (`409`), and the last admin user can't be removed or demoted while it is the level. Should the site lose its admin users anyway (on another tool), a client that hasn't signed in acts as `admin` so the site can't be locked out.
  - Changing the level is an admin action (`PUT /site/settings`, with `If-Match`). `pin` needs `pin` the first time (and takes a new one later); leaving `pin` forgets the PIN; leaving `users` ends the sign-ins. Users can be added at any level, ready for `users`. A user's new password (`PUT /site/users/{name}` with `password`) ends their sign-ins on every other client (the client that set it stays signed in when it is signed in as them); removing a user ends all of theirs.
  - A refused request is `403 forbidden`, its message saying what is missing (unlock, sign in, a higher role).
- **Tokens** for IDN's engines (`desk`, `switcher`, `display`, `data`, `node`), signed by the site, at most 12 hours. Not for `app:vesper` (`400`): Vesper's own API takes a paired client's key, which revoking stops. With `open` and `pin` (unlocked) any scope, issued to a name of the client's own, `vesper:<client id>` (its key's id; `IssuedToken.user`), so `DELETE /site/tokens/vesper:<client id>` ends one client's tokens and no one else's (an admin, or that client itself, locked or not). With `users` they are issued to the signed-in user: viewers get `read`; operators and admins also `control`, `bus:<bus>` and `iem:<mix>`. A role or password change, or removing the user, ends their tokens. The server machine itself gets tokens as `admin`.
- **Names and passwords** follow the Site Manager's rules, and its words come back in the `400`: user names of letters, digits and `. _ - @`; passwords (users' and the site password) of 8 characters or more. `admin` is the built-in administrator, not a user name (its password is the site password).
- **Audit** (`GET /site/audit`), never a PIN, password or token: every change through this API with who, which client and from where. Changes and tokens the Site Manager carries out are its own entries (`source: "siteManager"`, IDN's action names such as `approve`, `join_code`, `token (host)`), with the client and address this server passes it; this server's own entries (`source: "vesper"`) are sign-ins, unlocks (right, wrong, locked out), locks, the settings and requests it refused before they reached the Site Manager. The Site Manager's also has machines typing codes and changes made on its own HTTPS API.

| Route | |
|---|---|
| `GET /api/v1/site` | `SiteStatus`: enabled, running (`error` why not; `apiError` why it runs without its API), `settingsDamaged` (started from a damaged `site.json`, until the access is set again), joinMode, pairingOpen, siteId, ports, `you` (`SiteSession`), revision, `membership` |
| `GET/PUT /api/v1/site/settings` | `SiteSettings {enabled, joinMode, access, pin?}` (responses: `pinSet`, never the PIN) with `ETag` / `If-Match` (admins); `409` with `current` on a stale revision; `enabled` only changes by joining or leaving another site (`409` otherwise) |
| `GET/POST/DELETE /api/v1/site/session` | who this client acts as (`SiteSession`); sign in `SiteSignIn` (`users`; 401 wrong; 5 tries a minute per address); sign out and lock |
| `POST/DELETE /api/v1/site/unlock` | unlock with the admin PIN `SiteUnlock {pin}` → `SiteSession` (`pin`; `401` wrong, `429` locked out, `409` another level); lock again (`204`) |
| `GET /api/v1/site/join-requests` | `JoinRequestInfo[]` (with the comparison `code`, `byCode`) |
| `POST /api/v1/site/join-requests/{id}/approve` (`ApproveJoin {name?}`: the machine's name in the site instead of the one it asked with; its certificate is issued under it, so the device list shows it), `…/deny` | `204` |
| `POST /api/v1/site/pairing` (`OpenPairing {seconds?}`), `DELETE /api/v1/site/pairing` | open / close the approval window |
| `GET/POST /api/v1/site/join-code` | the site code `JoinCodeInfo` (`404` none) / a new one (`201`) |
| `GET /api/v1/site/join-codes`, `DELETE /api/v1/site/join-codes/{id}` | the codes waiting / take one back |
| `GET /api/v1/site/devices`, `GET/DELETE /api/v1/site/devices/{fingerprint}` | `SiteDevice[]`, the Site Manager's view of each machine: `health` (`online`, `needsAttention` + `reason`, `offline` + `lastSeen`, `restarting`, `updating`, `removed`), `problems` (what needs attention: IDN's `problem` name, `level`, `text`, `fields`), the IDN `version` its Node runs, its `services` as its Node last reported them (`DeviceService {id, name, state}`: `id` is what `…/restart` takes; `state` is IDN's word, `running`, `starting`, `restarting`, `failed`, `off`, `unset`, `unsupported` or `no_session`; empty when no Node has reported), services `restarting`, `updating`; `updateManagedBy` (IDN Beta14: the product that installs and updates its IDN Node instead of the Site Manager, `Vesper` on a machine with the vesper-idn-node package; absent: the Site Manager's updates, which leave such a machine out); `nodeStartedAt` (RFC 3339, when its Node last started; absent when no Node has reported) and `machine` (G9, `SiteDeviceMachine {cpu?, cpus?, gpus, os?, osVersion?, kernel?, bootedAt?, memoryTotal?, memoryFree?, reportedAt?}`, bytes for memory, as its Node last reported: an idle Node reports every 30 s, so `memoryFree` can be that old; absent for a device whose Node doesn't report it, an older Node or none, and anything its system doesn't say is left out, never zero); `builtIn: true` (absent otherwise) marks the Codex's own membership of its site, part of Vesper: Oriel shows it as "Codex (built-in)" with no remove action, and `DELETE` answers `409` for it (it would only join again at the next start); `relay: true` (absent otherwise) marks a pairing relay (below) / one / Remove |
| `POST /api/v1/site/devices/{fingerprint}/allow` | let a removed or denied machine ask again |
| `POST /api/v1/site/devices/{fingerprint}/relay` | `SetDeviceRelay {on}` (admins): mark the machine a pairing relay, or not; `204`; `409` for the built-in Codex, to mark a removed machine, or to mark a machine that isn't a Vesper box (an app named `<name> (Vesper)`; Oriel offers Relay on only for those). The mark comes off any machine. Kept as set: the site's server never marks it again by itself |
| `POST …/identify` (`IdentifyDevice`), `…/restart` (`RestartDevice {service}`), `…/logs` | on machines running an IDN Node (`409` otherwise) |
| `GET …/logs`, `GET …/logs/{snapshot}/{file}` | `LogSnapshot[]`; a log file as text |
| `GET /api/v1/site/users`, `PUT/DELETE /api/v1/site/users/{name}` | `SiteUser[]`; add / change (`PutSiteUser`, `If-Match`); remove |
| `PUT /api/v1/site/password` | `SetSitePassword`: the built-in administrator's password (it also opens the Site Manager's HTTPS API) |
| `POST /api/v1/site/tokens`, `DELETE /api/v1/site/tokens/{user}` | `IssueToken` → `IssuedToken` (`201`, shown once); withdraw a user's tokens |
| `GET /api/v1/site/pairings` | `PairingRequest[]`: clients waiting for approval, here and relayed |
| `POST /api/v1/site/pairings/{id}/approve`, `…/deny` (`DenyPairing {reason?}`) | `204` (admins); `404` when no longer waiting |
| `GET /api/v1/site/api-keys` | `SiteApiKey[]`: the site's named keys (empty at a server that isn't the site's) |
| `DELETE /api/v1/site/api-keys/{id}` | `204`: a named site key revoked on every machine (admins) |
| `GET /api/v1/site/membership` | `SiteMembership`: this server in another server's site |
| `POST /api/v1/site/join` | `JoinSite {server?, code?}` → `SiteMembership` (admins; `409` while this site has other machines, or already in another) |
| `POST /api/v1/site/leave` | `204`: leave it; this server runs its own site again (admins; `409` when not in one) |
| `GET /api/v1/site/audit?n=200` | `SiteAuditEntry[]`, newest first |
| `GET /api/v1/site/events?n=200` | the newest `SiteEvent`s |

Changes answer `503 unavailable` while this server isn't running the Site Manager (for a server in another site: make them at that site's server). A Site Manager that should run but didn't start (its ports in use at boot: `GET /site` says so in `error`) is tried again every 15 seconds, and when the site settings are saved with `enabled: true`; once it runs, `siteChanged status`. When only its API (admin panel) port is taken, it starts without the API: `running: true`, no `apiPort`, and `apiError` says why (no other port is tried). Discovery and the site's authority run, but what needs the API (pairing a client, API keys, changes run as a site user) answers errors until a start with the API port free (the next server start); Oriel shows `apiError` as a warning on the Site page.

On the WebSocket, `{"type":"siteEvent","event":{…}}` carries a typed `SiteEvent` (`name`, `level`, `text`, `at`, and `fingerprint`, `machine`, `address`, `requestId`, `codeId`, `triesLeft` where they apply). They are the Site Manager's typed events as they happen, with its names (`SiteEventName`): `join_requested`, `join_cancelled` (a request left the list: withdrawn, or answered), `device_joined`, `device_online`, `device_offline`, `join_denied`, `join_refused`, `join_withdrawn` (a machine approved by code withdrew), `device_revoked`, `join_code_used`, `join_code_wrong` (`triesLeft`), `join_code_burned` (`detail.replaced` when a newer code took its place), `join_code_expired`, the machines' own as their IDN Nodes report them (`already_in_site`, `site_manager_unreachable`, `revoked`, `two_sites`, …) and what needs attention on them; `key_requested` (a client waits for approval: `detail.pairing` its id, `machine` the server it asked at), `key_request_cancelled` (one ended unanswered), `device_left` (a machine left by itself); plus this server's own, for codes issued and taken back through this API (`join_code_issued`, `join_code_cancelled`) and for pairings waiting at this server (`key_requested`, `key_request_cancelled` with `p<n>` ids). Show the `text` of a name you don't know. `{"type":"siteChanged","parts":["joinRequests","devices"]}` says which lists to refetch (`status`, `joinRequests`, `joinCodes`, `devices`, `users`, `membership`, `pairings`, `apiKeys`, `mosaic` (`GET /mosaic`: the display boxes, their screens and the channels), `signage` (`GET /signage`), and `session` when an unlock ended by itself or the access level changed: refetch `GET /site/session`).

## Codex

**Vesper Codex** is the site's content store, a module of the Vesper server on the site's server (one Codex per site; other buildings reach it over the church VPN). Content lives in **collections** (per campus, ministry or room), each holding presentations, libraries (the sidebar shelves, unchanged: `Library`, `/api/v1/libraries`), playlists, themes, media assets, media bins and roots, global groups, and the timers, messages, props, macros, clear groups and stage layouts that slide actions refer to. Screens, looks, show settings, masks and inputs stay with each box. A box shows one **active collection**; the content routes above act on it, unchanged.

Every server with this module announces the capability `codex` in `GET /api/v1/server`, whatever its part in the site's Codex: the box that holds it and a show box alike (its part is `GET /api/v1/codex`'s `role`). Every route below exists on both; where a show box answers differently, the text says so.

### Roles and status

`GET /api/v1/codex` (`CodexStatus`) is this box's view of the site's Codex:

- `id`: the Codex's ID; `role`: this box's part in it: `host` (it holds the Codex, `codex/` in its data folder; a server on its own is the host of its own Codex), `client` (a **show box**: it shows a copy of one of the site's collections, see [Show boxes](#show-boxes)) or `dormant` (a show box whose first copy is on its way: its own Codex is asleep, kept intact on disk, read only, still shown);
- its active `collection` and `collectionName`; counts: `collections`, `documents`, `trash`, `deletedCollections`;
- `mediaLibrary` (`MediaLibraryStatus {available, problem?, served}`, see [Media](#codex-media));
- on a show box also `sync` (`CodexSync`, see [Show boxes](#show-boxes)) and `dormantCollection` (its own Codex's active collection, shown again when it leaves the site).
- on the box that hosts the Codex, `boxes` (`CodexBoxSync[]`, omitted when empty): the show boxes that followed one of its collections in the last 24 hours, learnt from their requests over the [box-to-Codex link](#box-to-codex-link): `fingerprint`, `name` (its name in the site), `collection`, `seq` (the change-log position it has been sent; `null` until it asked for changes), `behind` (entries behind the collection's newest; `null` while unknown), `lastSeen` and `online` (it asked in the last 10 minutes; a box asks every 30 s and at once after the Codex's push). A box that starts following, switches collection, comes back or moves on in the change log sends `codexChanged {parts: ["status", "collections"]}`.

`codexChanged {parts: ["status"]}` says when to read it again.

### Collections

- **Collections** (`CollectionSummary`: the `Collection` document `{id, name, description?, color?, sortKey, purpose, createdAt}`, its `revision`, `active` (this box shows it), `readOnly` (written by a newer server version), `counts` per kind, `documentBytes` on disk and `mediaBytes` of the media files its assets use, each once, and on the Codex box `boxes {showing, synced}`: the Vespers showing it (the online show boxes following it, plus the Codex box itself when it is its active collection) and how many of them have every change, for "Synced to 2 of 3 Vespers"; `{0, 0}` on a show box). Listed by sort key, then name. Create with a name (and optionally `description`, `color`, `purpose`, `sortKey`; a new one goes last); change with `If-Match` (omitted fields stay, `null` clears `description` and `color`); delete with `If-Match` (any collection but the one this box shows: `409`, assign another one first). Names are 1–200 characters; validation errors are `400` with `field`; a stale `If-Match` is `409` with `current` (the summary now).
- **Collection trash** (`GET /api/v1/codex/collection-trash`, `DeletedCollection[]`, newest first: the `collection`, its `revision`, `counts`, `documentBytes`, `mediaBytes`, `trashItems`, `deletedAt`, `by`): a deleted collection disappears from the list, from the status counts and from every other view, but stays on disk intact (its documents, change log and media references) **with no expiry**, until an admin restores it (`POST /api/v1/codex/collection-trash/{id}/restore` → `CollectionSummary`, back as it was; `409` for one whose `collection.json` can't be read, which then stays in the collection trash) or deletes it for good (`DELETE /api/v1/codex/collection-trash/{id}`, `If-Match` optional → `204`: its documents, its change log and its entries in the item trash go). While a collection is in the collection trash, its item trash entries are left out of `GET /codex/trash` (they still expire after 30 days) and can't be restored (`409`); they come back with it.
- **Assignment** (`PUT /api/v1/codex/assignment {collection, confirm?}`): the collection this box shows. The box switches at once and keeps running (like after `reloadData`: a live slide from the other collection is cleared, the screens stay), records it in `machine/codex.json` (kept across restarts), and announces every document that came or went with `documentChanged` (`removed` ones name the collection they left). While something is live on the box (a slide or announcement, background or foreground media, audio, a prop, a message), the switch is refused with `409 liveShow`, whose message names what is live (and `error.live` lists it, each item named for people, e.g. `a slide of “…”`, `the message “…”`), unless the request carries `"confirm": true`. On a show box the named collection of the site's Codex is copied first (see [Show boxes](#show-boxes)).
- **Access.** Reading needs a paired client, like every route. Content edits stay as they are (every paired client, at every access level). Creating, changing and deleting collections, restoring a deleted collection, deleting it for good and the assignment are the site's admin actions (see [Site](#site)): at `open` any paired client, at `pin` an unlocked one, at `users` a signed-in admin; otherwise `403`. They are in the site's audit (`codex.createCollection`, `codex.updateCollection`, `codex.deleteCollection`, `codex.restoreCollection`, `codex.purgeCollection`, `codex.assign`), refused ones too.
- **Writes are serialized:** every collection write (content edits included) goes through one writer, in order; `reloadData` waits for it.

### Change log and trash

- **Change log** (`GET /api/v1/codex/collections/{id}/changes?since=<seq>&limit=`, `CollectionChanges {from, to, changes, resync}`): every document added, changed or removed in the collection, oldest first, once it is on disk, as `ChangeEntry {seq, at, kind, id, change, revision?, by: {client, clientId?, machine}}` (`by.client`: the paired client's key name, `server machine`, `reload` for what `reloadData` found changed on disk, or `migration` for old-layout documents moved in and media locations rewritten at a start; `machine`: the server ID the write came in through, a show box's for its forwarded edits). `seq` grows by one per entry and is never reused. Ask with the last `to` you got (`since=0` the first time); at most `limit` (default and most 1000) entries come at once, and `to` is then the last one sent. The log is compacted when it passes 10,000 entries or its oldest is 90 days old (entries older than 90 days go, at most the newest 5,000 stay): asking from before what it still holds, or from beyond its newest `seq` (another Codex's numbers), answers `resync: true` with no changes; refetch everything and continue from `to`. A change that is on disk but whose entry couldn't be appended (a full disk, a stop in between) is appended at the next change or the next start.
- **Trash** (`GET /api/v1/codex/trash`, `TrashItem[]`, newest first: `{id, collection, kind, docId, name, deletedAt, expiresAt, by, subtree}`): every deleted presentation, library, playlist or folder, timer, message, prop and stage layout goes there for 30 days (a folder in one entry with its sub-folders and playlists, `subtree` counting them). `POST /api/v1/codex/trash/{id}/restore` recreates them in their collection with their IDs and content (a content edit: any paired client; `TrashRestored {collection, kind, documents}`; `409` when it can't, e.g. the presentation's library is gone too: restore that first, or its collection was deleted); sending a deleted document back to its create route (undo) takes it out of the trash too. `DELETE /api/v1/codex/trash` empties it for good (admins, audited as `codex.purgeTrash`; `TrashPurged {purged}`). Expired entries are removed hourly. A damaged trash index (a crash in the middle of a delete) is rebuilt from the entries' own files: no entry is lost with it.

<a id="codex-media"></a>
### Media

- **The media library.** The Codex's media files are hosted by IDN: the Codex box runs IDN's site media library in `codex/library/` (named `Codex`; a folder per collection, by its name), served to the site's machines with the Codex's own membership once it has one (`mediaLibrary.served`). Media assets refer to its items as `{"type": "library", "id", "sha256"}`: the item's lasting ID (it survives renames and moves in the library) and the SHA-256 of its file. A library IDN can't open (its index can't be read) is reported in `mediaLibrary` (`available: false`, `problem`), nothing in it is changed or recreated, its media are missing and uploads can't finish (`503 unavailable`) until it opens again (at the next start or `reloadData`). The Codex marks every item a document uses (a collection's media asset, a deleted collection's, a document in the item trash) as in use in the library (holder `vesper-codex`), at start and after every write that changes media references; the library refuses to delete a marked item unless a person types its name back. Items no document uses stay in the library unmarked: the Codex never deletes library items itself (removing media is the library owner's action, through IDN's delete and trash). Older documents' `{"type": "managed", sha256, extension}` locations (the Codex's own store before, `codex/media/`) still read; at start the Codex box moves that store into the library (resumably; files whose ending the library doesn't take stay where they are, and move in at a later start once it takes them) and rewrites those locations as `library` ones (schema 3, see [Screen configuration](#screen-configuration)). The library takes IDN's media endings and every ending Vesper plays.
- **Media uploads** (a content edit: any paired client) are resumable and deduplicated by SHA-256. `POST /api/v1/codex/uploads` with `CreateUpload {collection, name, size, sha256?, bin?, relativePath?, replaces?}` → `201 UploadInfo {id, collection, name, size, offset, sha256?, bin?, createdAt, updatedAt, expiresAt}`; the extension of `name` must be an image, video or audio type the media library takes (`400` with `field` otherwise, and for a size over the server's limit, 64 GiB by default, or a bin that isn't a manual bin of the collection). Then `PUT /api/v1/codex/uploads/{id}` sends the bytes in chunks of any size, each with `Content-Range: bytes <start>-<end>/<size>` (no body limit on this route): a chunk must start at `offset` (`409` with `UploadConflict {error, offset}` otherwise: continue from there); a chunk that breaks off keeps the bytes that came (`GET /api/v1/codex/uploads/{id}` says the offset); one longer than its range is dropped (`400`). `POST /api/v1/codex/uploads/{id}/finish` (optional `FinishUpload {sha256}`, needed when the upload didn't carry it) checks the size (`409 {offset}` while bytes are missing) and the SHA-256 (`400` with `field: sha256` on a mismatch: the upload is dropped, send the file again), takes the file into the media library under its uploaded name (made a file name every platform takes: `: ? * " < > |` become `_`, no trailing dot or space, `_` before a device name such as `CON`), in the collection's folder (or drops the bytes when the library has that file already), and answers `201 UploadFinished {collection, asset, revision, deduplicated, binItem?, existing?}`: a new `MediaAsset` with a `library` location (and a new last item of the bin, when one was named), announced with `documentChanged`; the item is marked in use. When the collection has an asset for those bytes already (the same SHA-256; not a theme's asset), no second asset is made: `asset` is that one and `existing: true` (it is pointed at the library item when its file was elsewhere, and added to the named bin unless it is in it already: `binItem` is its item there). Another collection gets an asset of its own for the same file. An upload with `replaces` relinks as below. Without an open media library the finish answers `503 unavailable` and the upload stays (finish it again later). The server then probes the file (size, duration, frame rate, codec, audio, alpha; the library's own background probe shares that probe) and makes its thumbnail at low priority; the asset's `probe` is filled with another `documentChanged` (a file that can't be read is marked `unsupported`). `DELETE /api/v1/codex/uploads/{id}` cancels; `GET /api/v1/codex/uploads` lists the open ones. An upload with no new chunk for 24 hours is dropped. Limits: a new upload must leave 5 % of the disk plus 2 GiB free after every open upload is complete, and each chunk the same (`507 insufficientStorage`); at most 64 open uploads and 4 chunks received at once (`429 rateLimited`).
- **Media files by SHA-256** (`GET /api/v1/codex/blobs/{sha256}.{ext}`): the library's item with that SHA-256 (else a file of the old managed store), with `Range` requests, `ETag: "<sha256>"` (`If-None-Match` → `304`) and a long cache lifetime (the content never changes). `GET /api/v1/codex/blobs/{sha256}.{ext}/thumbnail`: a 320 px wide JPEG (an image, or a video's frame at 10 %; a generic picture for audio), made on first request when the probe hasn't made it yet; `ETag`, `304`. A video whose frame can't be decoded (yet) gets the generic picture without an `ETag` and with `Cache-Control: private, max-age=60`, so a thumbnail made later is fetched.
- **Missing media** (content edits: any paired client). `GET /api/v1/codex/collections/{id}/media?status=online|missing|unsupported|all` (comma-separated; default `all`; sorted by name) lists `CollectionMedia {asset, revision, availability, problem?}`: `availability` (`online`, `missing`, or on a show box `copying`) is worked out on request (the file of the asset's location found on this box: the library's item is there now, a managed file of the old store, a path under a media root bound here); the asset's stored `status` is the last known one and is written only when it actually changes. Library items a collection uses that change or vanish in the library (moved, rewritten, deleted there) are announced as they happen with `codexChanged {parts: ["media"], collection}`: refetch that collection's media. Every relink appends a `RelinkRecord {method, previous, at}` to the asset's `relinks`, sets its `status`, is announced with `documentChanged` (and probed again):
  - **manual**: `POST /api/v1/codex/collections/{id}/media/{asset}/relink` with `RelinkTarget` `{"type": "library", id}` (an item of the media library; `400` with `field: id` otherwise, `409` while the library is still reading its SHA-256), `{"type": "blob", sha256, extension}` (a media file the Codex has by its SHA-256: the library's item with those bytes, else a managed file of the old store; `400` with `field: sha256` otherwise) or `{"type": "linked", root, path}` (a root bound here or among the collection's media roots, a relative path that stays below it; `400` with `field` otherwise) → `RelinkedAsset {asset, revision}`; or an upload with `CreateUpload.replaces: <asset>`, whose `finish` relinks that asset to the uploaded file (`UploadFinished.relinked: true`) instead of making a new one;
  - **search**: `POST …/media/search` (`RelinkSearch {assets?, dryRun?}`; default: every missing asset) tries, in order, for library and managed media the library's item with the same SHA-256 (`contentHash`: the bytes are in the library under another item), then under the media roots bound on this box the same relative path under a bound root (method `rootBinding`; the asset's own root first), then for library and managed media a file with the same SHA-256 (`contentHash`, files of the same name first), else a file with the same name found exactly once (`fileName`) → `RelinkReport {checked, relinked: [{asset, name, method, location}], ambiguous: [{asset, name, candidates}], notFound, statusChanged}`;
  - **client folder match**: `POST …/media/match` (`MediaMatchRequest {files: [{name, size, relativePath?}], assets?}`) → `MediaMatchReport {matches: [{asset, name, file, by}], ambiguous, unmatched}`: a file whose relative path ends like the asset's known path (at least a folder and the name, `by: relativePath`), else the only file with its name (`fileName`). Nothing changes: upload each match with `replaces`;
  - **bulk re-root**: `POST …/media/reroot` (`RerootRequest {from: {root?, prefix}, to: {root, prefix}, onlyExisting (default true), dryRun?}`): every asset linked under `from.root` in the folder `from.prefix` (or, without `from.root`, whose original absolute path starts with `from.prefix`, ignoring case and slash direction) gets the same rest of its path under `to.root` and `to.prefix` (method `rootBinding`), only where that file exists on this box unless `onlyExisting: false` → `RelinkReport`.
- **Storage** (`GET /api/v1/codex/storage`, `CodexStorage`): the `disk` (`total`, `available`, `keepFree`), the media `library`'s own figures (`LibraryUsage {bytes, items, trashItems, trashDays}`: its items and its trash), per collection (`CollectionStorage`: `documents`, `documentBytes`, `mediaBytes` of the library items its assets use, `uniqueMediaBytes` of those only it uses, `assets`, `missing`: assets whose file can't be found on this box), every library item (`media`), those used by more than one collection (`shared`), those no document uses (`unreferenced`: not marked in use; the Codex never deletes them), files left in the old managed store (`managed`), the trashes (`trash`: item trash entries and bytes, deleted collections and their bytes, `mediaBytes` of library items used only by trashed documents and deleted collections), stored thumbnails and uploads in progress (each `FileUsage {files, bytes}`). Thumbnails of media the Codex no longer has are removed hourly.

### Search

**Search** (`GET /api/v1/codex/search?q=&collection=&kinds=&limit=`, any paired client → `SearchResults {hits, total}`): the documents matching `q`, best first, from an in-memory index the server keeps up to date with every change (a search always sees the documents as they are, its own edits included). Scope: this box's active collection by default, `collection=<id>` one collection (`404` when it isn't in the Codex, e.g. in the collection trash), `collection=all` every collection (never the collection trash or the item trash). `kinds` (comma-separated) narrows it to some of what is searched: `presentation` (name, details: song title, artist, authors, publisher, CCLI number, category, scripture reference; group names, slide labels, slide text, slide and presentation notes), `library`, `playlistNode` (playlists and folders), `mediaAsset` (name, the original file reference), `theme` (name, template slide names), `prop` (name, slide text), `message` (name, text), `timer`, `macro`, `stageLayout` (names). `limit`: 1–500, default 50; `total` counts every match. Matching ignores case and the accents of Latin letters (`é` = `e`, `ß` = `ss`), and apostrophes inside words (`don't` = `dont`); every word of `q` must match the start of a word somewhere in the document (`gra` finds "Grace"); `"…"` is a phrase: whole words, in that order, in one field (while its closing quote is missing, the last word is matched as a start). Names rank first (a name that is the query, then one that starts with it, then names containing the words), then details, groups, labels and template names, then origins, slide text and notes; whole words before starts of words; equal ones by name. A hit (`SearchHit`) has the document's `kind`, `id`, `collection` and `collectionName`, its `title`, the `field` the snippet comes from (`name`, `details`, `group`, `label`, `text`, `notes`, `template`, `origin`), the `snippet` (`SnippetPart[]`: pieces of text, the matching words with `mark: true`; line breaks shown as ` / `, `…` for text left out) and, for a presentation, where: `slideIndex` (0-based over all slides in document order, as `SlidePosition`; a group's first slide), `slide` and `section` (for a theme's template slide only `slideIndex`). `400` with `field` for a missing or too long `q` (over 500 characters), an unknown `kinds` entry, a bad `limit` or `collection`. On a show box, see [Show boxes](#show-boxes).

<a id="settings-backups"></a>
### Settings backups and restore

- **Settings backups** (kept on the box that holds the Codex; on a show box the backup routes answer `409 codexElsewhere` naming the Codex box, but `POST …/backups/snapshots`, which offers the show box's own settings, see [Show boxes](#show-boxes)). The Codex keeps **backup sets**, one per product and machine: Vesper's per box (`product: "vesper"`, `machine`: its server ID), Oriel's per paired client (`"oriel"`, its key's id; `local` for the server machine's own clients). A set (`BackupSet {id, product, machine, name, fingerprint?, createdAt}`) holds **snapshots** (`SnapshotSummary {id, set, seq, takenAt, reason, by, digest, items, bytes}`; `reason`: `scheduled`, `change`, `manual`, `beforeRestore`, `upload`), each a list of items (`SnapshotItem {key, kind, id?, name?, schemaVersion, sha256, size}`) whose bytes are kept once by their SHA-256; every item carries its own `schemaVersion`. A snapshot holding the same items as the set's newest isn't made again: that one is answered (`SnapshotTaken {snapshot, created: false}`). A Vesper box's snapshot holds its own settings: screens, looks, show settings, masks, video and audio inputs (`portable/<kind>/<id>.json`), its output, input and media root bindings, audio device and font substitutions (`machine/<kind>/<id>.json`) and its name and server ID (`machine/server.json`); **never** device identities (`machine/site-member/`, `machine/codex-member/`, the Site Manager's folder), `machine/site.json` (the PIN hash), `clients.json`, tokens, keys, `cache/`, `machine/codex.json` (the box's Codex role and assignment), fonts, media or collection content. This box's settings are snapshotted daily, about 10 minutes after they last changed (looked at every minute), and on demand (`POST /api/v1/codex/backups/snapshots`, admins, audited as `codex.backupNow` → `201 SnapshotTaken`, `200` when nothing changed). Kept per set: the newest 10, plus the newest of each of the 14 most recent days that have snapshots and of each of the 8 most recent weeks; blobs no snapshot names go. A show box offers its own settings to the site's Codex over the box-to-Codex link on the same schedule (and on demand: `POST …/backups/snapshots` on the show box is sent there); the Codex takes an offer only for the device it came from (never the Codex box's server ID, nor a server ID whose set another device of the site offered, unless the site lists that device as removed). `GET /api/v1/codex/backups` (`BackupSetSummary[] {set, snapshots, latest?, thisBox, yours}`) and `GET …/backups/{set}/snapshots` (newest first) are for every paired client; a snapshot's contents (`GET …/snapshots/{snapshot}` → `SnapshotContents {snapshot, items: [{item, content}]}`) are the site's admin actions, as are the preview and restore.
- **Restore** (admins). `GET …/backups/{set}/snapshots/{snapshot}/preview` → `RestorePreview {snapshot, currentAsOf?, changes, unchanged, summary, restart}`: per item what restoring would do to the box (`RestoreChange {key, kind, id?, name?, change: added|changed|removed, restart?}`; `removed`: the box has it, the snapshot doesn't), compared with the box it would be restored onto: `?target=<server ID>` as in `RestoreRequest.target` (absent: the set's own box). Onto this box: its settings now; onto a show box: that box's newest snapshot here (`currentAsOf`; `409` for a box that never offered its settings). And in words (`"3 screens, 2 looks changed; 1 output binding added"`). `POST …/snapshots/{snapshot}/restore` (`RestoreRequest {items?, adoptServerId?}`, audited as `codex.restore`) restores every item (what the snapshot doesn't hold is removed) or the chosen ones by `key` (`400` with `field: items` for a key neither side has). All of it or nothing: should a file fail to be written, those written before are put back (`409`). First a `beforeRestore` snapshot of the box's settings is taken (or the newest is answered when it is the same): restoring it is the undo. The files are written (never over a document from a newer schema) and the data reloaded as `reloadData` does: screens, looks, outputs, show settings and the audio device apply at once, without a restart; the server's name and font substitutions only after a restart (`RestoreResult {beforeRestore, applied, summary, restart, serverIdAdopted?}`, `restart` names them). The server ID stays; a restore never touches device identities. `RestoreRequest.target` names the box to restore onto by its server ID (absent: the set's own box). Onto the Codex box itself the restore runs at once (`200 RestoreResult`). Onto a **show box** it is **queued**: `202 QueuedRestore {id, set, snapshot, target, targetName, items?, adoptServerId?, state, queuedAt, updatedAt, expiresAt, by, error?, result?}`; the Codex tells the box at once and the box takes it (it also asks at each poll, every 30 s), offers its `beforeRestore` snapshot (the undo), writes the files and reloads, and reports back. `state`: `queued`, `applying`, `applied` (`result` is the `RestoreResult`, `serverIdAdopted` for a replacement box), `failed` (`error` says why), `expired` (the box didn't ask within 24 hours), `cancelled`. `GET /api/v1/codex/backups/restores` lists them (newest first), `GET …/restores/{id}` reads one (both: any paired client, not audited), `DELETE …/restores/{id}` cancels one still `queued` (admins; audited as `codex.cancelRestore`); `codexChanged {restores}` when one moves on. One restore per box waits at a time (`409` for a second); the snapshot a queued restore waits to restore is never pruned meanwhile; a box that never offered its settings (no set with its site fingerprint) can't be a target (`409`). A **replacement box** restoring an old box's set may take over the old box's server ID (`adoptServerId: true`), only once the site lists the old box as removed (`409` otherwise, also for a box that wasn't a machine of the site); the IDN device identity is always the new box's own. Oriel's sets are restored by Oriel (`409`).
- **Oriel's preferences** (each client its own, any paired client): `PUT /api/v1/codex/backups/oriel` with `OrielPreferences {schemaVersion, preferences}` (a JSON object, at most 1 MiB; `400` with `field: preferences` otherwise) → `201 SnapshotTaken` (`200` when unchanged); `GET /api/v1/codex/backups/oriel[?snapshot=]` → `OrielPreferencesBackup {schemaVersion, preferences, snapshot}`, the newest or the given snapshot's (`404` none). **Taking over another computer's preferences:** `GET /api/v1/codex/backups/oriel?set=<set>[&snapshot=]` answers another Oriel set's (any paired client, no admin needed; `404` for a set that isn't Oriel's: a box's settings stay the admins').

<a id="codex-export"></a>
### Export and import

The Codex's protection without replicas (admins only, but the reads `GET …/export/settings` and `GET …/export/scheduled`, which need a paired client like the site's other reads and are not audited; capability `codexExport`; on a show box `409 codexElsewhere`). **Documents only, never media:** an export holds every collection (its documents and change log) and the settings backups (sets, snapshots, their blobs), and on the site's server the site's Site Manager folder (always in a scheduled export, in a manual one only when asked); **media files are not in it, so the media folder needs its own backup** (e.g. a copy of the Codex's library folder on a NAS). Media references stay as they are (library items by ID and SHA-256): after an import they are missing until their files are back, then the relink search (`POST …/media/search`) finds them by SHA-256, in the media library or under a media root. Every answer about exports says so (`mediaIncluded: false`, `note`; the download's `X-Codex-Media-Included: false` and `X-Codex-Export-Note` headers).

- **The file** is an IDN backup (`idn_open_protocol::backup`, kind `vesper-codex`; Argon2id and ChaCha20-Poly1305 with the passphrase; at most 512 MB of files, more answers `409`) holding `export.json` (`ExportManifest {format: "vesper-codex", formatVersion, createdAt, reason, codexId, serverId, serverName, version, collections: [ExportedCollection {id, name, documents, seq, lastChangeAt?}], backupSets, snapshots, site, mediaIncluded}`), `codex/codex.json`, `codex/collections/<id>/…`, `codex/backups/…` and, on the site's server (a scheduled export always; a manual one only with `includeSite`), `site/site-manager.idnbackup`: the Site Manager's folder as IDN's own site backup makes it (kind `site-manager`, without what IDN leaves out), with the same passphrase. Not in it: the item and collection trashes, uploads, thumbnails, the media library, anything of `machine/`. Passphrases have at least 8 characters (`400` with `field: passphrase`).
- **Manual export:** `POST /api/v1/codex/export` with `CreateExport {passphrase, includeSite?}` (its own passphrase, never the stored one) → the file (`application/octet-stream`, `Content-Disposition: attachment; filename="vesper-codex-<date>.vcodex"`). **The site's identity only when asked:** `includeSite: true` adds the Site Manager's folder (manifest `site: true`); absent or `false` (the default) leaves it out, even on the site's server. Asked on a box that doesn't run its site's Site Manager it answers `409`. Oriel offers it as a tick box "Include the site's identity", off by default (owner, 2026-11-23). Scheduled exports on the site's server always include it.
- **Scheduled exports:** nightly at `time` (the Codex box's local time, `02:00` by default) into `folder` (an absolute path on the Codex box: a USB drive, a NAS mount; default `<data>/codex/exports/`), encrypted with a passphrase an admin stored, keeping the newest export of each of the `keepNightly` most recent days that have exports (7) plus of each of the `keepWeekly` most recent weeks (4); only files named `vesper-codex-<date>.vcodex` are pruned. A box that was off at the time exports when it is back. `GET /api/v1/codex/export/settings` → `CodexExportStatus {settings: CodexExportSettings {enabled, folder?, time, keepNightly, keepWeekly}, folder, passphraseSet, active, problem?, last?, nextAt?, mediaIncluded, note}`; `PUT` with `UpdateExportSettings` (the fields to change; `folder: ""` the default; `400` with `field`). **The stored passphrase** (`PUT /api/v1/codex/export/passphrase` `SetExportPassphrase {passphrase}`, `DELETE` clears it; both → `CodexExportStatus`) is kept on the Codex box only (`machine/`, readable by the server's user alone: mode 0600 on Linux; on Windows a protected ACL for SYSTEM, the Administrators and the server's account), never answered back (only `passphraseSet`), never logged or audited, never in a settings snapshot or an export. **Without one no scheduled export runs** (`active: false`, `problem` says why). A scheduled export that failed (the folder can't be written, a drive gone; each try first writes a small file into the folder) is tried again 30 minutes later, or at once when the settings or the stored passphrase change; `nextAt` says when. `GET /api/v1/codex/export/scheduled` → `ScheduledExport[] {file, createdAt, bytes}`, newest first. `codexChanged {parts: ["exports"]}` tells clients to read both again: after every scheduled export (made or failed: `last`, `nextAt`, `problem`), and when the settings or the stored passphrase change.
- **Import:** upload the file (`POST /api/v1/codex/export/imports`, the file as the body → `201 ImportUpload {id, bytes, expiresAt}`; kept an hour, `DELETE …/imports/{id}` drops it), preview it (`POST …/imports/{id}/preview` with `OpenImport {passphrase}` → `ImportPreview {id, manifest, sealedAt, collections: [ImportCollection {collection, comparison, here?, active, tooNew}], backupSets: [ImportBackupSet {id, product, machine, name, snapshots, present}], site, mediaIncluded, note}`; `comparison`: `new`, `same`, `newer` / `older` (by the newest change), `inTrash` (in this Codex's collection trash: restore it or delete it for good first)), then apply it (`POST …/imports/{id}/apply` with `ImportRequest {passphrase, replace?, skip?, backups?, site?}` → `ImportResult {added, replaced, skipped, backupSets, snapshots, siteStaged, restartNeeded, mediaIncluded, note}`; the upload is used up). A wrong passphrase or a damaged file answers `400` with `field: passphrase`; a file that isn't a Codex export `400`; one from a newer format `409`. **Defaults:** collections not in this Codex are added (unless in `skip`); a collection that is here is kept unless named in `replace` (not the one this box shows: `409`, assign another first): its current copy goes to the collection trash first, as a collection of its own named "… (before import)" (restoring it is the undo), the collection keeps its ID and its change log, which logs every document the import added, changed or removed (`documentChanged` with `collection`; show boxes following it catch up as after any edit); settings backup sets this Codex lacks (by ID and by product and machine) are added with their snapshots (`backups: false` leaves them), sets it has are kept. New collections come with their change log as exported; the Codex keeps its own ID, so show boxes that followed the exported Codex compare everything once. `codexChanged` (`status`, `collections`, `collectionTrash`, `backups`) as for any change; the search covers the imported collections at once.
- **The site's identity** (`site: true`, on the site's server only; `409` elsewhere, `400` with `field: site` for a file without one): the file's Site Manager folder is staged (IDN's `backup::stage`) and **takes the site's place at the server's next start** (the Site Manager runs IDN's `apply_staged` before it opens its folder; IDN refuses it, setting it aside, when machines joined the site since, or after a week): `siteStaged: true`, **`restartNeeded: true`**. Audited as `codex.export`, `codex.exportSettings`, `codex.exportPassphrase` (whether set, never the passphrase), `codex.importUpload`, `codex.importPreview`, `codex.import`, `codex.importDrop`.

### PowerPoint import

A `.pptx` becomes a presentation in a collection (a content edit: any paired client, like uploads; on a show box `409 codexElsewhere`). **Keynote and Google Slides** come in as `.pptx` exported from them (Keynote: File ▸ Export To ▸ PowerPoint; Google Slides: File ▸ Download ▸ Microsoft PowerPoint). **Editable where possible, a picture of the slide as fallback** (owner, 2026-11-25):

- **Editable:** one slide per slide; text boxes and text placeholders become text elements (position, size, rotation, flips; font, size, colour, bold, italic, underline, strikethrough, caps, super/subscript, letter spacing; alignment, indents, bullets and numbering, line and paragraph spacing; insets, vertical anchor, shrink-to-fit; text shadow), pictures become picture elements (crop, stretch) whose files are taken into the collection like uploads (deduplicated by SHA-256, named `<presentation> <file>`), rectangles, rounded rectangles, ellipses, isosceles and right triangles and diamonds become shapes with their solid or gradient fill, outline and shadow; groups are flattened into place. Theme fonts (`+mj-lt`, `+mn-lt`) and colours (through the master's colour map, with their tint, shade, luminance and alpha) are resolved, and text inherits slide → layout placeholder → master placeholder → master text styles → the presentation's default text style. The layout's and master's own decorations are drawn under the slide's shapes (unless the slide or layout hides master shapes). Backgrounds: a colour becomes the slide's background colour, a gradient or picture a full-canvas element at the bottom. Notes become slide notes, PowerPoint sections become sections (else one section, "Slides"), hidden slides come in disabled, the title becomes the slide's label. The canvas is 1920 px wide, its height from the slide size (16:9 → 1080, 4:3 → 1440). Provenance: `import.source: powerPoint` on the presentation (with the file name as `documentPath`) and its slides.
- **Pictured:** what the model can't hold (charts, tables, SmartArt, embedded objects, freeforms and other shapes, lines and connectors, pattern and tiled fills, pictures in formats Vesper doesn't show such as EMF/WMF) is drawn from a **picture of the slide**, cut (a media fill's `crop`) to that object's box at its place in the stack, so the rest of the slide stays editable. The pictures come from **LibreOffice** (headless `soffice`, an optional program on the server: see `docs/PACKAGING.md`), one run for all slides that need one, each slide converted as its own one-slide package, with its own profile and a time limit; else, for the first slide only, the thumbnail the file carries (small); else the object is **approximated** (a shape as a rectangle with its text) or **left out**. The import never waits on or fails for the pictures. Server options: `--soffice FILE` (or `VESPER_SOFFICE`), `--no-slide-pictures`.
- **Not imported:** animations and transitions, video and sound (their poster picture is), linked (not embedded) pictures, vertical text and columns (shown as horizontal text), a rotated group's rotation; each is a note in the report.

`POST /api/v1/codex/imports/pptx?collection=&library=&name=&fileName=` with the file as the body (at most 1 GiB) → `202 PptxImportJob {id, collection, fileName, state, done, total, error?, report?, createdAt}`. `collection` defaults to the box's active one; `library` must be one of its libraries (default: the collection's **Imported Slides** library, made when missing); `name` defaults to the file's name. `400` with `field`: `fileName` for a Keynote file (`.key`: export it as `.pptx`), an old `.ppt` (save it as `.pptx`) or another ending (`.pptx`, `.ppsx`, `.potx`, `.pptm`, `.ppsm` are taken), `body` for a file that isn't a zip package, `collection`, `library`, `name` (over 200 characters). One import runs at a time and at most one more waits (each holds its whole file): a third answers `429 rateLimited` until one of them has finished; `state` goes `queued` → `reading` (`done` of `total` slides) → `pictures` → `media` (`done` of `total` files) → `saving` → `done` with `report`, or `failed` with `error` (nothing saved; media already taken in stay in the media library, unused). `GET /api/v1/codex/imports/pptx` lists the jobs (the last 20, newest first; kept in memory only), `GET …/pptx/{id}` one (`404`). The report (`PptxImportReport`): `presentation`, `name`, `library`, the canvas `width` / `height`, per slide (`PptxSlideReport`) its `number`, `title`, `editable` elements, the objects `pictured`, `approximated` (or left out) and other `notes` (each `PptxImportNote {kind, name?, detail?}`), `picture` (`libreOffice` or `thumbnail`) and `hidden`; totals `editableSlides` (nothing pictured, approximated or left out), `picturedSlides`, `media` files taken in, the `renderer` used and `rendererProblem` (why pictures couldn't be made when some were needed).

### Show boxes

A server that joins another server's site (see [Site](#site)) whose server hosts the Codex becomes a **show box**. It finds the site's Codex (the site's media library named `Codex`, then the link's `hello`), puts its own Codex to sleep (`role: dormant`: kept intact on disk, read only, still shown while the first copy arrives), copies the Codex's first collection (or the one assigned to it) into `codex-cache/`, and from then on shows that copy (`role: client`; its own Codex's active collection in `dormantCollection`). Leaving the site (or being removed from it) wakes the box's own Codex: `role: host` again, its collection shown again (the copy stays in `codex-cache/`).

- **Following the Codex.** The box follows the collection's changes as they happen (the Codex tells it; it also asks every 30 s), and each applied change is announced like an edit (`documentChanged` with `collection`; the show follows: an edited live slide re-renders, a deleted one stays on screen until the next trigger). If the Codex can't be reached the box keeps showing what it has and says so in `sync`.
- **Media.** Live output always plays from the box's own disk and never waits on the network: the media library items the box's playlists and media bins use (directly, or through the presentations they hold) are copied to it ahead of time (IDN's library cache, `codex-cache/media/`, 32 GiB by default, `vesper --codex-cache-gib`; unscheduled copies are removed, least recently used first, when it needs room); anything else is copied when a cue that uses it is triggered (or its presentation becomes the current one). Until a file is on the box its `availability` is `copying` (`GET /codex/collections/{id}/media`, media bin items), the cue behaves like missing media, and triggering it sends a `still copying` notice.
- **Sync status** (`CodexSync`, `GET /codex`'s `sync`): `reachable` (the Codex answered the last exchange), `serverId`, `name` (once heard from) and `fingerprint` of the Codex's box, `lastSync` (RFC 3339), `seq` (the change-log position the box's copy is at), `codexSeq` (the Codex's, as last heard), `behind`, `problem?` (why it isn't following: the Codex unreachable, a collection no longer there, a Codex writing a newer schema than the box reads; a document of such a schema isn't copied: the box keeps showing its older copy and fetches it once it is updated), and `media` (`CodexCacheStatus {pinned, cached, copying, missing, failed, bytes, capBytes}`). `codexChanged {parts: ["status"]}` when it changes meaningfully (reachability, positions, problem, the media counts; not on every byte copied).
- **Write-through edits.** Content edits on a show box go through to the site's Codex: the routes of libraries, presentations, playlists, themes, media bins, global groups, timers, messages, props and stage layouts that change something, the item trash (`GET /codex/trash` and restoring from it) and the relinks (`…/media/{asset}/relink`, `…/media/match`, `…/media/reroot`) are forwarded over the [box-to-Codex link](#box-to-codex-link) as they came (the `If-Match`, `Content-Type` and `X-Client-Id` headers, the body), with the client that sent them (its key's id and name) and the box (its server ID). The Codex runs them as a local REST request of that client on the collection the box shows: the same validation and errors, `If-Match` checked against the Codex's revision (`409` with `current`, unchanged, when another box or client changed the document meanwhile), and the change log names the client and the show box (`by.machine`). The Codex's answer comes back unchanged (status, body, `ETag`), and only after the box's copy has taken the change, so the client reads its edit from the box at once (with `documentChanged` as for any change of the copy). While the Codex can't be reached these routes answer `503 unavailable` (the message says edits need the site's server); the show keeps playing from the box's copy; nothing is queued. A forwarded request waits at most 20 seconds for the Codex's writer (a long import or export holds it) and is then answered `503 unavailable` (busy) without being applied, before the box stops waiting (30 seconds): a `503` means the edit was not made, so trying again can't make it twice.
- **Uploads and Codex management go to the box that holds the Codex** (Oriel connects there for them): on a show box `…/codex/uploads…`, creating, changing or deleting collections, restoring or purging a deleted collection, emptying the trash, the relink search under media roots, the settings backup routes (lists, snapshots, preview and restore, queued restores, Oriel's preferences: send them to the Codex box; `POST …/backups/snapshots` still offers the show box's own settings) and the export and import routes answer `409 codexElsewhere` with `ErrorResponse.codex` = `CodexBox {serverId, name?, fingerprint?, addresses}` (`addresses`: the Codex box's REST base URLs the show box knows, e.g. `http://10.0.0.5:7878`; empty until the box has heard from the Codex since it started; `503 unavailable` without `codex` while the box doesn't know it at all).
- **Search** covers the box's copy of its collection itself; `collection=all` or another collection is asked of the Codex (`503` while it can't be reached).
- **Assignment.** `PUT /codex/assignment` copies the named collection of the site's Codex first (`400` with `field` for one the Codex doesn't have, `503` while the Codex can't be reached), then shows it as on the Codex box (`409 liveShow` unless confirmed).

<a id="box-to-codex-link"></a>
### The box-to-Codex link

Server to server, over IDN's file channel `channel/1` between the site's machines; not for clients.

- **The Codex's own site membership.** On the Codex box (the site's server), the Codex service joins its own site as an App member named `Codex` when the Site Manager starts (approved there at once; its identity in `machine/codex-member/`, kept across restarts and never in a settings backup). It is listed among the site's devices as `builtIn: true` (shown as "Codex (built-in)", never removable: `DELETE /api/v1/site/devices/{fingerprint}` answers `409`; owner, 2026-11-23) and serves the link; it doesn't count as another machine of the site (the box may still join another site, and the membership is put away while it is in one).
- **Requests.** On the Codex box one channel with the Codex's own membership answers Vesper's requests (`vesper.codex.hello`, `vesper.codex.changes`, `vesper.codex.index`, `vesper.codex.docs`; JSON, versioned) and the media library's; only devices the site lists are answered. The Codex sends `vesper.codex.changed` to the show boxes following a collection after each change (to the boxes that asked about it in the last 10 minutes), and `vesper.codex.restore_queued` to a box a settings restore waits for. A show box reports its clients and takes edit locks site-wide (`vesper.codex.presence`, `vesper.codex.lock`; the Codex pushes `vesper.codex.presence_changed`), uploads its own dormant collection (`vesper.codex.adopt`, `adopt_docs`, `adopt_finish`, `upload_begin`, `upload_chunk`, `upload_finish`; below), forwards its content writes (`vesper.codex.write`) and searches beyond its collection (`vesper.codex.search`), and offers its settings backups and takes queued restores (`vesper.codex.backup_offer`, `pending_restore`, `restore_begin`, `restore_result`).
- **Documents** travel as their files (the show box keeps them byte for byte, by SHA-256); the first copy (and a resync) compares the Codex's index with the box's files and fetches only what differs.
- **Firewalls.** Each box's channel listens on a UDP port the site's directory tells the others (a free one of IDN's media range by default). Where firewalls sit between buildings, the VPN must be routed without NAT, every box must reach the site's server's discovery TCP port, and boxes must reach each other's channel UDP port: set a fixed port on every box (`vesper --codex-channel-port <port>`) and open it for UDP.

### Routes

| Route | |
|---|---|
| `GET /api/v1/codex` | `CodexStatus` |
| `GET /api/v1/codex/collections` | `CollectionSummary[]` |
| `POST /api/v1/codex/collections` | `CreateCollection {name, description?, color?, purpose?, sortKey?}` → `201 CollectionSummary` (`ETag`; admins) |
| `GET /api/v1/codex/collections/{id}` | `CollectionSummary` (`ETag`) |
| `PUT /api/v1/codex/collections/{id}` | `UpdateCollection` (`If-Match`) → `CollectionSummary` (admins) |
| `DELETE /api/v1/codex/collections/{id}` | `204`: into the collection trash (`If-Match`; `409` for the one shown here; admins) |
| `GET /api/v1/codex/collection-trash` | `DeletedCollection[]` |
| `POST /api/v1/codex/collection-trash/{id}/restore` | `CollectionSummary` (admins) |
| `DELETE /api/v1/codex/collection-trash/{id}` | `204`: gone for good (`If-Match`; admins) |
| `PUT /api/v1/codex/assignment` | `SetAssignment {collection, confirm?}` → `CodexStatus` (admins; `400` with `field` for a collection that isn't there; `409 liveShow` while something is live, unless confirmed; on a show box `503` while the site's Codex can't be reached) |
| `GET /api/v1/codex/collections/{id}/changes?since=&limit=` | `CollectionChanges` |
| `GET /api/v1/codex/trash` | `TrashItem[]` |
| `POST /api/v1/codex/trash/{id}/restore` | `TrashRestored` (`404` gone, `409` can't) |
| `DELETE /api/v1/codex/trash` | `TrashPurged` (admins) |
| `GET`, `POST /api/v1/codex/uploads` | `UploadInfo[]`; `CreateUpload` → `201 UploadInfo` |
| `GET`, `PUT`, `DELETE /api/v1/codex/uploads/{id}` | `UploadInfo`; a chunk (`Content-Range`) → `UploadInfo` (`409 UploadConflict`); cancel → `204` |
| `POST /api/v1/codex/uploads/{id}/finish` | `FinishUpload` → `201 UploadFinished` |
| `GET /api/v1/codex/blobs/{sha256}.{ext}`, `…/thumbnail` | the file (`Range`, `ETag`); its 320 px JPEG |
| `GET /api/v1/codex/storage` | `CodexStorage` |
| `GET /api/v1/codex/collections/{id}/media?status=` | `CollectionMedia[]` |
| `POST /api/v1/codex/collections/{id}/media/{asset}/relink` | `RelinkTarget` → `RelinkedAsset` |
| `POST /api/v1/codex/collections/{id}/media/search` | `RelinkSearch` → `RelinkReport` |
| `POST /api/v1/codex/collections/{id}/media/match` | `MediaMatchRequest` → `MediaMatchReport` |
| `POST /api/v1/codex/collections/{id}/media/reroot` | `RerootRequest` → `RelinkReport` |
| `GET /api/v1/codex/library?q=&kind=&limit=` | `LibraryItems {items: LibraryItem[] {id, path, title, kind, size, sha256?, duration?}, total}`: the media library's items by path, for a relink to any of them (`{"type": "library", id}`); `q`: words of the path or title (any case), `kind`: `video`, `picture` or `sound`, `limit` 1–5000 (500); `400` with `field`; `503` while the library isn't open; on a show box `409 codexElsewhere` |
| `GET`, `POST`, `DELETE /api/v1/codex/dormant/upload` | `DormantUpload`; `StartDormantUpload` → `202 DormantUpload` (admins); cancel → `204` (admins). On a show box with a dormant collection (`404` otherwise) |
| `GET /api/v1/codex/search?q=&collection=&kinds=&limit=` | `SearchResults` (`400` with `field`; `404` for a collection that isn't there) |
| `GET /api/v1/codex/backups` | `BackupSetSummary[]` |
| `POST /api/v1/codex/backups/snapshots` | this box's settings now → `201 SnapshotTaken` (`200` unchanged; admins) |
| `GET /api/v1/codex/backups/{set}/snapshots` | `SnapshotSummary[]`, newest first |
| `GET /api/v1/codex/backups/{set}/snapshots/{snapshot}` | `SnapshotContents` (admins) |
| `GET /api/v1/codex/backups/{set}/snapshots/{snapshot}/preview` | `RestorePreview` (`?target=`) (admins) |
| `POST /api/v1/codex/backups/{set}/snapshots/{snapshot}/restore` | `RestoreRequest` → `RestoreResult` (this box), `202 QueuedRestore` (a show box; admins) |
| `GET /api/v1/codex/backups/restores` | `QueuedRestore[]`, newest first (admins) |
| `GET`, `DELETE /api/v1/codex/backups/restores/{id}` | one; cancel while `queued` (admins) |
| `GET`, `PUT /api/v1/codex/backups/oriel` | `OrielPreferencesBackup` (`?snapshot=`); `OrielPreferences` → `SnapshotTaken` (the client's own) |
| `POST /api/v1/codex/export` | `CreateExport {passphrase, includeSite?}` → the encrypted file (admins; the site's identity only with `includeSite`, `409` where the Site Manager doesn't run) |
| `GET`, `PUT /api/v1/codex/export/settings` | `CodexExportStatus`; `UpdateExportSettings` → `CodexExportStatus` (admins) |
| `PUT`, `DELETE /api/v1/codex/export/passphrase` | `SetExportPassphrase` → `CodexExportStatus`; clear it → `CodexExportStatus` (admins) |
| `GET /api/v1/codex/export/scheduled` | `ScheduledExport[]`, newest first (admins) |
| `POST /api/v1/codex/export/imports` | the file as the body → `201 ImportUpload` (admins) |
| `DELETE /api/v1/codex/export/imports/{id}` | `204` (admins) |
| `POST /api/v1/codex/export/imports/{id}/preview` | `OpenImport {passphrase}` → `ImportPreview` (admins) |
| `POST /api/v1/codex/export/imports/{id}/apply` | `ImportRequest` → `ImportResult` (admins) |
| `POST /api/v1/codex/imports/pptx?collection=&library=&name=&fileName=` | the `.pptx` as the body → `202 PptxImportJob` (see [PowerPoint import](#powerpoint-import)) |
| `GET /api/v1/codex/imports/pptx`, `GET …/pptx/{id}` | `PptxImportJob[]`, newest first; one |

### Events

On the WebSocket, `{"type":"codexChanged","parts":["status","collections"],"collection":"…"}` says what to refetch, and `collection` names the collection concerned when there is one:

| part | refetch | when |
|---|---|---|
| `status` | `GET /codex` | the active collection, the counts, a show box's sync |
| `collections` | `GET /codex/collections` | a collection added, changed or deleted, or its counts changed |
| `trash` | `GET /codex/trash` | the item trash changed |
| `collectionTrash` | `GET /codex/collection-trash` | a collection deleted, restored or deleted for good |
| `media` | `GET /codex/collections/{id}/media` | library items the collection uses changed or vanished |
| `backups` | `GET /codex/backups` | a settings snapshot taken or removed |
| `restores` | `GET /codex/backups/restores` | a queued restore moved on |
| `exports` | `GET /codex/export/settings`, `GET /codex/export/scheduled` | a scheduled export made or failed; the export settings or the stored passphrase changed |

`documentChanged` carries `collection` for the documents of a collection (absent for the box's own: screens, looks, outputs). `cargo run -p vesper-server --example ws_probe -- --codex` prints a box's Codex status and collections and then these events as they come (see [Trying it](#trying-it)).

### Uploading a show box's own collection

A box that joined a site with a Codex keeps its own Codex dormant (`dormantCollection` in `GET
/codex`). An admin can upload that collection to the site's Codex as a **new** collection (G15;
`docs/MEDIA-BINS-CODEX-PLAN.md`):

- `GET /api/v1/codex/dormant/upload` → `DormantUpload {state, source, name, songs, collection?,
  collectionName?, documents, media, bytes (TransferCount {done, total}), error?, startedAt?,
  finishedAt?}`: `state` `idle` is the offer; `copying`, `done`, `failed` (with `error`),
  `cancelled`. `404` on a box without a dormant collection.
- `POST /api/v1/codex/dormant/upload` `StartDormantUpload {name?}` → `202 DormantUpload`
  (admins; `503` while the site's Codex can't be reached). Starting again resumes: the same
  collection on the Codex (never a second one), only what isn't there yet. A job belongs to
  the site's Codex it was started for: after the box leaves the site (or follows another
  site's Codex) the offer is `idle` again, and media sent to a collection the Codex no longer
  has are sent again. Uploading after such a leave and a rejoin of the same site makes a new
  collection (the box asks with `fresh` on `vesper.codex.adopt`), so changes the box made
  meanwhile reach the Codex; the earlier upload stays as it is. A start right after the job
  ended (a client saw `failed`) starts a new job. While a long job (an import, an export) holds the Codex's writer, the
  upload stops (`failed`, "busy") before its documents go in; starting again resumes.
- `DELETE /api/v1/codex/dormant/upload` → `204` (admins): stops it; what was copied stays. A
  job saved as `copying` that no thread runs (a restart, the Codex out of reach) becomes
  `cancelled` and isn't resumed. Starts and cancels are in the site's audit log
  (`codex.uploadDormant`, `codex.cancelDormantUpload`).
- `codexChanged {status}` when it starts and when it ends; poll `GET` while `copying`.

The box sends the documents first (their IDs kept; logged on the Codex as added by the box: client `server machine`, `machine` the box),
then each media file its assets use, through the Codex's resumable uploads, relinking the asset
(bytes the Codex's media library has already aren't sent). A document of a newer schema than
the site's server reads stops it with the reason. The box's own Codex is never changed. Over
the link: `vesper.codex.adopt`, `adopt_docs`, `adopt_finish`, `upload_begin`, `upload_chunk`,
`upload_finish`.

## Updates

The site's server is the **update channel** of the site's Vesper Launchers (G10,
`docs/LAUNCHER-PLAN.md`; capability `updates`). An admin uploads signed product packages; each
machine's Launcher reports its products, versions and health every 30 s and gets back the
actions queued for it in Oriel. Nothing is pushed: a Launcher needs no inbound port.

**Packages are signed with Ed25519 in minisign's format** (prehashed, minisign's default). The
signature's *trusted comment* names the package, and is signed too:
`vesper-package product=vesper version=0.2.0 platform=linux-x86_64` (`platform` is
`<os>-<arch>` as Rust names them: `linux-x86_64`, `windows-x86_64`, `linux-aarch64`). The
server accepts a package only when the signature holds for one of its update keys (the
compiled-in release key and `--update-key <file.pub>`); every Launcher verifies it again with its
own keys before installing.

| Route | |
|---|---|
| `GET /api/v1/updates/packages` | `UpdatePackage[]` `{id, product, version, platform, fileName, size, sha256, keyId, trustedComment, uploadedAt}`, by product and platform, newest version first |
| `POST /api/v1/updates/packages?file=NAME` | the package as the body (`application/octet-stream`, up to 4 GiB), its `.minisig` file base64-encoded in `X-Vesper-Signature` (admins) → `201 UpdatePackage`; `400` missing or refused signature (bad, tampered, unknown key, no `vesper-package` comment), `409` the same product, version and platform is already there |
| `GET /api/v1/updates/packages/{id}/file` | the package (`Range`, `Content-Disposition`) |
| `GET /api/v1/updates/packages/{id}/signature` | its `.minisig` text |
| `DELETE /api/v1/updates/packages/{id}` | (admins) → `204` |
| `PUT /api/v1/updates/launchers/{machineId}` | a Launcher's `LauncherReport` `{machineId, machineName, launcherVersion, platform, roles, products: LauncherProduct[], health: MachineHealth}` → `LauncherActions {actions}`: what was queued for it, each given out once. Only with **that machine's own Launcher key** (below), from the server machine, or with a key that may administer the site (`403` otherwise: another machine's Launcher key, or any client's key that can't administer); `400` when the body's `machineId` isn't the path's or it lists more than 32 products, `413` over 64 KB, `409` a new machine when 256 are known |
| `GET /api/v1/updates/launchers` | `LauncherEntry[]` `{report, lastSeenSeconds, online (reported within 90 s), pending, updates: AvailableUpdate[]}` (newer versions in the channel for the machine's platform), by machine name |
| `POST /api/v1/updates/launchers/{machineId}/actions` | a `LauncherAction` (`{"type":"update","product","version"}` or `{"type":"rollback","product"}`; admins) → `202 LauncherEntry`; `404` unknown machine or product, `409` no such package for its platform, or nothing to roll back to |
| `POST /api/v1/updates/launchers/{machineId}/idn-join` | (that machine's own Launcher key only; body `{}`) → `LauncherJoinCode {code, expiresInSeconds}`: an IDN join code the site's server makes as the site's administrator (in the Site Manager's audit as `Vesper Launcher – <machine name>`), for the machine's IDN Node to join the site (`idn-node join CODE`) when the Launcher makes it a display box. Single use, 10 minutes; the machine's waiting code from before is taken back. `403` another machine's Launcher key or any other key; `503` the Site Manager isn't running |

A `LauncherProduct` is `{product, name, version?, previous?, state, restarts, lastUpdate?}`:
`state` `running` / `stopped` / `starting` / `updating` / `rollingBack` / `failed`; `previous` is
the kept version "Roll back" returns to; `lastUpdate` is an `UpdateOutcome {from?, to, result
(installed / rolledBack / failed), message, at}`. An update that doesn't come up healthy (its
`GET /api/v1/server` answering with the new version) within the Launcher's health timeout is
rolled back by the Launcher itself (`rolledBack`). `MachineHealth` is `{cpus, cpuPercent?,
memoryTotal?, memoryAvailable?, diskTotal?, diskFree?, gpus}`. Launchers are kept in memory
(they report again within 30 s of a restart); packages in `<data>/updates/packages/`.

**Launcher keys (the join round).** A Launcher joins the site by pairing with the **site's server**
(`POST /api/v1/pair` with `"launcher": "<machineId>"`, `clientName` its machine's name; the server's
code pairs at once, without one it waits for an admin's approval like any pairing, listed in
`GET /api/v1/site/pairings` with `launcher: <machineId>` and named `Vesper Launcher – <name>`; a
machine of the site that isn't the site's server answers `409`; a `machineId` that isn't 1 to 64
of `A-Z a-z 0-9 - _ .` is `400`). What it gets is its machine's **Launcher key**: a named site API
key for the audience `app:vesper-launcher` (not `app:vesper`), scope `control`, bound at the site's
server to that machine ID (`site.json`). With it a Launcher reaches only `GET
/api/v1/updates/packages`, the package files and signatures, `PUT
/api/v1/updates/launchers/{its own machineId}` and its `POST …/idn-join`; every other route answers `401`, and a report as
another machine `403`. A machine joining again gets a new key and its key from before is revoked.
Revoke one in Site ▸ API keys like any key; one still unused a day after it was made is revoked
as an Oriel pairing's is.

The Launcher's own local API (loopback only, `127.0.0.1:5556`; it refuses requests a browser makes
for a web page of another origin (an `Origin` or `Referer` other than its own
`http://127.0.0.1:5556`, `Sec-Fetch-Site` other than `none` or `same-origin`: `403`) and a
`POST`, `PUT` or `DELETE` without `Content-Type: application/json` or the package's
`X-Vesper-Signature` (`415`)) uses the same types:
`GET /launcher/v1/status` (`LauncherReport`), `POST /launcher/v1/actions` (`LauncherAction` →
`UpdateOutcome`), `POST /launcher/v1/packages?file=NAME` (a package with `X-Vesper-Signature`),
`GET /launcher/v1/products/{product}/logs?lines=200` (text). For the tray window (`GET /`, the
Launcher's own page, same-origin) and the first-run wizard: `GET /launcher/v1/overview`
(`{status: LauncherReport, available: {product: version}, channel: {server?, lastReport?,
problem?}, setup, light: ok|updating|attention}`), `GET /launcher/v1/setup` (`{firstRun, roles,
availableRoles: [{id, name, available}], join: {state: notJoined (problem?) | waiting (server,
serverName, expiresInSeconds) | joined (server, serverName, keyId, joinedAt)}, machineName,
machineId}`), `PUT /launcher/v1/setup` (`{roles?, done?}`), `POST /launcher/v1/setup/join`
(`{server, code?}`: joins the site as above), `DELETE /launcher/v1/setup/join` (takes a waiting
join back, or forgets the key), `POST /launcher/v1/products/{product}/start` and `…/stop` (body
`{}` → `204`).

## Mosaic

The site's server manages the site's **display boxes** (Vesper Mosaic, `docs/MOSAIC-PLAN.md`;
capability `mosaic`): machines of the site whose IDN Node runs IDN's display service. Vesper sets
each screen's feed and settings through that service with a token it issues for the box, and
keeps them in `<data>/machine/mosaic.json`; each box keeps its own as well (it keeps showing while
the site's server is down). A machine running a Vesper server is never a display box (its screens
are Vesper's own outputs, Configure Screens): it is listed only when its Node's display service is
on as well, with `vesper: true` and a `conflict` to resolve, and changes to it answer `409`. A
server that isn't the site's server answers `managed: false` and `409` for changes.

| Route | |
|---|---|
| `GET /api/v1/mosaic` | `MosaicState {managed, boxes: MosaicBox[], channels: MosaicChannel[], walls: MosaicWall[], routingProblem?, logo?: MosaicLogo {hash, width, height}, logoProblem?, machines?: MosaicMachine[] {machine, name, health, headlessOk}}` (`walls` absent from a server before walls; `machines`: the site's other machines with an IDN Node, neither boxes nor Vesper's) |
| `PUT /api/v1/mosaic/boxes/{machine}` | `PutMosaicBox {address?}` (admins): add a machine of the site as a box (its Node doesn't report the display service), or set where its service is reached (an address, `host:port` or an `https://` URL; plain `http://` only to this machine; absent or empty: the address the site sees its IDN Node at, else the one it asked to join from) → `MosaicState`; `404` not a machine of the site, `409` a Vesper machine or a removed one |
| `DELETE /api/v1/mosaic/boxes/{machine}` | (admins) forget its address and its screens' feeds and settings → `204` |
| `PUT /api/v1/mosaic/boxes/{machine}/screens/{n}/feed` | a `MosaicFeed` (operators): `{"kind":"none"}`, `{"kind":"source","source":"BOOTH (Program)"}` or `{"kind":"channel","channel":id}` → `MosaicState`; `404` no such box, screen (while the box answers) or channel; `409` the screen is a wall's tile (change the wall's feed); `503` kept, but the box didn't take it now (Vesper sets it again when it answers) |
| `PUT /api/v1/mosaic/boxes/{machine}/screens/{n}/settings` | `MosaicScreenSettings {fit (fit/fill/stretch), rotation (0/90/180/270), overscan (0–20 %), sound? {device, volumeDb (−60–12), mute}, fallback (logo/last/black), fallbackAfterSeconds (0–3600), tile?}` (admins; every field optional, defaults as listed first; `tile` is ignored: tiles come only from walls) → `MosaicState`; `400` out of range, `503` as above |
| `POST /api/v1/mosaic/boxes/{machine}/screens/{n}/identify` | `MosaicIdentify {seconds? (0.5–600, 5)}` (operators): the screen shows its number and name → `204` |
| `POST /api/v1/mosaic/boxes/{machine}/screens/{n}/test-pattern` | `MosaicTestPattern {on, seconds? (1–3600)}` (operators) → `204` |
| `GET /api/v1/mosaic/boxes/{machine}/screens/{n}/thumbnail?w=480` | a JPEG of what it shows; `404` it shows nothing, `503` the box doesn't answer |
| `POST /api/v1/mosaic/identify`, `/api/v1/mosaic/test-pattern` | the same for every screen of every box → `204`; `503` names the boxes that didn't take it |
| `GET /api/v1/mosaic/channels` | `MosaicChannel[] {id, name, source, screens, router?}` (`screens`: how many follow it; `router`: the IDN router carrying it on the site's server, `HOST (Channel Main)`) |
| `POST /api/v1/mosaic/channels` | `PutMosaicChannel {name, source?}` (admins) → `201 MosaicChannel`; `409` the name is taken (case-insensitive) |
| `PUT /api/v1/mosaic/channels/{id}` | `PutMosaicChannel {name?, source?}` (a new name: admins; only the source: operators): every screen following it shows the new source (on its router: one `routers.route` on the site server's Node) → `MosaicState`; `503` names the screens (or the router) that didn't take it now |
| `DELETE /api/v1/mosaic/channels/{id}` | (admins) → `204`; `409` while screens or a wall follow it |
| `POST /api/v1/mosaic/walls` | `PutMosaicWall {name (1–48 characters, unique; `wall:` and the name at most 64 bytes), rows, columns (1–16), bezelMm: [h, v] (0–10000), screenMm?: [w, h] (needed with a bezel), feed?, cells?: MosaicWallCell[] {row, column (from 0), machine, screen}}` (admins) → `201 MosaicWall`; `400` a rule broken (a cell outside the grid or holding two screens, a screen in two cells), `404` no such box, screen or channel (for a screen new in the wall: one in it already stays while its box is gone or doesn't list it now), `409` the name is taken, a screen is in another wall, a Vesper machine; `503` made, but not every screen took it now |
| `PUT /api/v1/mosaic/walls/{id}` | `PutMosaicWall`, the wall whole (admins): moved screens get their new tile, screens taken out lose theirs and keep showing the feed → `MosaicState`; errors as above |
| `PUT /api/v1/mosaic/walls/{id}/feed` | a `MosaicFeed` (operators): every tile → `MosaicState`; `503` names the screens that didn't take it now |
| `POST /api/v1/mosaic/walls/{id}/identify` | `MosaicIdentify` (operators): every tile's screen → `204` |
| `DELETE /api/v1/mosaic/walls/{id}` | (admins): its screens lose their tiles and keep showing the feed → `204` |
| `GET /api/v1/mosaic/logo` | the site's logo (PNG) for the boxes' fallback; `404` none |
| `PUT /api/v1/mosaic/logo` | a PNG or JPEG body (admins; ≤ 8 MB, ≤ 7680 × 4320; kept as PNG) → `MosaicState`; `400` not a picture. Sent to every box online in the site with the site server's IDN Node's `media_send replace="true"` (IDN Beta14: the box replaces the file once the new one is whole) under the one of `logo-a.png` / `logo-b.png` the box's screens don't show (a screen reads a logo again only when its path changes), followed to the transfer's `sent`; then each screen of the box is pointed at it as its fallback logo (`settings.fallback.logo`) and shows it at once. A box from before Beta14 gets it under a name of its own (`logo-<hash>.png`), trusted as there 15 s after its Node accepted it. `MosaicBox.logoState`: `sending`, `onBox` (the box has it and every screen has it), `failed` (refused, or the transfer failed: `MosaicState.logoProblem`) |
| `DELETE /api/v1/mosaic/logo` | (admins) no logo pushed any more (the boxes keep theirs) → `204` |
| `PUT /api/v1/mosaic/boxes/{machine}/service` | `PutMosaicService {mode}` (admins), "Use as a display box": `headless` (from boot, no desktop: Linux; `apps.display` and `apps.display_boot` both on), `desktop` (in the desktop session: `apps.display` on, `apps.display_boot` off) or `off` (both off), set on the machine's IDN Node → `MosaicState`; `400` headless on a machine whose system isn't Linux, `404` not a machine of the site, `409` a Vesper machine or a removed one, `503` its Node didn't take it |

A `MosaicBox` is `{machine (fingerprint), name, health (as in Site ▸ Devices), service (the
display service's state as its Node reports it; empty: added by an admin), address?,
serviceMode? (`off`/`desktop`/`headless`: as Vesper last switched its display service),
serviceModeReported? (the mode is what the box's Node reports: its display service's `runs` and
`on`, else its `apps.display` / `apps.display_boot`; not only Vesper's last), headlessOk?,
updateManagedBy? (IDN Beta14: `Vesper` when its IDN Node is Vesper's package, which the Site
Manager doesn't update; as `SiteDevice.updateManagedBy`),
logoState? (`sending`/`onBox`/`failed`), addressFrom? (`admin`: set with `PUT …/boxes/{machine}`, it overrides the others; `site`: where
the Site Manager sees the box's IDN Node; `join`: where it asked to join from), added, vesper,
conflict?, reachable, problem?, screens: MosaicScreen[]}`. A box's display service is reached over
HTTPS accepting only that machine's own site certificate (IDN's `lan_client_config`), at any
address. A machine is `vesper` when it runs a Vesper server: a Vesper box of the site, or the IDN
Node a Vesper server reports as its own machine's (each server reads its Node's fingerprint and
sends it over the Codex link's presence report, `node`). A `MosaicScreen` is `{number
(from 1), name (the desktop's output name), width?, height?, feed (what Vesper has it show;
`none` also while Vesper never set it: the box keeps its own feed, and changing only the screen's
settings doesn't touch it), showing (the feed the box has now), testPattern?, settings (Vesper's,
or the box's own as it last reported them while Vesper never set them, so that a change starts
from them), status, problem?}` with `status`
`showing`, `connecting`, `lost` (the feed went; the last picture holds), `fallback`, `idle` (no
feed), `offline` (the box doesn't answer or is offline in the site) or `error` (`problem`). A box
whose display service answers with an error of its own (no desktop session: "no desktop on this
computer") is `reachable` with that `problem`, and its screens keep what Vesper set for them. A
`MosaicTile {rows, columns, row, column, bezelMm: [h, v], screenMm?: [w, h], group?}` places a
screen in a video wall; a screen of a wall also has `wall` (the wall's id) and, while it shows a
picture, `sync` (`MosaicSync {group, mode, delayMs, late, lateRecently}`: IDN's frame sync, late
pictures counted by the box and since the read before).

**Channels are IDN routers** on the site's server's own IDN Node (`routers.list`, `Channel
<name>=SOURCE; …`, applied through the Site Manager it hosts): screens and walls following a
channel are given the router's source (`MosaicChannel.router`), and a channel's new source is one
`routers.route` there (the screens hold their picture and cut to the new one; the Node keeps it
in its saved list). Sending `routers.list` puts every router back to the sources in it, so the
list always carries each channel's current source; at its start the server doesn't send it when
the Node's report already shows the same list. Without an IDN Node
on the site's server, or while its Node refuses the list (`MosaicState.routingProblem`), each
screen is given the channel's source itself, as before; existing channels move onto routers by
themselves once the list is taken.

A **video wall** (`MosaicWall {id, name, rows, columns, bezelMm, screenMm?, feed, cells, status,
synced, problem?, delayMs?, lateRecently}`) shows one picture across its screens, of one box or
several: every tile gets the same geometry with its own row and column, the sync group
`wall:<name>` and the wall's feed (settings before the feed). `status` is its tiles' worst;
`synced` is true while every tile showing a picture is in the wall's group and none dropped late
pictures lately; `problem` says what is wrong, for people. A wall of this server's own displays is
Configure Screens' Grouped / Edge Blend, not Mosaic. The state is read from every box every 5 s
(feeds, and settings the box lost, are set again; settings a box didn't take when they changed,
a tile leaving included, are sent until it does); a change is announced with `siteChanged`
(`mosaic`).

## Signage

The site's server renders the site's **signage** (`docs/SIGNAGE-PLAN.md`; capability `signage`).
Each **channel** is a screen of that server (kind `{"type":"signage","channel":id}`, named
`Signage: <name>`; not an operator screen) with an IDN output whose source name is the channel's
name, so receivers list it as `HOST (Lobby)` (`source`) and Mosaic screens, walls and channels
follow it like any other source. A channel's **schedule** (`SignageSlot {id, playlist, days (0 =
Monday … 6 = Sunday; empty: every day), start, end (minutes; an end at or before the start runs
past midnight), from?, until? (YYYY-MM-DD, inclusive), priority}`) picks its playlist: the highest
priority of the slots playing now, a tie going to the one listed first, else the channel's
`defaultPlaylist`. A **playlist**'s items (`SignageItem {id, content, duration (1–3600 s),
transition?}`, a 1 s dissolve by default) play in turn in the layout's main zone; `content` is
`{"kind":"slide","presentation":id,"slide":n}` (from 0), `{"kind":"image","asset":id}`,
`{"kind":"video","asset":id}` (looping, muted), `{"kind":"text","title","body"}` or
`{"kind":"feed","feed":id,"rows":1–8}`. An item that can't be shown (a missing presentation,
slide or media file, a feed with no rows) is skipped and named in the channel's `now.problem`.
**Layouts:** `full`, `mainTicker`, `mainSidebar`, `mainSidebarTicker`; the sidebar shows the
clock, the countdown to the next of the settings' `services` and the weather (with its credit),
the ticker scrolls `ticker` then the titles of `tickerFeed`'s rows. **Feeds** are read with IDN's
`idn-feeds` every `refresh` seconds (60–86400): `url` is an `http(s)://` address (a Google Sheet's
link is read as its CSV export) or a file on the server, `format` `auto`/`csv`/`tsv`/`json`/`xml`
(RSS too); a failed read keeps the last good rows (`problem` says why). The **weather** is
Open-Meteo's for `settings.place`, read every 30 minutes and left out once older than 6 hours.
Kept in `<data>/machine/signage.json` and `signage-cache.json`; a server that isn't the site's
server answers `managed: false` and `409` for changes. Changes are announced with `siteChanged`
(`signage`) and audited.

| Route | |
|---|---|
| `GET /api/v1/signage` | `SignageState {managed, channels: SignageChannel[], playlists, feeds, settings, weather?, override?}`; each channel has `now: SignageNow {playlist?, slot?, item?, next?, health (ok/idle/problem), problem?, overridden}` |
| `POST /api/v1/signage/channels` | `PutSignageChannel {name, layout?, defaultPlaylist?, slots?, ticker?, tickerFeed?}` (admins) → `201 SignageChannel` (its screen and output made); `409` the name is taken (case-insensitive) |
| `PUT /api/v1/signage/channels/{id}` | `PutSignageChannel` (admins; only fields sent change, `""` clears `defaultPlaylist`/`tickerFeed`, `slots` whole) → `SignageChannel`; a new name renames its screen and output, and Mosaic's channels, walls and screens showing the old source name show the new one |
| `DELETE /api/v1/signage/channels/{id}` | (admins) → `204`; its screen and output go; `409` while a Mosaic channel, wall or screen shows its source |
| `POST /api/v1/signage/playlists`, `PUT …/{id}` | `SignagePlaylist {name, items}` whole (admins) → `201`/`200 SignagePlaylist` |
| `DELETE /api/v1/signage/playlists/{id}` | (admins) → `204`; `409` while a channel plays it |
| `POST /api/v1/signage/feeds`, `PUT …/{id}` | `SignageFeed {name, url, format?, refresh?, mapping? {title, body, when}}` (admins) → `201`/`200 SignageFeed`; read at the next tick. `url` is an `http(s)://` address (not this machine's loopback or link-local addresses) or a file in the server's `<data>/signage-feeds/` folder (its name there, e.g. `events.csv`); anything else is `400` |
| `DELETE /api/v1/signage/feeds/{id}` | (admins) → `204`; `409` while an item or a ticker uses it |
| `POST /api/v1/signage/feeds/preview` | a `SignageFeed` (admins): read once → `SignageFeedPreview {columns, rows}` (20 rows); `400` it can't be read |
| `PUT /api/v1/signage/settings` | `SignageSettings {place? {name, latitude, longitude}, fahrenheit, services: [{day, minute, name}]}` (admins) → `SignageSettings` |
| `GET /api/v1/signage/places?q=Springfield` | Open-Meteo's geocoding (admins) → `SignagePlaces {places}`; `503` it can't be reached |
| `POST /api/v1/signage/override` | `SignageOverride {text (1–500), style (urgent/info), channels (empty: all), seconds?}` (operators): over everything on its channels at once → `SignageOverride` with `since`, `until?` |
| `DELETE /api/v1/signage/override` | (operators) → `204` |

## Scripture

Bible translations stored on the server (`*.vbible` files: the bundled folder, `--scripture-dir`, and the data folder's `scripture/`), so the Bible works offline. Capability `scripture`. A translation is named by its `id` (`KJV`, case-insensitive). References are typed the way people write them: `Jn 3:16-18`, `1 Cor 13`, `Psalm 23:1–4, 6`, `John 3:35–36, 4:1–2` (after a comma a bare number is a verse of the chapter before it, `C:V` starts another chapter, `;` always starts a chapter context), `Gen 1:31-2:3`, `Jude 3`; English book names and the usual abbreviations, and (batch 9) the book names of the translation's `language` (tables for `es`, `pt`, `fr`, `de`; accents optional: "Genesis" finds "Génesis"). A translation whose file names its books in English (or not at all) shows them in its language where there is a table (`books`, `bookName`, labels: "Juan 3:16", "Salmo 23"); names the file gives in its language are kept. Labels come back normalised with en dashes ("Psalm 23:1–4, 6", "John 3:35–36; 4:1–2", "1 Corinthians 13").

| Route | |
|---|---|
| `GET /scripture/translations` | `ScriptureTranslation[]`: id, abbreviation, name, language, licence, `attribution` (a line generated slides must carry, when the translation has one), `origin` (`bundled` / `local`), number of books |
| `GET /scripture/translations/{id}/books` | `ScriptureBook[]`: USFM `key`, `name`, `testament` (`old` / `new` / `other`), `chapters` (verse count of each) |
| `GET /scripture/parse?translation=&q=` | `ScriptureParse`: `ok`, `book` / `bookName` once a book is recognised, `ranges` (`VerseRange`: chapter, verse, endChapter, endVerse), `label`, or `error` ("John 3 has 36 verses"): live feedback for a reference box |
| `GET /scripture/passage?translation=&ref=` | `ScripturePassage`: `label`, `ranges`, `attribution`, `verses` (`ScriptureVerse`: book, chapter, verse, text; verses the translation lacks are left out). `400` with the reason for a bad reference, `404` for an unknown translation |
| `GET /scripture/search?translation=&q=&limit=` | `ScriptureSearch`: verses holding every word of `q` (case-insensitive), canonical order, at most `limit` (default 50, at most 500), and `total` |
| `POST /scripture/slides` | `ScriptureSlideRequest` → `ScriptureSlides`: the presentation's `name` and each slide's `text`, `reference` and `attribution`; nothing is written |
| `GET /scripture/preview?translation=&reference=&theme=&template=&versesPerSlide=&maxChars=&verseNumbers=&referenceOn=&secondTranslation=&secondLayout=&slide=&w=` | PNG (`w` default 320) of generated slide `slide` (0-based) with the theme, as it would be stored (a parallel passage as its layout shows both translations) |
| `POST /scripture/presentations` | `ScriptureCreate` → `201 ScriptureCreated` (`presentation`, `name`, `slideCount`, `playlistItem`) |
| `POST /scripture/presentations/{id}/resplit` | `ScriptureResplit {force?}` → `ScriptureResplitResult {presentation, slideCount, changed}`: the passage split again with its theme as it is now; `409` when the operator edited it and `force` isn't set (or it changed while it was being split), `423 locked` when another client holds its edit lock (the writer is `X-Client-Id`), `400` for a presentation not made on the Bible page |

**Slides** (`ScriptureSlideRequest`): `translation`, `reference`; `theme` (+ `template`; default the theme's template slide named like "Scripture", else its first) styles every slide by applying the template (copy semantics, the theme isn't changed), else a plain layout (white on black); slides are split **by measuring**: each candidate slide is built as it will be stored (the theme applied, its reference and attribution lines) and its text laid out with the renderer's text layout in its box at the authored size (font, size, box, margins, line spacing; a shrink-to-fit box is measured unshrunk), and a slide takes as many whole verses as fit, at most `versesPerSlide` when given, at most `maxChars` characters when given (60–2000); a verse that doesn't fit a slide on its own breaks between words (the longest start that fits) and continues on the next slide, where following verses may join it: the broken part ends with ` …`, the continuation starts with `… `, the verse number is on the first part only, and both slides' references name the verse (a box too small for even one word, e.g. a one-line box that must also hold the reference, gets one word a slide and a warning in the log); `verseNumbers` (superscript); `referenceOn` `each` (default) / `last` / `none`: the reference line names the verses on that slide and the translation ("John 3:16–17 KJV"); the translation's attribution line, if any, is on the last slide. Each slide has a text element (placeholder slot `text1`) and a reference element (slot `reference`); a theme without a `reference` slot gets the reference appended to its last text slot.

**Parallel Bible** (batch 16): `secondTranslation` (a translation ID; `404` when unknown, `400` when it lacks the book) adds the same verses in that translation to every slide as its second language (`Slide.translation` on the `text1` element, see [Dual-language slides](#dual-language)), verse-aligned by chapter and verse (verses only the second has are left out; a verse the primary breaks across slides is broken in the second in proportion of words), with `secondLayout` as the slide's preferred layout (`sideBySide` by default; `stacked`, `second`, `primary`). The primary decides the split; the fits check lays each slide out as that layout draws it (in the theme's `secondLanguage` style), so both texts fit. The reference line names both translations ("John 3:16–17 KJV · RVR"), the attribution line both notices, the default name "John 3:16–17 (KJV · RVR)"; `ScriptureSlides` gives each slide's `second` text; `scripture.generated` records `secondTranslation` and `secondLayout`, so splitting again keeps them. The preview route takes `secondTranslation=&secondLayout=` too.

**Presentations** (`ScriptureCreate` = the slide request + `then`, `playlist`, `name`): a presentation with `scripture` set (so its summary's `kind` is `scripture`), category `Scripture`, named by the label and abbreviation ("John 3:16–18 (KJV)") unless `name` is given, in the active collection's library named **Scripture** (created when missing), with `templateOrigin` naming the theme. `then`: `none`, `playlist` (appended to `playlist`; `400` without one, `404` for an unknown one) or `show` (its first slide goes live at once, no playlist). Nothing is written when the request is refused. On a show box the writes go to the site's Codex like other content writes, as the client that asked.

**Split again when the theme changes** (batch 9): a created passage records its request in `scripture.generated` (`ScriptureGenerated`: `translation`, `reference`, `theme`, `template`, `versesPerSlide`, `maxChars`, `verseNumbers`, `referenceOn`, `secondTranslation`, `secondLayout`, `slides`: a digest of the sections as generated, `stale`). After an edit of a theme, the server (the Codex box) splits again, in the background and a moment later, every passage made with that theme whose sections still match the digest, in the collection the theme edit changed (a show box's edit: the collection it shows): one presentation edit replacing its slides (so undo brings the old split back; a slide that still starts at the same verse keeps its ID, so a live passage keeps its place), applied only to the revision it was split from, `documentChanged` as for any edit. A passage the operator edited, one someone holds the edit lock on, or one that changed while it was being split keeps its slides and gets `stale: true`; `resplit` with `force` splits it again (the edits are replaced). Passages made before batch 9 have no `generated` and stay as they are.

## Recording and streaming

Capability `streaming` (`docs/RECORD-STREAM-PLAN.md`). A screen's picture and the program audio
are recorded to a file on the server and streamed to RTMP(S) and SRT targets through IDN's
gateway (one hardware encode per job; the CPU's H.264 where the box has no GPU encoder).

| Route | |
|---|---|
| `GET /api/v1/streaming` | `StreamingSettings`: `recording` (`RecordingSettings`), `targets` (`StreamTarget[]`, keys never included: `hasKey`), `recordingsFolder` (where files go now), `encoders` (`{codec, label}[]`: what this box encodes; empty = it can't record or stream) |
| `PUT /api/v1/streaming/recording` | `RecordingSettings` → `StreamingSettings`: `screen` (`null` = the first enabled screen), `container` `mp4` (fragmented, default) / `mkv`, `codec` `h264` / `hevc` / `av1`, `videoMbps` 0.5–100 (12), `audioKbps` 64–512 (192), `splitMinutes` 0–1440 (0 = one file), `folder` (absolute, `null` = the data folder's `recordings`), `minFreeGb` (2); `400` for values outside these |
| `POST /api/v1/streaming/targets` | `StreamTarget` → the target with its `id`: `name` 1–64 characters, `kind` `rtmp` (`url` `rtmp://` or `rtmps://` without the key, `key` the stream key) or `srt` (`url` `srt://host:port?…`, passphrase and stream ID in the query; a `key` sent with an SRT target is not kept), `screen`, `codec`, `videoMbps` 0.5–50 (6), `audioKbps` 64–320 (160); at most 16 targets |
| `PUT /api/v1/streaming/targets/{id}` | the whole target; `key` absent keeps the saved key while the server (scheme, host and port) stays the same (and the kind: RTMP and SRT are different servers); for another server an RTMP target with a saved key needs `key` (`400` without it: enter the key again, or `""` to save the target without one), `""` clears it; an SRT URL is shown with its query values as `***`, and values sent back as `***` keep the saved ones on the same server (`400` for another server: enter them again). A running stream keeps its old settings until restarted |
| `DELETE /api/v1/streaming/targets/{id}` | `204` (a running stream is stopped first) |
| `POST /api/v1/streaming/targets/{id}/start`, `…/stop` | optional `{screen}` → `StreamingStatus`; `409` when already running / not running or no encoder takes the codec, `400` for an RTMP target without a key |
| `POST /api/v1/recording/start` | optional `{screen}` → `StreamingStatus`; `409` when already recording, `507 insufficientStorage` under `minFreeGb` free in the folder |
| `POST /api/v1/recording/stop` | the file is finished (trailer written) before the answer; `409` when not recording |
| `POST /api/v1/recording/split` | closes the file and goes on in a new one (`part` + 1) |
| `GET /api/v1/streaming/status` | `StreamingStatus`: `recording` (`screen`, `screenName`, `file`, `state`, `detail`, `startedAt` ms, `seconds`, `bytes`, `part`, `videoKbps`, `skippedPictures`, `dropped`, `freeBytes`) or `null`, and `streams` (per running target: `state`, `detail`, `seconds`, `bytes`, `sentKbps`, `videoKbps`, `reconnects`, `dropped`, `skippedPictures`, `lagMs`) |

`state` is `connecting`, `live`, `retrying` (a stream that can't connect keeps trying; `detail`
says why) or `failed` (a recording that couldn't be written). Files are named after the screen and
the start time (`Audience 2026-11-25 14-03-07.mp4`); `splitMinutes` parts get `-2`, `-3`, ….
A running recording stops itself, with a warning notice, when the folder's disk falls under half of
`minFreeGb`. There is no WebSocket message yet: clients poll the status (Oriel every 2 s). Stream
keys and SRT URLs stay in the box's `machine/streaming.json` (owner-only on Unix), are never
logged, never sent back and never part of settings backups.

## Media preparation

On the Codex box (`docs/MEDIA-PREP-PLAN.md`). After an upload is probed, a background job
analyses a video or sound file (EBU R128 loudness, the gain to the house target, a poster frame
that isn't black) and converts a video that is hard to play live to the house format (H.264 or
HEVC 8-bit 4:2:0, constant frame rate, AAC 48 kHz stereo, MP4); the asset then plays the copy
and keeps its original. What it found is the asset's `prep` (`MediaPrep`): `trimIn`/`trimOut`
(the operator's, non-destructive: a use that sets its own in or out point keeps its whole range), `loudness` (LUFS), `peak`
(dBFS), `gainDb` (added to a use's level at playback, never baked in), `posterAt`, `original`
(`PreparedOriginal {location, probe, reasons, format}`), `analysed` (the SHA-256 of the file
prepared: each file once), `notes`, `error`, `at`. A relink forgets all but the trim.

| Route | |
|---|---|
| `GET /api/v1/codex/media-prep` | `MediaPrepStatus`: `settings`, `jobs` (`MediaPrepJob {asset, collection, name, step, progress, error?}`; `step` `queued`, `analysing`, `converting`, `waitingForLive`, `done` (listed a minute), `failed`), `converter` (IDN's converter and the FFmpeg it runs on, `IDN media conversion, FFmpeg 8.0.1 (…)`; absent: videos are analysed, not converted), `encoder` (the encoder a conversion would use now with the settings, for people: `VA-API`, `NVENC`, `OpenH264 (CPU)`; absent: none, media aren't converted), `gpuEncoder` / `cpuEncoder` (what this box has for the house codec; absent: none), `codecEncoders` (`{h264, hevc}`, each `{gpu?, cpu?}`: what this box has for either codec, for a codec chosen but not saved yet; absent from older servers), `live` |
| `PUT /api/v1/codex/media-prep` | `MediaPrepSettings` (admins) → the saved settings: `enabled` (new uploads, default on), `transcode` (on), `video` `h264` / `hevc`, `loudnessTarget` −30…−5 (−16), `maxJobs` 1–4 (1), `whileLive` `lowPriority` (default: conversions go on at the lowest priority while something is live on the box) / `wait` (they stop and start again once nothing is live), `priority` `low` (default) / `normal` (while nothing is live), `quality` `standard` (default; IDN's bit rate by size and rate × 1.6: 12 Mb/s at 1080p30 H.264) / `high` (× 2.67) / `maximum` (× 4), `videoKbps` (optional: an explicit bit rate, 1 000–60 000), `maxSize` `keep` (default) / `1080p` / `2160p`, `encoder` `auto` (default: the GPU's where there is one, the CPU's while live) / `gpu` / `cpu`. Fields left out take their defaults (settings saved before load so) |
| `POST /api/v1/codex/collections/{id}/media/{asset}/prepare` | optional `PrepareMedia {again}` → `202`; `400` for a picture |
| `PUT /api/v1/codex/collections/{id}/media/{asset}/trim` | `SetMediaTrim {trimIn?, trimOut?}` (seconds; absent = none) → `RelinkedAsset`; `400` for a negative time, one past the end, or an out point not after the in point |
| `POST /api/v1/codex/collections/{id}/media/{asset}/original` | the asset plays its original again (not converted again by itself) → `RelinkedAsset`; `409` when it plays its original already |

On a show box, `…/media-prep` answers `409 codexElsewhere`; `prepare`, `trim` and `original` are
forwarded to the Codex box; a show box plays a prepared asset's original while its copy is still
copying to it. Jobs not finished survive a restart. There is no WebSocket message for jobs: clients poll the status
(Oriel every 2 s while jobs are open); asset changes arrive as `documentChanged` as usual.

## Reports

`GET /api/v1/reports/ccli?from=YYYY-MM-DD&to=YYYY-MM-DD` → `CcliReport {from, to, rows, uses}`
(both days included; default the last 6 months). Each time a song (a presentation with song
metadata: a CCLI number, title or authors) goes live, the box logs one `CcliUse` per service
(the playlist it was shown from, else the day): `date`, `at`, `service`, `presentation`, `title`,
`ccliNumber`, `authors`, `projected` (an audience screen was on), `streamed`, `recorded` (at the
time it went live, or later in the same use). `rows` (`CcliReportRow`) count the uses per song,
most used first. `GET /api/v1/reports/ccli.csv?from=&to=` gives the rows as CSV with the columns
CCLI's reporting asks for (`Song Title`, `CCLI Song #`, `Authors`, `Times Used`, `Projected`,
`Streamed`, `Recorded`, `From`, `To`). Reporting itself is done on CCLI's site. The log is per box
(`reports/ccli-usage.jsonl` in its data folder).

## Machine translation

<a id="machine-translation"></a>Offline machine translation of a presentation's slides into a second language (capability `translate`; `docs/DUAL-LANGUAGE-PLAN.md` §5). One model per language pair (OPUS-MT on candle, CPU), kept in the box's own model store (`<data>/cache/translate/<pair>/`), fetched from its publisher at a pinned revision and checked against a pinned SHA-256; never part of a package. Every proposal carries the notice **"Machine translation: check it, and check your licence allows a translation of this song"** and nothing is saved until the client keeps it.

| Route | |
|---|---|
| `GET /translate/models` | `TranslationModels {engine, models, notice}`: each `TranslationModel {id ("en-es"), from, to[], model, repo, revision, card, licence (SPDX), attribution, size (bytes), status, downloaded?, detail?}`; `status` is `notDownloaded`, `downloading` (`downloaded` bytes so far), `ready` or `failed` (`detail`, kept over a restart). Any client. |
| `POST /translate/models/{id}/download` | Starts fetching the pair (site admin; audited `translate.models.download`). `200` with the model; `404` for an unknown pair. Each file is hashed as it arrives; a size or SHA-256 mismatch fails the download and removes what was fetched. |
| `POST /translate/models/{id}/stop` | Stops a download (admin); the pair is `notDownloaded` again. |
| `DELETE /translate/models/{id}` | Removes the pair's files (admin; unloads it first). |
| `POST /translate/preview` | `TranslatePreview {presentation, slides?, to, from? ("en")}` → `202` with a `TranslateJob`. Any client: a preview only reads, and fetches the pair's model first when it is not on the box (models come on first use). `400` when no pair translates `from` → `to` (BCP 47; `es-MX` uses the `es` model) or for a bad code, `404` for an unknown presentation or slide, `409` with 32 previews kept. At most 500 slides. |
| `GET /translate/jobs/{id}` | The `TranslateJob {id, presentation, from, to, model, status, done, total, downloaded?, downloadSize?, detail?, slides, seconds?, notice}`; `status` is `queued` (one preview translates at a time), `downloading`, `translating` (`done` of `total` slides with text), `done`, `failed` (`detail`) or `cancelled`. `slides` (once `done`) holds `TranslatedSlide {slide, translation}` per slide with text, in order: the proposed `SlideTranslation` (each text element's lines translated one to one, blank lines kept, the slide's `layout` kept, `machine` set). Kept 10 minutes after it ends. |
| `DELETE /translate/jobs/{id}` | Cancels a preview (`204`; a download it started is stopped). |

**Keep** = one `setSlideTranslation {slide, translation}` per proposed slide (the layer as proposed, `machine` included); **Discard** = nothing (nothing was written).

## TypeScript bindings

`bindings/` holds one `.ts` file per type and an `index.ts` that re-exports all of them, along with `PROTOCOL_VERSION` and `PREVIEW_HEADER_LEN`. The files cover every protocol type and every model type the UI receives. In the bindings:

- ID newtypes are `string` aliases.
- `Link<T>` stays generic.
- Fields that serde omits are optional (`?`).
- `serde_json::Value` becomes `unknown`.
- 64-bit integers are `number`.

```ts
import type { ServerMessage, Command, ShowState, Presentation } from "./bindings";
```

The `bindings` test fails when the committed files are stale. After changing a wire type, regenerate them:

```sh
UPDATE_BINDINGS=1 cargo test -p protocol --test bindings
```

The `wire` test serializes sample values with serde and checks that every field name and tag exists in the TypeScript, and that every required TypeScript field is actually sent. On the model side, a field that serde may omit needs a `#[cfg_attr(feature = "ts", ts(optional))]` annotation (or `ts(as = "Option<_>", optional)` when it is not an `Option` and only the struct-level `#[serde(default)]` applies). The test catches a missing one.

## Trying it

```sh
vesper --data <dir>                                              # 0.0.0.0:5555, mDNS on
cargo run -p vesper-server --example ws_probe -- --addr 127.0.0.1:5555  # hello, state, triggers, preview, thumbnails
cargo run -p vesper-server --example ws_probe -- --discover             # mDNS browse
cargo run -p vesper-server --example ws_probe -- --codex                # Codex status, collections, then codexChanged / documentChanged as they come
cargo run -p vesper-server --example ws_probe -- --codex-search grace --codex-collection all --watch-seconds 0
```

Environment: `VESPER_VIDEO_DECODE=software` (same as `--software-decode`) and `VESPER_VAAPI_DEVICE=<render node>` (default `/dev/dri/renderD128`). The pre-rename names `STAGE_VIDEO_DECODE` and `STAGE_VAAPI_DEVICE` still work for one release, with a deprecation warning in the log.
