# Vesper Mosaic: displays and channels (plan)

**Owner's decision (2026-11-25, option A):** a Mosaic box runs the **IDN Node with IDN's display
service** (`idn-display --service`, the Node's `apps.display`). **Vesper's site server manages the
boxes; Oriel is the UI.** Vesper does not run or render on a Mosaic box. `docs/PRODUCTS.md`
("Vesper Mosaic") has the product view; this is the technical plan. Rounds: **M1**
(displays, screens, channels, health, the Displays view), **M2** (video walls and the wall
designer, below), then signage channels rendered by Vesper.

## What a display box is

A **display box** is a machine of the site whose IDN Node reports the `display` service
(`SiteDevice.services`, from IDN's `DeviceHealth::services`), whatever its state (`running`,
`no_session`, `failed`…: shown as it is). An admin can also add a machine by hand (its Node is
older, or doesn't report the service yet). The Launcher's first-run wizard calls the role
**"display box (Mosaic)"**: on such a machine it installs the IDN Node and switches its
`apps.display` on; nothing of Vesper runs there.

**Screens of a Vesper server are never Mosaic's** (decided 2026-11-25). A display
physically connected to a box running a Vesper server is driven by Vesper's own display outputs
(Configure Screens ▸ Outputs ▸ System display: KMS or windowed full screen, no encode,
frame-accurate, with layouts, Fill/Key and corner pin). IDN's display service must not drive it
as well (the README: two programs on one screen fight). So Mosaic manages display-service
screens only on machines that don't run a Vesper server:

- **A Vesper machine** is one whose IDN Node a Vesper server reported as its own (batch 15: each
  server connects to its machine's Node, `Node::connect("Vesper")`, reads
  `enrollment().fingerprint()` and sends it over the Codex link's presence report; the site's
  server keeps them in `mosaic.json`, one per reporting box, and its own), one whose Node reports
  a Vesper service, or a Node machine named like a Vesper box of the site (`<name> (Vesper)`: the
  fallback for a box without a Node or from before). **Never for good** (bug run 6): a reported
  Node counts while its box has reported within 15 minutes, a name while its `(Vesper)` device
  isn't removed and was online within 15 minutes; on a machine whose Launcher reports the
  display role (and no presentation server) only a Vesper server heard from right now counts
  (a report within a minute, its device online), so a machine re-roled as a display box can be
  managed at once.
- Such a machine is listed only when its display service is on too, as a **conflict** ("switch
  the Node's display service off there"), and its screens are never assigned (`409`).
- Oriel's Displays view shows **this server's own outputs read-only** ("driven by Vesper on this
  server", with a link to Configure Screens), so the room's whole picture is in one place.

The display service has an API on each box: `http://127.0.0.1:29222/api/v1/` locally and, once
the box is in a site, `https://HOST:29223/api/v1/` for site tokens of the engine `display`
(`media/idn-display/README.md`). Vesper uses only the network one, from the site's server.

## Model

| Thing | What it is | Kept |
|---|---|---|
| **Display box** | A site machine (its fingerprint) with the display service; its address; whether it answered; its screens | Address in `mosaic.json`; the rest read from the box |
| **Screen** | One output of a box (`number` from 1, its `name` from the service); its **feed** and **settings** | Feed and settings in `mosaic.json` (Vesper's wish) and on the box (`screens.json`, by screen name: it keeps showing while Vesper is down) |
| **Feed** | What a screen shows: nothing (back to the desktop), a **source** (`HOST (name)`), or a **channel** | per screen |
| **Channel** | A site-wide named feed ("Main", "Lobby") pointing at one source; every screen following it moves when its source changes | `mosaic.json` |
| **Settings** | Fit (fit / fill / stretch), rotation, overscan, sound (card, level), fallback (last picture, then the site's logo after N s, or black), and a wall **tile** | per screen |
| **Wall** (M2) | A named grid (rows × columns), its bezels and the screens' visible size (mm), a **cell** per screen (on one box or several), and one feed (a source or a channel) | `mosaic.json`; each tile is pushed as the screen's `settings.wall` |

Health per screen: `showing` (has a feed and the box answers), `idle` (no feed), `offline` (the box
doesn't answer or is offline in the site), `error` (the box refused the last change: the reason).

**Where it is stored (recommendation: the site's server, not the Codex).** `mosaic.json` in the
site's server's machine folder (`<data>/machine/`, beside `site.json`), so it is part of that box's
settings backup and restore. The Codex holds content in collections; displays and channels are
site infrastructure (like the site's devices), and the signage *content* a channel shows later
lives in the Codex like other content. Each box also keeps its own feeds and settings, so screens
keep their picture while the site's server is down.

## Video walls (M2)

A **wall** is a name, rows × columns (1–16 each), the bezels between screens (horizontal and
vertical, mm) with the screens' visible size (needed with a bezel), the cells holding a screen
(a cell may stay empty), and the wall's **feed** (a source or a channel, like a screen's). A wall
of one box's several screens and a wall of several boxes are the same thing: the display service
handles both alike.

- **Applying a wall** gives every tile the same `settings.wall {rows, cols, row, col, bezel_mm,
  screen_mm, group}` with its own row and column, one explicit **sync group** named after the
  wall (`wall:<name>`; names are 1–48 characters, so the group stays within IDN's 64), and the
  wall's feed (the same source string on every tile: different senders stay in step only per
  sender). Settings go before the feed, so the first picture is cut right. The picture covers
  the whole wall, bezels included, and `fit` applies to the wall as a whole.
- **Rules** (`400`/`404`/`409`): each cell inside the grid and holding at most one screen, a
  screen in at most one cell and one wall, only display boxes' screens (a Vesper machine's never),
  a known channel. A wall's tile takes its feed from the wall (`409` on the screen's own feed);
  its other settings (fit, rotation, sound, fallback) stay per screen.
- **Changing a wall** (`PUT`, whole): moved screens get their new tile, screens taken out lose
  theirs (`wall: null`) and keep showing the wall's feed, whole; deleting a wall does the same for
  all its screens. A forgotten box's screens leave the walls they were in.
- **Kept right:** the 5-second poll compares each screen's settings as the box reports them with
  Vesper's (fit, rotation and the wall tile) and sets them again when the box lost them (replaced
  or restarted), as it does for feeds.
- **Channels:** a wall can follow a channel; its tiles then follow it like any screen, so a
  channel's new source moves the whole wall (a wall following a channel keeps the channel from
  being deleted).
- **Sync health** from each tile's `stats.sync {group, mode, delay_ms, late}`: a wall is **in
  step** when every tile showing a picture is in the wall's group and none dropped late pictures
  since the read before; its state is its tiles' worst, with the first problem in words ("Row 2,
  column 1 (Lobby · HDMI-1) is in another sync group"), the largest delay and the late pictures.
- **Identify a wall:** every tile's screen shows the service's identify overlay, which on a wall
  tile shows its place itself ("Wall R1 C2 of 2×3 · group", IDN Beta12), and Oriel's preview shows
  each tile's row and column large while it does.
- **Oriel** (Site ▸ Displays ▸ Video walls): each wall as a card with its picture (the source's
  thumbnail cut by the tiles, bezel gaps left out), its state and sync, what it shows, Identify,
  Edit, Delete; the **wall designer** (name, rows × columns, bezels, screen size, feed; screens
  dragged from the Displays grid or the designer's list into the cells, or picked per cell).
- **Not Mosaic:** a wall of the *server's own* outputs (displays plugged into the Vesper server
  itself) is Configure Screens' Grouped / Edge Blend (batch 12), frame-accurate in one renderer.

## How Vesper talks to a box

The IDN maintainers answered the display-service questions on 2026-11-25 (trusted, documented
semantics, IDN v0.3.4-Beta11). One small client module (`crates/vesper-server/src/mosaic/client.rs`)
holds every detail of the service's API:

- **Token:** the site's server issues it itself through the Site Manager it hosts
  (`Handle::issue_token`, user `vesper-mosaic`, audience `{engine: "display", machine: <the box's
  fingerprint>}`, scope `control`, one hour, renewed ten minutes before it ends; IDN allows at
  most 12 h). Per box rather than one site-wide token: it limits a leak. Sent as
  `Authorization: Bearer idn1.…`; every change carries `X-IDN-Display: 1`.
- **Address and certificate (batch 15, IDN Beta12):** the box is reached where the Site Manager
  sees its IDN Node (`Authority::node_reports()`'s `address`), an address an admin sets
  overriding it, the join request's address last (`MosaicBox.addressFrom`). HTTPS accepts only
  the box's own site LAN certificate (`Authority::lan_client_config(fp)`: the site CA, that
  machine's fingerprint, at any address); another machine of the site answering with its own
  valid certificate, or a self-signed one, is refused. Plain HTTP only to this machine (a test's
  loopback fake).
- **Reading:** `GET /api/v1/state` (each screen's id, name, number, size, feed, `routed_to`, error,
  `status {state: idle|connecting|playing|lost|fallback, showing, reason}`, settings, test
  pattern); `GET /api/v1/thumbnail?screen=N&width=W` (JPEG; `404` when it shows nothing).
- **Changes:** `POST /api/v1/feed {screen, source}` (`""`: back to the desktop; the picture holds
  until the new feed's first one), `settings {screen, fit, rotation, overscan, audio, fallback,
  wall}` (only fields sent change; `audio`, `fallback`, `wall` whole), `identify {screen|"all",
  seconds}`, `test_pattern {screen|"all", on, seconds}`. Errors are `{"ok": false, "error"}`.

**Channels are IDN routers (batch 15)** on the site server's own IDN Node: `routers.list`
(`Channel Main=BOOTH (Program); Channel Lobby=…`, "Channel" before the name so it never takes the
name of one of the server's own sources) applied with `SiteCommand::Apply` as the server; screens
and walls following a channel are fed `SITEHOST (Channel Main)`, and a new source is one
`routers.route` (the screens hold their picture and cut to the new one; the Node writes it into
its saved list, so it survives a Node restart). Sending `routers.list` puts **every** router back
to the sources in it (the IDN maintainers, 2026-11-26), so the list always carries each channel's
current source; at the server's start it isn't sent when the Node's report (`<routers list>`)
already shows the same list. A rename sends the list again and re-points the followers. Without a Node on the site's server, or while it refuses the
list (`MosaicState.routingProblem`, tried again every 30 s or when the channels change), each
screen is given the channel's source itself (as M1 did); existing channels and their screens move
onto routers by themselves once the list is taken. A refused `routers.list` or `routers.route`
(the Node went away) drops the routers until the list is taken again, so the screens are given
the sources themselves meanwhile (bug run 6). A channel whose source has a `;` (the list's
separator) isn't on a router: its screens get the source itself. Mosaic owns `routers.list` on
that Node once it has channels (owner question 7).

**Keeping boxes right:** every 5 s the site's server reads each box's state; a screen whose feed
isn't the one Vesper set (a replaced or restarted box) is set again, and changes are announced
with `siteChanged` (`mosaic`).

**Fallback content (batch 15, fixed in 15b).** The site's logo is set in Site ▸ Displays (`PUT
/mosaic/logo`, a PNG or JPEG kept as PNG in `<data>/machine/mosaic-logo.png`). The site's server
puts it at the top of its own Node's media folder and sends it with `media_send replace="true"`
(lane I, IDN Beta14) to every box online in the site, under **two names in turns**
(`logo-a.png`, `logo-b.png`; owner approved 2026-11-26): a screen already showing a logo reads
its file again only when its path changes (the IDN maintainers' answer,
`2026-11-26-display-replaced-logo-pickup.md`), so a new logo goes under the name the box's
screens **don't** show, replacing what the box had under it once the new one is whole and
checked. The sending Node's transfer goes `offered` → `sent` (the box fetched all of it) or
`failed`; Vesper follows it (the Node's `<media>` transfers) and, once it is `sent`, points each of
the box's screens at it as its own fallback logo (`settings.fallback.logo` = `logo-b.png`,
relative to the box's media folder): the new path makes a screen showing the logo show the new
one at once. Until then the screens keep the old name (and picture), also after a transfer that
failed or stuck (bug run 7, H03-1: they are pointed at a name only once its transfer reached
`sent`). Sent again under the same name until it arrives; the next logo after it takes the other
name (a → b → a). Then again on every poll until the box reports it. A failed transfer is tried
again after a minute; one whose fetching hasn't moved for two minutes (the transfer's `done` bytes,
or no longer listed) at once. One the box is still fetching is waited for however long it takes:
two transfers of one name never run at once (H03-2). **Boxes from before Beta14** (their Node's build, or their refusing
the replacement) never overwrite a file (a second `logo.png` would arrive as "logo (2).png") and
let the stream go early (their sender says `failed` though the file arrived), so they get the logo
the batch 15 way: under a name of its own, `logo-<hash>.png` (the site's server removes the ones
it sent before from its own media folder), trusted as there **15 s after** their Node accepted it
(only the box knows its own "received"). Likewise when the site server's own Node is from before
Beta14 (its answer names no offer). `MosaicBox.logoState`: `sending`, `onBox` (the box has it and
every screen has it), `failed` (its Node refused it, or the transfer failed; tried again after a
minute). A fallback sent whole by Vesper keeps
the logo, and what the box has that Vesper can't say (a slideshow fallback, `slide_s`,
`reconnect_cap`, the sound's channel map while the device stays) is kept in a fallback or sound
Vesper sends whole (bug run 6). The box keeps its feeds, settings and logo while the site's
server is down.

**Setting a box up (batch 15).** Site ▸ Displays lists the site's other machines with an IDN
Node ("Other machines of the site"); **Use as a display box** switches the machine's display
service on through its Node's settings (`SiteCommand::Apply`): **headless** from boot on Linux
(`apps.display` **and** `apps.display_boot` on; IDN Beta12's KMS display service, recommended
for a dedicated box) or in the desktop session (`apps.display` on, `apps.display_boot` off; on
Windows with auto-logon); off is both off. A box's panel switches it. The mode shown is the one
the box's Node reports (`serviceModeReported`; what the Site Manager's status passes on since IDN
Beta14 as `machines[].node.settings`, read from the same Node reports through the authority): its
display service's `runs` (`node`: from boot, headless; `session`: the desktop session) and `on`,
else its `<apps display display_boot>`; else the one Vesper last set.

## API (site's server; `crates/protocol/README.md` "Mosaic")

`GET /mosaic` (boxes, screens, channels, the logo, the other Node machines) · `GET/PUT/DELETE
/mosaic/logo` (admins change it) · `PUT /mosaic/boxes/{machine}/service` (admins) · `PUT/DELETE /mosaic/boxes/{machine}` (address; add or
forget a box: admins) · `PUT …/screens/{n}/feed` (operators) · `PUT …/screens/{n}/settings`
(admins) · `POST …/screens/{n}/identify`, `…/test-pattern` (operators) · `GET
…/screens/{n}/thumbnail` · `POST /mosaic/identify`, `/mosaic/test-pattern` (every box) · `GET/POST /mosaic/channels`, `PUT/DELETE
/mosaic/channels/{id}` (create, rename, delete: admins; set the source: operators) · `POST
/mosaic/walls`, `PUT/DELETE /mosaic/walls/{id}` (admins) · `PUT /mosaic/walls/{id}/feed`, `POST
/mosaic/walls/{id}/identify` (operators). A server that
isn't the site's server answers `managed: false` and `409` for changes.

## Oriel

**Site ▸ Displays** (shown with the `mosaic` capability): the grid of every screen (thumbnail,
name, box, source or channel, status), click to assign a source or a channel, per-screen
settings, identify and test pattern (each, and for all); the **Channels** list (create, rename,
set source, delete); **this server's own outputs read-only** ("driven by Vesper on this server",
a link to Configure Screens) and any **conflict** (a Vesper box whose Node also runs the display
service). Site ▸ Devices shows a display box's Mosaic status (screens, what they show). Batch 15:
a box's panel says where its address came from (the field is an override) and switches its
display service (headless / desktop / off); **Other machines of the site** offers "Use as a
display box"; the Channels show each one's router; **Site logo** (choose, change, remove; how
many boxes have it).

## Owner questions (with recommendations)

1. **Where Mosaic is kept:** the site's server's machine folder (recommended, above), not the Codex.
2. **Who may do what:** operators assign feeds, identify and switch a channel's source (live
   work); admins add boxes, change screen settings and create / rename / delete channels.
   Recommended as built.
3. ~~Unchecked certificate until IDN answers~~: settled in batch 15 (only the box's own site
   certificate is accepted).
4. **Signage channels** (later): rendered by a Vesper server as an IDN source; a channel then
   points at that source like any other.
5. **A screen leaving a wall** (taken out, or the wall deleted) keeps showing the wall's feed,
   whole (recommended: nothing goes dark mid-service; an operator then gives it its own feed),
   rather than going back to the desktop.
6. **Walls are admins' work** (create, change, delete), the wall's feed and identify operators'
   (live work), as for screens. Recommended as built.
7. **Mosaic owns `routers.list` on the site server's Node** once there are channels (routers an
   admin set there by hand are replaced). Recommended: yes (one place for channels); the
   alternative is merging with the Node's own list, which Vesper can't read back today.
8. **The site's logo goes into the site server's own media folder** (that is what `media_send`
   sends) as `logo-a.png` / `logo-b.png` in turns for Beta14 boxes (decided 2026-11-26: a
   screen reads a logo again only when its path changes), `logo-<hash>.png` for older ones.

## Waits for the IDN maintainers, and later rounds

Done in batch 15: the box's address and its certificate check, which Node is which Vesper,
headless boxes (switched from Oriel), wall tiles identifying their place, channels on IDN
routers, the logo pushed to the boxes.

- Answered 2026-11-26 (`2026-11-25-mosaic-routers-logo-display-switch.md`) and adopted in 15b.
  Built in IDN Beta14 and adopted in lane I: `media_send replace="true"` and the sender-side
  "sent" state (the logo goes without the settle wait on Beta14 boxes; since 2026-11-26 under
  two names in turns, as a screen reads a logo again only when its path changes), and the
  Site Manager's status passing on `<apps>` and the services' `runs`.
- Signage channels (lane S: the site server's IDN outputs, picked like any source).
