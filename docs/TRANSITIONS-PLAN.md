# Transitions, scrolling text and video elements (G16)

Batch 8, lane T. Design note for the rendering half of G16 in `docs/ORIEL-REDESIGN-PLAN.md`
(section 4.3, the slide editor) and the Editor board (its Presentation tab's transition grid,
the Slide tab's transition select, the Text tab's "Scrolling" section, the Shape tab's stroke
styles). Status per stage at the end; all four stages are done.

## 1. Transitions

**Model** (`crates/model/src/effects.rs`, one type for every layer; lane L's look transitions
use it as is). A `Transition` is `{ kind, duration, easing }`:

- `kind` (`TransitionKind`, tagged by `type`): `cut`, `dissolve`, `fadeThroughColor`, and the
  geometric kinds, each with a direction where it makes sense:
  - `push { direction }`: the new content pushes the old out, both moving in `direction`;
  - `wipe { direction }`: a hard edge moving in `direction` reveals the new content
    (diagonals reveal from a corner, `center` opens a box from the middle);
  - `slide { direction }` (new): the new content slides in over the old, which stays;
  - `zoom { direction: in | out }` (new): the new content grows in from the centre (`in`) or
    shrinks onto the screen (`out`) while the two cross-fade.
  - `vendor`, `unsupported`, `unknown` stay as they are and play as a dissolve.
  Directions are `Direction` (the compass type vendor parameters already use); push and slide
  treat `center` / `unknown` as `left`.
- `duration`: seconds, as today.
- `easing` (new, `linear` default so stored dissolves play exactly as before): `linear`,
  `easeIn`, `easeOut`, `easeInOut` (cubic). Oriel picks `easeInOut` for the moving kinds.

Old servers read the new kinds as `unknown` (a dissolve) and ignore `easing`: no migration.

**Which transition plays** is unchanged (`Transition::resolve`): the command's one-shot
transition → the slide's → the presentation's → the layer override → the show's layer default.
The **theme default** is the template slide's transition: a slide linked to its theme takes
the template's unless it overrides `transition` (`theme_link`, batch 5). So "per slide / per
presentation / per theme" already exist as storage; this lane adds kinds, not levels.

**Engine** (`crates/vesper-server/src/engine.rs`). A `Track`'s `Trans` keeps the renderable kind
and easing next to its duration and store key. Progress is still timed from the layer change on
the screen's clock (so every screen runs in step); the eased progress and the kind go to the
renderer in the `LayerMix`. Nothing else changes: an interrupted transition still starts from
the last blend (`MixInput::Snapshot`), the outgoing video keeps playing, and the media,
announcements, props and messages layers get the new kinds through the same tracks.

**Render** (`crates/render`, `LayerMix` gains `kind: model::TransitionKind`). Dissolve (and every
kind that plays as one) keeps today's single pass, bit for bit. A geometric kind draws each side
into a screen-sized side texture (a snapshot side is already one), then composites them into the
mix store with a transform or a clip: push translates both, slide translates the new one and clips
the old one to what it doesn't cover, wipe clips both (the old one to the complement of the
revealed rectangle, so transparent slides don't show each other), zoom scales both about the
centre and cross-fades them in linear light like a dissolve. Two extra passes for the length of a
transition; no new shader.

**Import** (`crates/pro-import`). Vendor transitions named Push, Wipe, Slide (and Cover, Move In),
Zoom (Zoom In, Zoom Out) map to the typed kinds with the effect's first direction parameter; others
stay `vendor` (a dissolve).

**API.** No new message: `Transition` is already in `updatePresentation` / `updateSlide` edit
ops, trigger commands, `setTransitionOverride`, looks, props and messages; `TransitionState` in
the show state carries the new kinds. Bindings regenerated; README notes the new kinds.

**Oriel.** The editor's transitions grid (Presentation and Slide tabs) enables Push, Wipe, Zoom
and Slide, with a direction picker (arrows; zoom: in/out), duration and easing. The Show page
plays them (the server draws them; nothing to do client-side).

## 2. Scrolling text

**Model.** `Element.scroll: TextScroll` (a decoration: always present, `enabled` flag, omitted
when default): `direction` (left, right, up, down), `speed` (canvas px per second, 120),
`start` (`auto`: on screen when the text fits its box, else off it; `offscreen`; `onscreen`),
`repeat` (seamless loop, default on) and `gap` (canvas px between copies, ink to ink, 120).
`ticker` stays as imported provenance; the importer fills `scroll` from ProPresenter's text
scroller (rate 0..1 as 240 px/s at full speed, direction, repeat, start off screen). Its fades
stay in `import.unmapped` (deferred). Theme links treat `scroll` as an element property.

