# Vesper signage: channels, schedules, playlists, feeds (plan)

The product view is `docs/PRODUCTS.md` ("Vesper Mosaic" ▸ "Signage"); Mosaic's side is
`docs/MOSAIC-PLAN.md`. **Owner's decisions (2026-11-25):** signage is rendered by the **site's
server** (the always-on Vesper server hosting the Site Manager); each signage channel is a screen
of that server with an **IDN output named after the channel** ("Lobby"), so Mosaic screens, walls
and channels follow it like any other source. Weather comes from **Open-Meteo** (no key; the
credit "Weather data by Open-Meteo.com" (CC BY 4.0) shows wherever weather does). No other
internet services.

## Model

| Thing | What it is |
|---|---|
| **Channel** | A name ("Lobby"), a **zone layout**, a **default playlist**, its **schedule** (slots), its ticker text. Owns one screen of the site's server (kind `signage`, 1920×1080@30) and one IDN output whose source name is the channel's name: `HOST (Lobby)`. Renaming renames both; deleting removes both. |
| **Slot** (schedule) | A playlist on some **days** of the week, from a **start** to an **end** time (an end before the start runs past midnight), optionally only **from / until** a date, with a **priority**. The highest priority of the slots active now wins (a tie: the one listed first); with none, the channel's default playlist; with none of that, the channel shows its zones only. |
| **Playlist** | Items in order, looping. Each item: what it shows, a **duration** (1–3600 s; a video loops for it) and a **transition** (cut, dissolve, …: the renderer's own). |
| **Item** | A **slide** of a presentation (Codex content), an **image** or a **video loop** (Codex media assets), a **text** announcement (title + body), or a **feed** slide (a feed's rows through its column mapping). |
| **Zone layout** | Fixed: `full`, `mainTicker`, `mainSidebar`, `mainSidebarTicker`. The main zone shows the playlist; the **sidebar** shows the clock, the **countdown to the next service** and the weather; the **ticker** scrolls the channel's ticker text (or a feed's titles). No free-form designer this round. |
| **Feed** | A name, an address (a published Google Sheet CSV link, a CSV/TSV/JSON/XML/RSS address, or a file in the server's `<data>/signage-feeds/` folder: no other file, and no address of the server itself), a refresh interval and a **column mapping** (`title`, `body`, optional `when`, `image`). The last rows read well are kept (also across restarts) and shown while a read fails. |
| **Settings** | The weather location (lat/lon and the place's name, found with Open-Meteo's geocoding), the temperature unit, and the **service times** (weekly day + time) the countdown counts to: Vesper has no service schedule of its own, so they are set here. |
| **Override** | An urgent message (text, style `urgent` / `info`, optional duration) on all or chosen channels, at once, over everything; cleared by hand or when its duration ends. Audited. |

**Feeds use IDN's support:** IDN's `idn-feeds` crate (public API: `read_table`,
`parse_table`, `sheets_csv`; formats auto/CSV/TSV/JSON/XML-RSS; Google Sheets links read as their
CSV export) reads and parses every feed; Vesper only schedules reads, keeps the last good rows and
maps columns. Open-Meteo is read with the IDN library's HTTP client (`web::httpc`), every 30 min;
the last forecast is cached and the weather zone hides when it is older than 6 h.

## Where it is kept

**`<data>/machine/signage.json` on the site's server** (beside `mosaic.json`; part of that box's
settings backup), with `signage-cache.json` beside it for feed rows and the last forecast.
The brief suggested Codex collection documents for playlists and layouts. This round keeps them
with the channels instead, because: signage renders only on the site's server, which hosts the
Codex anyway; a new Codex collection kind needs writer plans, change logs, show-box mirrors and
migrations of its own; and what a playlist *shows* already lives in the Codex (presentations and
media assets, referred to by ID, from the server's active collection). Moving playlists into a
Codex kind later changes only where they are stored (owner question 1).

## Server

- `crates/vesper-server/src/signage/`: `mod.rs` (store, channels, the per-channel players, the
  tick), `schedule.rs` (pure rules: which playlist plays at a local date-time), `feeds.rs`,
  `weather.rs`, `compose.rs` (the synthetic slides a channel draws).
- **Rendering through the existing renderer:** a signage screen's plan comes from the channel's
  player instead of the show state: the main zone is a slide on the slide layer (a presentation's
  slide scaled into the zone with its own canvas; an image or a video as a media-filled element
  of the zone, which the renderer's muted element players play), the zones and the override are
  one slide on the messages layer. Item changes play the item's transition (the plan carries the
  channel's own layer changes). Signage screens are never operator screens: Show, looks and the
  slide layer of the show don't reach them.
- **The tick** (4 a second): each channel picks its playlist, advances its item when its time is
  up, skips what can't be shown (a missing presentation, slide or media file, an empty feed: logged
  once, never a stop), rebuilds the zones once a second (clock), and wakes the renderer only when
  something drawn changed. Feeds refresh on their own interval off the async threads.
- **Recovery:** channels, playlists and the override are documents; the screen and its output
  are re-created at start when missing (a deleted output, a restored backup). A channel whose
  item can't be shown moves on.
- **API** (`/api/v1/signage/…`, site's server only; others answer `managed: false` and `409`):
  `GET /signage` (everything + now/next/health per channel) · `POST /signage/channels`,
  `PUT/DELETE /signage/channels/{id}` · `POST /signage/playlists`, `PUT/DELETE …/{id}` ·
  `POST /signage/feeds`, `PUT/DELETE …/{id}`, `POST /signage/feeds/preview` · `PUT
  /signage/settings` · `GET /signage/places?q=` (geocoding) · `POST/DELETE /signage/override`.
  Admins create, change and delete; operators push and clear the override. `siteChanged`
  (`signage`) announces changes; overrides are audited.

## Oriel

**Site ▸ Signage** (beside Displays, shown with the `signage` capability): it is site
infrastructure like Displays, and the site's server is the one place it runs. Channels with a live
thumbnail (the channel's screen), now/next and health; a **week calendar** per channel (drag on a
day to make a slot, recurrence = the days ticked, priority, date range, a "now" line); the
**playlist editor** (items with duration and transition, add from presentations/media/feeds/text);
the **zone layout** picker; the **feeds** view (address, preview rows, map columns); settings
(weather place, service times); and a prominent **Override** button (all or chosen channels,
confirm, clear). Mosaic's source picker labels signage channels' sources "Signage: Lobby".

## Owner questions (with recommendations)

1. **Playlists in the site server's `signage.json`** this round (recommended: simplest, backed
   up with the box), moving them into a Codex collection kind when signage content should be
   shared across collections or edited on show boxes.
2. **Service times for the countdown** are set in signage settings (recommended until Vesper has a
   service schedule; Planning Center plans can feed it later, M6).
3. **Override scope:** channels only this round (recommended); "chosen screens" of Mosaic that
   don't follow a signage channel would need the display service's own overlay (a request to the IDN maintainers).
4. **Video sound:** signage videos play muted (recommended for lobbies); sound later per item.
5. **Who may do what:** admins author (channels, playlists, schedules, feeds, settings);
   operators push and clear an override (recommended as built: an emergency shouldn't need an
   admin).
