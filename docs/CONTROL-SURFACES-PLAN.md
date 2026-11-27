# Control surfaces: a Companion module and Remote API compatibility

Batch 13, lane S. Stream Deck panels (through Bitfocus Companion) and the button profiles churches
already have should drive Vesper. Two pieces:

1. **A Bitfocus Companion module** for Vesper (`integrations/companion/`), speaking Vesper's own
   API: REST commands and the WebSocket's show state.
2. **Remote API compatibility** on the server: the commonly used endpoints of the presentation
   remote REST API that existing Stream Deck / Companion setups call, on a port of its own, off
   by default, mapped onto Vesper's commands.

Both use the API's one auth model: the server machine is trusted, every other client sends a
paired site key (see `crates/protocol/README.md`, "Authentication and pairing").

## 1. Model and API (server)

- **Macros** become triggerable: `triggerMacro {macro}` runs a macro's enabled actions in order
  (the same runner as a slide's macro action: media actions go to their layer, nested macros at
  most 4 deep, missing targets skipped and counted); `notFound` for an unknown macro.
  `GET /api/v1/macros` lists the active collection's macros (`Macro[]`, stored form, in their
  list order). Capability `macros`.
- **Remote API compatibility settings** (machine-local, `machine/remote-compat.json`):
  `RemoteCompatSettings {enabled, port}`, default off, port **50001**.
  `GET /api/v1/remote-compat` → `RemoteCompatStatus {enabled, port, listening, problem?}`;
  `PUT /api/v1/remote-compat` `{enabled?, port?}` saves and applies at once (the listener
  starts, moves or stops; a port in use answers with `listening: false` and the `problem`,
  and the setting is kept, so a restart retries). Any client allowed to run the show may change
  it: the port answers only with the same auth. Port `0` takes a free port (tests).
  Capability `remoteCompat`.

## 2. The compatibility layer (server)

A second listener on the configured port (same interface as the main API) with its own small
router (`crates/vesper-server/src/remote_compat.rs`), behind the same authentication as
`/api/v1`: requests from the server machine are trusted (same `Host` rule), others need a site
key as `Authorization: Bearer <key>` or `?token=<key>` (otherwise `401`). Its triggers are plain
`GET`s that any web page could send (`<img src=…>`), so a request a browser makes for a page
(`Sec-Fetch-Site` other than `none`, or any `Origin` or `Referer`) is never the server machine
here and needs a key too (bug run 4). Commands go
through the same dispatcher as `POST /api/v1/commands`, so they behave exactly like Oriel's.

`{id}` is a UUID, a name (case-insensitive) or a 0-based index in the list's order. Triggers
answer `204`; errors answer the API's JSON error with its status (`404` unknown id, `409` nothing
live, …). List answers use the API's `{"id": {"uuid", "name", "index"}, …}` shape.