**Render** (`crates/render/src/scroll.rs`). The slide is drawn without the scrolling elements'
text (their fill, stroke and shadow stay) as the usual cached slide layer. Each scrolling
element's text is laid out once as a strip (unwrapped for left/right with the box's height and
vertical alignment, wrapped to the box width for up/down) into a cached texture; each frame
draws the strip's copies clipped to the box at `speed · t`, `t` being the seconds since the
slide went live on that screen's track (`LayerSource::Slide::time`; `None` for thumbnails and
`/render`, which show it still, where it is laid out). A slide with scrolling text makes its
screen render every tick, like a video. Limits: drawn above the slide's other elements, no
rotation, cut off past the largest GPU texture.

## 3. Video elements

An element whose enabled media fill is a video asset plays it inside its box
(`crates/vesper-server/src/element_video.rs`). The render thread keeps a registry of element
players keyed by (slide content, element): the first frame asked for opens a `media::Player`
on its own thread (opening blocks: file, decoder, first frames), so the render loop never
waits; until it is open, or while a frame is not decoded yet, the element shows no picture.
The player follows the fill's `playback`: `repeat` (loop, count, once holding the last frame),
`inPoint` (the start offset), `outPoint`; it was muted in this version (sound: see "Video element sound" below, batch 11). The clock is the
slide's: seconds since it went live on the track (the same `since` scrolling text uses), times
`rate`. A player not asked for in 2 s closes off the render thread. The renderer draws the
slide with the video elements' fills taken out (cached) and each current frame in its
element's box with the fill's display (fit / fill / stretch, clipped to the box) through the
existing NV12 / P010 upload (`LayerSource::Slide::videos`). A slide with video elements makes
its screen render every tick. Limits: drawn above the slide's other elements, no rotation,
thumbnails and the editor show no picture for a video fill (as before), one player per screen
showing the slide (batch 11: one for every screen on the slide layer).

## 4. Dotted and double strokes

`Stroke.style` already had `squareDash` (dotted), `shortDash`, `longDash` and `custom`; this
adds `double`: two lines, each a third of the width, a third apart, drawn by the analytic
stroke pass on rectangles, rounded rectangles and ellipses (other shapes draw it solid; older
servers read it as solid). Oriel's Shape tab offers Solid, Dashed, Dotted and Double.

## Owner questions (answered with a recommendation)

- *Easing for imported and existing transitions?* Linear, as today; only new picks from Oriel
  get ease-in-out for the moving kinds. Nothing that plays today changes.
- *Fade through colour* stays a plain dissolve until a show asks for it (no import maps to it).
- *A theme-level transition* is the template slide's (exists); no separate theme field.

## Status (batch 8, 2026-11-24)

- **Stage 0** (this note): done.
- **Stage 1, transitions:** done. Model (`slide`, `zoom`, `easing`), render
  (`crates/render/src/transition.rs`, `LayerMix::kind`), engine (kind and eased progress per
  track), import (Push / Wipe / Slide / Zoom vendor effects), README, Oriel grid with direction,
  duration and easing. Tests: render probes at 0 / 50 / 100 % for each kind, a mid-transition
  retrigger, a server test of a push on the screen clock.
- **Stage 2, scrolling text:** done (`Element.scroll`, `crates/render/src/scroll.rs`, engine
  clock, import, Oriel insert and Text-tab section). Not done: left/right fades (ProPresenter's
  `fadeLeft` / `fadeRight` stay in `import.unmapped`), rotation of scrolling elements, keeping
  their z-order (they draw above the slide's other elements).
- **Stage 3, video elements:** done, muted (above). Oriel: Media ▸ Video inserts one (a
  looping media fill), and the Shape tab's fill has Loop and Start at. Tests: render (a frame
  in an element box, clipped), server (a counter video in an element advances at its frame
  rate on the screen clock, without a media-layer play).
- **Stage 4, double strokes:** done.

**Remaining (G16):** scrolling text fades; keeping the z-order and rotation of scrolling and
video elements (they draw above the slide); a poster frame for thumbnails and the editor
(video element sound and one shared player for every screen: done in batch 11 for the slide
layer). Link sources, visibility conditions and the artwork field: batch 12, section 5.

## Video element sound (batch 11, lane A: built as planned)

Video elements on the slide layer play with their sound (`crates/vesper-server/src/
element_video.rs`). The design as planned in batch 9, with what the build settled:

- **Model.** The element's fill already has `playback`; its sound settings use the media
  layer's `AudioSettings` (level, mute, channel routes) as a new `FillKind::Media.audio`
  (omitted = full level, unmuted), shown in the Shape tab's fill section as a mute toggle and
  a level slider.
- **Engine.** Element players move from the render thread's registry to the show actor's
  player registry (`crate::playback::LiveVideo`, keyed by slide content and element), opened
  with `audio: Some(channels)` at rate 1. Each gets a `SyncedClock` started when its slide goes
  live on the first screen that shows it, and a `PlaySource` in the audio engine on the slide
  group (`sound::GROUP_SLIDE`, so the slide layer's level and mute apply), with the element's
  `AudioSettings`. The render thread then asks the player for the frame at the clock's time
  (`LiveVideo::time_at`), exactly as for the media layer, so picture and sound stay in step
  (the sound is the clock).
- **Lifetime.** The sound starts when the slide goes live, fades with the slide layer's
  transition when it is replaced or cleared, and one player serves every screen showing the
  slide (which also removes today's one-player-per-screen limit).
- **Tests.** A server test with a synthetic clip with a tone: the audio engine's slide group
  meters rise while the slide is live and fall after a clear; the element's frame time follows
  the audio clock (as `audio::video_follows_the_sound_clock` does for the media layer).

As built (batch 11):
- **Which elements sound:** the video elements of the slide on the slide layer (the held copy
  of a deleted live slide too), keyed by (slide, element): the same slide re-triggered keeps
  its play, going to another slide and back starts over (the screens do the same). Not while a
  foreground video hides the slide. Props, messages, announcements and a look's template
  override (its elements are the template's) keep the render thread's muted players.
- **Opening** happens on a thread of its own, not in the actor: the slide goes live at once
  (text, other elements), and the play's clock starts when its player is open, **at the
  slide's time** (see "Starting at the slide's time" below). The actor inserts the play (still
  opening) before it publishes the state, so the render thread rarely opens a muted player of
  its own for it (only when it draws new content before the state goes out, such as a live
  edit that adds the element).
- **Lifetime:** a play that is no longer wanted leaves the registry and its sound fades with
  the slide layer's transition; the render thread keeps the play of the outgoing side of a
  dissolve until it is done (its picture keeps moving). A new level, mute or routing of the
  fill applies in place (no reopen); another video or playback reopens.
- **Oriel:** the Shape tab's fill section has Sound: Mute and Level (dB).
- **Import:** a media fill's audio properties (volume, channel routing) now map to `audio`
  (they were counted as `fill.media.audio` unmapped).
- **Tests:** `audio::a_video_element_plays_its_sound_in_step` (beeps heard within a frame of
  their flashes on the element's clock; the slide layer's mute and the fill's mute silence
  it; another slide replaces its play; a clear stops it);
  `audio::a_video_element_is_heard_only_while_the_screens_draw_it` (a build-in, a template
  override look, a timer condition); `audio::a_video_element_that_failed_to_open_opens_when_triggered_again`;
  `element_video::tests` (a replaced play takes over the render entry; a failed play is not
  drawn from).
- **Sound follows what the screens draw (bug run 3, E01-1):** an element hidden by an object
  build (before its build-in step, after its build-out step), a failed visibility condition, or
  a live look under which no audience screen draws the slide as authored (slide layer hidden or
  a template override everywhere) keeps its play open and in step with its source muted, so a
  reveal is heard in sync. The actor re-checks with every state change (build steps, timer
  commands, the look, video inputs) and when a timer's run state changes (a countdown running
  out). A build-out mutes at the step (the mixer's one-block glide), not over the build's
  animation.
- **A play that could not be opened (X3-2)** (file missing or still copying, media not mounted)
  is reported to the operator (a keyed notice) and opened again on the slide layer's next
  change (triggering the slide again); meanwhile the render thread draws its own muted player.
  A render entry always draws the registered play: one the actor replaced (the element left and
  came back within 2 s) takes over at once (E01-2).
- **Limits:** sound at rate 1 only (as for the media layer); no "used by" fade other than the
  layer's transition; a slide with the same video element on several layers plays the slide
  layer's with sound only.
- **Starting at the slide's time (bug run 4 follow-up, owner's decision, 2026-11-25):** until
  then a play started at its in point when it opened, while a muted player of the render
  thread that drew the element first was timed from the slide going live: when the play took
  over, the picture stepped back by the opening delay (M-3), and by much more for an element
  added to the live slide by an edit. Now the play opens its decoder at the slide's time and
  starts its clock and sound at the slide's time at the instant its first sound is heard: the
  seconds since the slide layer's last change (the instant the render thread's muted players
  count from) times the fill's rate, on the same timeline (in point, passes of a loop, out
  point). The picture continues without stepping back and the sound comes in at the matching
  point; the sound is still the clock. Edge cases:
  - a clip that does not repeat (or a `loopFor` / `loopCount` play) whose end is already past
    when its play opens holds its last frame and is not heard (no audio source);
  - a looping clip comes in at its place in the current pass;
  - the sound fades in over what is left of the slide layer's transition (none once it is
    over: it cuts in with the mixer's one-block glide);
  - a play opened again after a failure (X3-2), or after the element left and came back, counts
    from that new trigger, so it starts near its in point as before; the same slide triggered
    again keeps its open play (unchanged);
  - hidden elements (builds, conditions, the live look) open the same way, muted and in step;
  - kept as before: a new look that restyles the slide restarts the render thread's muted
    clock, not an open play; a play opening after such a look counts from the slide's go-live.
  - one start of the first build step (bug run 5, H04-2 / R2-2): the same slide triggered
    again goes live again on the screens too (its clock restarts: step-0 timed builds replay,
    muted players start over), and the element sound counts its step-0 builds from that
    trigger, or from a later look change that showed the slide layer again; an open play
    keeps going, and a play on another timeline than the render's muted player takes over at
    once (no hand-over hold);
  - after an output stall the element's sound skips the gap (owner, M-5), so its picture never
    steps back (`sound::tests::a_stall_never_steps_the_picture_back`).
  Test: `video::video_element_starts_at_the_slides_time` (a looping counter clip with beeps added
  2.5 s after its slide went live: the screen's picture is within 200 ms of the slide's time
  and never goes backwards, and every beep is heard within 1.5 frames of its place on that
  timeline; a once clip added after its end shows its last frame and is silent).