| Endpoint | Vesper |
|---|---|
| `GET /version` | server name, platform, `host_description: "Vesper <version>"`, `api_version: "v1"` |
| `GET /v1/trigger/next`, `/v1/trigger/previous` | `next`, `prev` |
| `GET /v1/presentation/{active\|focused}/next/trigger`, `…/previous/trigger` | `next`, `prev` |
| `GET /v1/presentation/{active\|focused}/trigger` | the live presentation's first slide |
| `GET /v1/presentation/{active\|focused}/{index}/trigger` | `triggerSlide {index}` in the live presentation (keeps its playlist item) |
| `GET /v1/presentation/{uuid}/{index}/trigger` | `triggerSlide {presentation, index}` |
| `GET /v1/presentation/active`, `/v1/presentation/slide_index` | the live presentation's id; its slide index |
| `GET /v1/status/slide` | current / next slide `{uuid, text, notes}` (text: the slide's text elements) |
| `GET /v1/playlists` | playlists (flat, folders left out) |
| `GET /v1/playlist/{id}/trigger`, `/v1/playlist/{id}/{index}/trigger` | `triggerPlaylistItem` (first / `index`-th item) |
| `GET /v1/clear/layer/{layer}` | `clearLayer` (`audio`, `props`, `messages`, `announcements`, `slide`, `media`, `video_input`) |
| `GET /v1/clear/groups`, `/v1/clear/group/{id}/trigger` | clear groups; `clearGroup` |
| `GET /v1/timers`, `/v1/timers/current` | timers; their values (`time`, `state`) |
| `GET /v1/timers/{start\|stop\|reset}` | every timer |
| `GET /v1/timer/{id}/{start\|stop\|reset}` | `timerStart` / `timerStop` / `timerReset` |
| `PUT /v1/timer/{id}/increment` (body: seconds) | `timerAdjust` |
| `GET /v1/timer/system_time` | the server's Unix time |
| `GET /v1/messages`; `GET` or `POST /v1/message/{id}/trigger` (body: `[{name, text: {text}}]`); `GET /v1/message/{id}/clear` | messages; `messageShow` with token values; `messageHide` |
| `GET /v1/looks`, `/v1/look/current`, `/v1/look/{id}/trigger` | looks; the live look; `setLook` |
| `GET /v1/props`, `/v1/prop/{id}/trigger`, `/v1/prop/{id}/clear` | props (`is_active`); `propShow`, `propHide` |
| `GET /v1/macros`, `/v1/macro/{id}/trigger` | macros; `triggerMacro` |
| `GET`, `PUT`, `DELETE /v1/stage/message` (PUT body: a JSON string) | the stage message; `stageMessage`; `stageMessageClear` |

Not covered (answered `404`): media and audio playlists' own routes, transport, library and
presentation documents, thumbnails, the capture/record routes, the status stream
(`/v1/status/updates`), announcements' own triggers, video inputs, themes, groups.

## 3. Companion module (`integrations/companion/`)

`@companion-module/base` **1.13.6** (the oldest 1.x with a secrets store: Companion 4.1 and
later load it; 1.14+ and 2.x need newer Companions) and `ws` 8.22.1, exact versions (a
transitive `colord` held at 2.10.0, so nothing is younger than two weeks); Node 22 runtime.
Config: host, port (5555), pairing code (used once: the module pairs, keeps the key in
Companion's secrets store and clears the code), name, key. Over one WebSocket it follows the show state (and `timerTick`s), refetches lists on
`documentChanged`, and sends commands with `POST /api/v1/commands`.

- **Actions:** next / previous slide; trigger slide by index (live or chosen presentation);
  trigger playlist item (playlist + item index); clear layer, clear all, clear group,
  restore layer; set look; timer start / stop / reset / adjust; show / hide message (with token
  values); hide all messages; prop show / hide / toggle; trigger macro; media play / pause /
  toggle / restart; audio next / previous; master mute / volume; stage message / clear.
- **Feedbacks** (boolean): slide live (index, optionally of a presentation), layer has content,
  look active, timer running / overrun, message shown, prop shown, master muted, connected.
- **Variables:** server name, live presentation / slide number / section, next slide, live
  look, media / audio name, transport position, stage message, each timer's value
  (`timer_<name>`) and state, connection state.
- **Presets:** next / previous, clear all / slide / media, a button per look, timer and macro.
- Tests: `node --test` against a mocked Vesper (HTTP + WebSocket): pairing, commands, state →
  variables and feedbacks, choice refresh. The Companion runtime itself is not started.

## 4. Oriel

Settings ▸ Network & IDN gets a **Control surfaces** card: "Remote API compatibility" on/off,
the port, the status (listening / the problem), and a line on keys ("Stream Deck setups on
other computers need this site's key; pair them like a tablet"). A stub route serves it for the
screenshots.

## 5. Status (batch 13)

Built as planned: the server's layer and setting (`crates/vesper-server/src/remote_compat.rs`,
tests in `tests/server/remote_compat.rs`), `triggerMacro` and `GET /macros`, the Companion
module with its tests (`npm test` in `integrations/companion/`), Oriel's card with two shots
(`settings-control-surfaces*`). Pairing in the module also works by an admin's approval (Oriel ▸
Site ▸ Join requests), not only by code. Not verified here: the module inside a running
Companion (its tests stand in for `@companion-module/base`), and real Stream Deck profiles.

## 6. Deferred

- Unauthenticated LAN access for old button profiles (see question 1).
- The compatibility layer's status stream, transport and media/audio playlist routes.
- Companion presets per message; image (thumbnail) feedbacks; finding Vesper by mDNS
  (`bonjour-device`, `_vesper._tcp`) in the module's settings.
- MIDI control surfaces (Cantor / Transept).

## Owner questions (answered with my recommendation)

1. *Should the compatibility port accept requests without a key from the LAN, as the original
   API does?* **No.** Church LANs aren't trusted networks (documented decision); a Companion on
   the server machine works without a key, others add the site key once (`Authorization` header
   or `?token=`). An "allow this address" list can come later if churches ask.
2. *Default port?* **50001**, configurable; churches set it to the port their profiles already
   use, so only the address changes.
3. *Who may switch it on?* Any paired client (as for the other show settings); it grants nothing
   the main API doesn't.
4. *Module API version?* `@companion-module/base` 1.13 (Companion 4.1+, with secrets for the
   key); move to 2.x when the churches' Companions are new enough.