## 5. Link sources, visibility conditions and song artwork (batch 12, lane G)

The rest of G16 ("more link sources and visibility conditions, artwork field"). Live data stays
in `crates/render/src/live.rs` (resolved per frame against a `DataContext`); the server fills the
context (`crates/vesper-server/src/live.rs`, `engine.rs::live_context`).

**Link sources** (`DataLink`, text unless noted):
- `timer.show`: `value` (default, as today), `elapsed` (a countdown's duration minus what is
  left; a count-up timer's value), `remaining` (a countdown's value; a count-up timer's end
  minus its value). Nothing to show (a countdown to a time has no elapsed, a count-up without
  an end no remaining) renders empty. `TimerValue` gains the timer's `total`.
- `presentationName { source: presentation | announcements }`: the live presentation's name on
  the slide (or announcements) layer. Import: the `presentation` link.
- `presentationNotes`: the live presentation's notes. Import: `presentationNotes`.
- `ccliText` (imported already, rendered as authored text until now): the live song's
  copyright line from its `songInfo` and the show's CCLI settings (`ccli.layout` tokens; the
  built-in layout when empty: title, author, `© year publisher`, `CCLI Song # n`, licence,
  one per line, empty parts left out). Empty when the live presentation has no song info.
  (Drawing the CCLI line on its own on the first / last slides was not part of this lane; done
  in batch 14, see the status below.)
- `songArtwork` (a **picture** link, like `slideImage`): the element shows the live song's
  artwork image (`songInfo.artwork`, a media asset) as its fill, fitted; no artwork = the
  element as authored with its fill off.

**Visibility conditions** (`VisibilityCondition`, all must hold as today):
- `message { message?, active }`: a message is on the messages layer (any, or that one).
- `stageMessage { active }`: a stage message is showing.
- `layer { layer, active }`: a show layer has content (video input, media, slide,
  announcements, props, messages, audio).
- `playlistItem { which, active }`: the playlist around the live item has a current / next
  item / header (e.g. "Up next" only when there is a next item).
- `videoCountdown { state }` / `audioCountdown { state }`: the presentation video or the audio
  layer `hasTimeRemaining` / `hasExpired` / `isRunning` / `notRunning` / `looping` /
  `notLooping`. Import: ProPresenter's video and audio countdown visibility conditions (today
  `unsupported`).

**Song artwork field.** `SongInfo.artwork: Link<MediaAssetId>` (an image from the media bins /
Codex). Oriel: the Presentation tab's Copyright section gets an Artwork row (pick from the
media bins, clear). Import: ProPresenter keeps artwork as embedded image bytes, which the
importer cannot write as media files yet; it stays in `import.unmapped` (as today).

**API.** `setElement` gains `visibility` (the whole list) and `dataLink` (`null` clears), so
Oriel can add and remove conditions and change a link's settings. Bindings and the protocol
README follow.

**Oriel.** The editor's Shape tab gets a **Visibility** section: the object's conditions, each
with its fields (other object, timer, input, message, layer, playlist item, media state) and a
remove button; "Add condition" offers every kind. The insert menu's data links add Presentation
Name, Presentation Notes, Group Name, Slide Label, Slide Count, CCLI Copyright and Song Artwork;
a timer link's Shape tab offers Value / Elapsed / Remaining.

**Owner questions (answered with a recommendation).**
- *Which "playlist item" condition?* Whether the current / next item (or header) exists: it
  needs no item picker and covers "Up next" boxes; "a given presentation is live" can follow.
- *Artwork import?* Not now: writing embedded images as media files is an importer feature of
  its own; the bytes stay in `import.unmapped`.
- *Media "has expired" with nothing playing?* Counts as expired (no time remaining).

**Status (batch 12, 2026-11-25):**
- **Stage 0** (this note): done.
- **Server:** done as planned. Links and conditions in `crates/render/src/live.rs`, the context in
  `crates/vesper-server/src/live.rs` (`ccli_line`, `layers_with_content`) and `engine.rs`
  (media states); the video-input condition now sees the live input (it never held before).
  `setElement` takes `visibility` and `dataLink`. The Codex counts a song's artwork among the
  presentation's media (`presentation_assets`), so a show box gets it with the song.
  Import: `presentation` / `presentationNotes` links, video / audio countdown conditions.
  Tests: render unit tests per link and condition, the copyright line, the importer, and
  `live_data::presentation_links_artwork_and_show_conditions` (linked text renders like the
  same text authored; artwork fills its frame; message, stage message and props-layer
  conditions follow the show; `setElement {visibility}` on the live slide).
- **Oriel:** done. Shape tab (slide editor) and Object tab (Library song editor): a Live Data
  section for timer links (Value / Elapsed / Remaining) and presentation names (slide or
  announcements layer), and the **Visibility** section (`InspVisibility.svelte`,
  `lib/edit/visibility.ts`); the Live text menu adds Slide Label, Group Name, Slide Number,
  Presentation Name, Presentation Notes, CCLI Copyright and Song Artwork (a square frame); the
  Presentation tab's Copyright section has Artwork (choose from the media bins, remove).
  Stub: a condition on the title slide's lower bar. Shots: `edit-song-artwork`,
  `edit-live-text-menu`, `edit-visibility`, `edit-visibility-added`.
- **Done since (batch 14, leftovers lane):** the CCLI line drawn on its own on a song's first /
  last slides (`ccli.showOn`): `live::ccli_shown` / `with_ccli` put the CCLI template (or a
  built-in bottom-left line) over the slide layer's slide in `show::plan_screen`, counted in the
  play order; test `live_data::ccli_line_on_a_songs_first_and_last_slides`. Oriel: "Add
  condition" offers **Video input live**, with the show's video inputs to pick from
  (`GET /video-inputs`, `state/sources.svelte.ts`) and live / not live; shot
  `edit-visibility-input`.
  The video element sound check (`element_video::drawn_elements`) now resolves conditions
  against the screens' own live data (`engine::live_context`, media states included), and the
  actor re-checks it with every media tick while media plays: an element shown or hidden by a
  video / audio state condition is heard exactly while drawn (test
  `audio::a_video_element_with_a_media_condition_is_heard_while_drawn`).
- **Not done:** conditions on a "given presentation is live"; importing artwork bytes.
