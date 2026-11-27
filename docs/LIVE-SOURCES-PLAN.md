# Live sources as layer content (G7)

Gap G7 of [ORIEL-REDESIGN-PLAN](ORIEL-REDESIGN-PLAN.md): cameras and other network video
sources (IDN Open Protocol, compatible with NDI®) go live on a screen layer, picked from the
Show page's bin ("Video Inputs" tab). Section 9 tracks what is built.

Inputs: the IDN maintainers' answer on receiving in Beta7 (discovery, `Input`, newest-frame model,
`set_max_size` / `set_low_bandwidth`, automatic reconnects, no A/V-pairing or frame-sync
helper), `docs/INTEGRATION.md` §3–4 of the library, and the existing model: `Layer::VideoInput`
(bottom-most), `VideoInput` documents with a per-box `InputBinding` (`InputSource::Idn
{sourceName}`), `VideoInputTrigger`, `LayerVisibility.videoInput` (the looks' "Video input"
row), `ClearLayer::VideoInput`, `protocol::VideoInputLayerState` (always `null` until now) and
the `triggerVideoInput` command (answered "not implemented" until now).

## 1. Discovery: the source registry

- One `discovery::Browser` per server, started on first use (the first `GET /sources`, a
  trigger, or the bin opening) and kept running. It uses mDNS; when the box is a member of
  a site, the site's discovery server (`BrowserConfig.discovery_server`, the address the box
  joined), restarted when the membership changes (checked every few seconds).
- The registry folds `Added` / `Changed` / `Removed` into a list of `LiveSource { name,
  address, idn, groups, status }`, where `status` is the receive state of a source in use
  (`idle`, `connecting`, `live`, `lost`, plus `width`, `height`, `frameRate` once pictures
  arrive). Sources this box sends itself (its own IDN outputs) are listed too (a loopback is
  useful for tests and for a stage feed), marked `own: true`.
- `GET /api/v1/sources` → `{ sources: LiveSource[] }`; every change of the list or of a
  status sends `sourcesChanged { sources }` on the WebSocket (the whole list: it is small).
- A source that vanishes without a goodbye drops off after about 20 s (library behaviour). A
  source that leaves the list while live stays on its layer (the input keeps reconnecting).

## 2. A source as layer content

**Layer: the video-input layer, not the media layer** (owner-level choice, recommended): the
model already has a dedicated bottom-most `Layer::VideoInput`, the looks' "Video input" row
toggles it, clear groups name it, and imported playlists' video-input items target it. Using
it keeps a camera and a background video independent (a background covers the camera, as on
the board's "No background: the text is keyed over … a camera"), and the looks row works with
no further change.

- `triggerLiveSource { source, transition? }` (new): puts a network source (its full name
  `HOST (name)`, or `ip:port`) live on the video-input layer, with the layer's transition.
- `triggerVideoInput { input }` (existing, until now "not implemented"): a logical input,
  through this box's `InputBinding` to an IDN source. Deferred to stage 3 (the box must first
  load `VideoInput` / `InputBinding` documents; imports already write them).
- `clearLayer videoInput`, `clearAll`, clear groups with `videoInput`, `restoreLayer
  videoInput` (un-clear) work like the media layer.
- State: `layers.videoInput: { input?, name, source }` (`input` becomes optional: the field
  was always `null`, so no client depends on it). The receive status is not in the show state
  (it would publish a new state on every reconnect); clients join it from `sourcesChanged` by
  `source`.
- Display: the whole screen, scale to fit, like a media layer (`MediaDisplay` default).

## 3. Receiving and pacing

- **One receiver per source in use, shared by every screen** (`LiveInputs` hub in `Shared`).
  The render thread asks the hub for a source by name (a map lookup); a new receiver opens on
  its own thread (`Input::open` and everything after it never run on the render thread).
- The receive thread converts each picture into a frame the renderer already draws: the
  library's Y'CbCr planes (8-bit, limited range, BT.709 or what the source says) are packed
  as NV12 (Y as is, Cb/Cr averaged down to 4:2:0 and interleaved, about 1 ms for 1080p) and
  drawn through the existing video path (`render::VideoFrameRef`, per-stream textures, the
  GPU YUV conversion). No `render` crate change. The newest frame replaces the previous one
  in a one-slot mailbox (`Mutex<Option<Arc<LiveFrame>>>`).
- **Newest frame at the screen rate.** A screen showing a live source is "animated": its
  clock asks for a render every tick and the render thread takes the newest frame; it only
  re-renders when the frame's sequence number changed. Camera drift becomes an occasional
  repeated or skipped frame, as the library intends. Nothing waits for a frame:
  `header.timestamp` is never used for pacing.
- **Format changes**: each picture's header is compared with the last; a new size or rate is
  logged and shown in the status, and the renderer re-creates the stream's textures (it does
  so on any size change). Interlaced sources: fields are woven with `input::weave_fields`
  (stage 3; until then a field is shown as a frame).
- **Alpha**: dropped in v1 (drawn opaque). Keyed sources (SHQ7/9, another Vesper's alpha
  output) need an alpha plane in the video path: stage 3, with lane R's keyhole work merged.
- **Size**: the receiver asks for the largest live screen's size (`cfg.max_size`), so an IDN
  4K source sends 1080p to a 1080p box and an NDI 4K camera is decoded at half size.
- **NDI|HX (H.264/HEVC) cameras** arrive compressed; v1 opens no decoder (status
  `unsupported`). Next: `cfg.codec` with idn-ffmpeg (the media crate already links it).
- **Dropped when unused**: a receiver nobody has drawn or asked for in 5 s is closed by the
  registry thread (a quick re-trigger reuses the running one).

## 4. When a source is lost

`State(Lost)` → status `lost`; the library reconnects every 2 s by itself. The last picture
holds for 3 s (a camera glitch or a switcher's reconnect shows no flash), then the layer
draws nothing (the screen's background shows; the video-input layer is the bottom one).
Before the first picture the layer draws nothing (status `connecting`). Oriel shows the
status on the bin tile and on the Show page's layer button.

## 5. Audio

Stage 3 item 3, built in batch 11 (`crates/vesper-server/src/live_sound.rs`):

- **Receiving.** A receiver in use asks for the source's sound (`LiveInputConfig.sound`;
  thumbnail proxies don't) and pushes each block (planar f32, the source's rate and channels,
  the timecode of its first sample) and the arrival of each picture into the source's
  `LiveSoundFeed` (one per source name, kept across reconnects and receiver restarts).
- **The lane.** The show actor adds one `LiveSoundSource` to the audio engine for the source on
  the video-input layer, on its own group (`GROUP_LIVE`: `setVolume` / `setMute` with
  `layer: "videoInput"`, `state.audio.layers.videoInput`), with the layer's `AudioSettings`
  (`LiveVideoInput.audio`: as is for `triggerLiveSource`; a logical input's
  `VideoInput.audio` once `triggerVideoInput` sets it, lane V). Another source cross-fades
  with the layer's transition; a clear fades it out. The sound follows the picture: when no
  screen draws the source for 5 s (a look hiding the layer, test patterns on every screen) its
  receiver closes as before and the sound stops until a screen draws it again (owner question
  below).
- **Pairing by timecode.** A sender gives the sound block attached to a picture that picture's
  timecode. The lane relates the sender's timecodes to this box's clock by the smallest
  transit over the last 2 s (a running minimum of "arrived − timecode" of sound blocks and of
  pictures: it follows the sender's drift) and plays, for each bus block, the source time due
  when it is heard: a picture is shown about 40 ms after it arrives (one screen frame rendered
  ahead, half a source frame waiting), so the sound with its timecode is heard 40 ms after the
  picture's arrival, or later when the sound itself needs it (its transit, 30 ms of jitter, the
  mix-to-hearing latency and a block). On the null output the sound is heard within a frame of
  its picture (the server test measures 8–12 ms early); a sound card's latency makes it that
  much later, within what viewers accept (late sound up to ~100 ms).
- **Drift.** Linear-interpolation resampling to 48 kHz at a rate corrected by up to ±0.5 %,
  steered by a PI loop (time constant 2 s; its integral learns the sender's clock rate) towards
  the due position; more than 60 ms off, it jumps (with a short fade), forward or back (1 s
  of samples is kept behind the read position: the due time moves back when pictures start
  arriving later than the sound, as after an H.264 camera's first keyframe; due earlier than
  that, it is silent until the due time gets there). A gap in the timecodes (over 20 ms)
  starts over (silence until it locks on again); smaller differences between the timecodes and
  the samples (a sender stamping its sound from another clock) are followed, so they become a
  rate difference for the drift loop instead of a periodic restart (bug run 3: R2-1, E02-2).
- **One lane per source.** The lane's engine ID belongs to the source's feed: a source put back
  on the layer while its lane still fades out (clear and Undo, A → B → A within a dissolve)
  gets that lane back, fading in again from its current level, still locked on
  (`audio::Mixer::resume`), never a second lane competing with it for the feed (bug run 3:
  E02-1).
- **NDI|HX AAC sound** (batch 12): `CompressedAudio` frames decode with the receiver's
  FFmpeg adapter (`output::AacSound`) into the same sound blocks (test:
  `live_sound::aac_sound_decodes_into_sound_blocks`).
- **Not covered:** a source used as a slide
  element's fill (lane V) plays no sound; sound with alpha / interlaced sources is unchanged.
- **Tests:** `live_sound::tests` (due time with no drift; a sender 0.1 % fast and slow at
  44.1 kHz with 8 ms jitter stays within 2 ms of due with one lock and no underrun; pairing
  with pictures; a gap re-locks; timecodes from another clock; the due time moving back; a new
  lane takes nothing from the feed until it plays), `audio::mixer::tests::resume_cancels_a_fade_out`, `output::idn_live::tests::sound_arrives_with_its_timecodes`,
  and the server test `live_sound::a_live_source_plays_its_sound_with_its_picture` (a loopback
  camera's beeps heard with its flashes, the layer mute, a clear).

**Owner question (batch 11):** should a live source's sound keep playing while nothing draws
its picture (a look hiding the Video input row on every screen, test patterns everywhere)? The
media layer's sound does. *Recommendation: no for now, as built* — the receiver's idle close
(D02-1) stays as it is and a hidden camera is silent; making the sound independent of the
picture is a small change (a receiver whose sound plays counts as used) if the owner prefers
it, with the D02-1 test then closing the receiver by clearing the layer instead.

No IDN requirement filed: the pairing above is a few dozen lines on top of what the library
documents (separate delivery, block timecode = frame timecode). A pairing / frame-sync helper
in the library would not make this much simpler, since the renderer's newest-frame model and
the bus's own clock are ours either way.

## 6. Thumbnails in the bin

"Live thumbnails if cheap": a tile asks `GET /sources/thumbnail?source=…` (JPEG, 320 wide)
every few seconds while the tab shows. A source already received answers from its newest
frame. Otherwise a **proxy receiver** opens (`low_bandwidth`, `max_size` 320×180,
`picture_every` 15, RGBA from the library), and closes after 10 s without a request. Cost:
a 640-wide picture every half second per visible source, well under 0.05 cores each.

## 7. CPU

Per 1080p30 SpeedHQ source in use: about 0.1 core for decoding (library figure) plus the NV12
packing (~1 ms a frame, ~3 % of a core) and one 3 MB upload per new frame. Shared by every
screen. A source on an idle bin costs nothing; a visible bin tile costs a proxy receiver.

## 8. What the IDN maintainers hear

Nothing is missing for v1: discovery, `Input`, RGBA and planes, `max_size`,
`low_bandwidth`, `picture_every` and reconnects cover it. Anything that misbehaves gets a
black-box report to the IDN maintainers. A/V pairing stays in our engine.

## 9. Stages

| Stage | | State |
|---|---|---|
| 0 | This plan | done |
| 1 | Server: registry, `GET /sources`, `sourcesChanged`, receivers, video-input layer content, `triggerLiveSource`, clear/restore, thumbnails, tests (loopback IDN sender) | done (`tests/server/live_sources.rs`) |
| 2 | Oriel: bin's Video Inputs tab (names, status, thumbnails), click to put live, the layer button and status on the Show page, looks' Video input row live, stub, vitest, shots | done: `VideoInputsPanel` (thumbnails every 3 s, status, switch, "+" by address), `VideoInputStrip` under the layer buttons (status, Clear, Bring back), `state/sources.svelte.ts`; the looks row needed no change (the server now honours it) |
| 3 | Batch 11: lane V the video side (items 1, 2, 4, 5, 6 below), lane A the sound (item 3) | video side done (lane V, see "Stage 3, batch 11": `tests/server/live_sources.rs` fill, input + alpha, H.264; `output` field pairing; import of live-video fills; editor's "Live source" fill); sound done (lane A, item 3 and §5): done |
| 4 | Batch 12, lane I: the `video_inputs` fix, logical inputs CRUD and Oriel list, the fill picker's inputs, NDI\|HX AAC sound | done: `live_sources::video_input_conditions_follow_the_live_input`, `live_sources::video_inputs_crud_and_bindings`, `live_sound::aac_sound_decodes_into_sound_blocks`; Oriel: the bin tab's Inputs, the fill picker's inputs, stub routes, shots `show-bin-input-menu`, `show-bin-inputs-live`, `edit-live-input-fill` |

Stage 3, precisely:
1. **A source as a slide element's fill** (the board's "Live stream" theme background and a
   camera box on a slide): model `FillKind::LiveSource { source, display }` (additive tag; old
   builds read it as `Unknown` and draw nothing), its bindings; the render crate draws an element
   whose fill is a live source from `LayerSource::Slide.videos` like a video element (its
   display settings come from the new fill kind: a small change in `render::scene`, coordinated
   with lane R, which owns element audio there); the engine's `Side::with_videos` asks the
   `LiveSources` hub for those elements' frames next to the element players; the editor's fill
   picker gets "Live source" with the source list. Tests: a slide with such an element shows the
   loopback camera inside the box.
2. **`triggerVideoInput {input}`**: load `VideoInput` and `InputBinding` box documents into
   `ShowData` (imports already write them), resolve the binding's `InputSource::Idn
   {sourceName}` and go live as above with `input` set in the state; playlist video-input items
   and slide actions then work (today counted as skipped actions).
3. **Sound** (done, batch 11, §5): a lane per live source on the program bus, resampled to the
   bus rate with drift correction, paired with video by timecode in our engine; the layer's
   level / mute (`videoInput`) and `LiveVideoInput.audio` (where `VideoInput.audio` goes).
4. **Alpha** (SHQ7/9): an alpha plane in the renderer's video path (`render::video`, after lane
   R's keyhole work), so keyed graphics from another Vesper composite over the media layer.
5. **Interlaced** sources: weave the two fields with `input::weave_fields`.
6. **H.264 / HEVC cameras** (`unsupported` today): `cfg.codec` with idn-ffmpeg's decoder.

### Stage 3, batch 11 (lane V: everything but sound)

Order: 1 fill, 2 `triggerVideoInput`, 6 H.264/HEVC, 4 alpha, 5 interlaced. Sound (3) is lane A's:
it adds an audio module that the receiver feeds; the video side keeps to its own functions.

1. **Fill.** `FillKind::LiveSource { source, input?, display }` (additive serde tag; older
   builds read `unknown` and draw nothing). `source` is a network source name or `ip:port`;
   `input` (a logical `VideoInput`) wins when set and resolves through this box's
   `InputBinding`, so a show moved to another box follows that box's cameras. The engine keeps
   a second list of element frames on a slide side (live frames next to the element players'
   file frames); the render crate's video-element path takes `FillKind::LiveSource` as it takes
   a video `Media` fill (display from the fill). Content with a live fill is "animated" (the
   screen asks for a render every tick and re-renders only on a new frame). Editor: the fill
   picker's "Live source" with the source list (and the logical inputs once Oriel lists them).
   Import: a slide element whose media fill is live video maps to `LiveSource { input }` by the
   workspace's input index (until now `Unsupported`).
2. **`triggerVideoInput {input}`.** `ShowData` loads `VideoInput` (portable, box) and
   `InputBinding` (machine) documents. The binding's `InputSource::Idn { sourceName }` goes
   live on the video-input layer with `input` and the input's name in the state; the use's
   display (`VideoInputTrigger.display`) and the input's `alpha` apply. A missing input or an
   unbound one / a non-IDN binding (capture devices: not supported; OMT since batch 13, `docs/OMT-PLAN.md`) answers 404 / 409; a
   playlist video-input item and a slide's video-input action use the same path (no longer
   skipped). Re-triggering the same input is a no-op.
3. Sound: lane A (done, see item 3 above and §5).
4. **Alpha.** The receiver keeps the A plane: Y and A are packed as pairs (`Rg8`) next to the
   CbCr plane (`render::VideoPlanes::Nv12Alpha`); `fs_yuv` outputs premultiplied linear RGB
   (straight wire alpha: multiplied after linearising; premultiplied wire alpha: divided, then
   linearised and multiplied again). `MediaDisplay.alpha` (and `VideoInput.alpha`): `opaque`
   ignores it, `straight` / `premultiplied` override what the source says, `auto` follows the
   source (`wire_alpha`).
5. **Interlaced.** Fields (`field_id` 1, 2) are woven with `input::weave_fields` in the receive
   thread: a field 1 is kept, the next field 2 is woven with it into one frame; a pair that
   doesn't belong together (`None`) is skipped. The status shows the woven frame's size and
   the frame rate.
6. **H.264 / HEVC.** Each receiver opens with `cfg.codec = CodecInput(ffmpeg, Allow)`: NDI|HX
   cameras arrive decoded as `Video` (4:2:0 planes), and a 4K IDN source with a GPU encoder
   may send compressed video above 1080p. The FFmpeg adapter is the media crate's (one probe
   per process). A source that still arrives `Compressed` (no decoder for its codec on this
   machine) keeps the `unsupported` status. GPU decoding needs the `render` group (as for files).

Owner questions (answered with the recommendation, proceeding):
- *A fill names a source or a logical input?* Both; the input wins (portable shows).
- *Codec use:* `Allow` (SpeedHQ on a LAN is cheaper to decode; compressed only above 1080p or
  from HX cameras), not `Prefer`.

What could not be verified on the build machine: real cameras (NDI|HX H.264/HEVC from a camera,
an interlaced 1080i source, a keyed SHQ7 source from another product). Covered instead by the
loopback IDN sender (alpha plane, an H.264 stream from FFmpeg's CPU encoder with the output
forcing compressed video) and a unit test of the field pairing (the library's `weave_fields`
itself is trusted). Checklist for the owner's hardware day: an HX camera shows (status `live`,
not `unsupported`); a 1080i camera shows full height without combing; a keyed source as a slide
element's fill shows the media layer through its transparent parts (on the bottom-most
video-input layer it shows the screen's background); `opaque` on the input draws it solid.

Left for later (done in batch 12, below): an Oriel list of logical inputs and their bindings,
and picking a logical input (not only a source) in the editor's fill picker.

### Stage 4, batch 12 (lane I: finishing live sources)

0. **Fix first:** the server never filled `DataContext.video_inputs`, so every "video input X
   is live" visibility condition saw the input as not live (screens and video element sound).
   It now lists the logical input on the video-input layer, or, for a source put live by name
   or address, every input bound to that source here.
1. **Logical inputs and their bindings (server).** `GET /video-inputs` lists
   `VideoInputEntry { input, source?, revision }` (the portable `VideoInput` with this box's
   binding: `source` is the `InputSource`, absent when unbound). `POST /video-inputs`
   (partial: a name, else "Camera N"; optional `source`), `PUT /video-inputs/{id}` (the whole
   input plus `source`; `If-Match`), `DELETE /video-inputs/{id}` (the input and its binding).
   Edits go through the show actor like the screen configuration (planned, persisted
   atomically, swapped in); `documentChanged` for `videoInput` / `inputBinding`. A live input
   follows its binding at once (rebound: the new source; unbound or deleted: off the layer,
   as for a reload). Names are trimmed, up to 64 characters; an IDN source name up to 512.
2. **Oriel: where the board puts it.** The bin's Video Inputs tab gets the logical inputs as
   a row of cards above the network sources ("Inputs": name, bound source and its status,
   LIVE badge; click puts it live with `triggerVideoInput`), with "+" to add one, and a menu
   per card: Rename, Bind to a listed source / an address, Unbind, Delete. A network source
   card gets "Make an input from this source". Screens stay about outputs (the board's
   Screens canvas shows no inputs).
3. **The editor's fill picker** ("Live source") lists the logical inputs first (preferred:
   portable shows follow each box's binding) and then the raw network sources; choosing an
   input sets `FillKind::LiveSource { input, source }` with the bound source as the fallback.
4. **NDI|HX sound (AAC).** `InputEvent::CompressedAudio` (AudioSpecificConfig + one raw
   frame) is decoded with the media crate's FFmpeg adapter (the receiver's `LiveDecoder`,
   idn-ffmpeg's `DecoderFactory::open_aac`, opened again when the config changes) into the
   same `LiveEvent::Sound` blocks as uncompressed sound, with the frame's timecode, so the
   live sound lane (§5) pairs and resamples it unchanged. A frame that fails to decode is
   skipped (logged once). The loopback sender's AAC goes to inputs that decode AAC
   themselves (the library then delivers `Audio`), so the test decodes an AAC stream made
   by the system FFmpeg in the test (skipped where it has no AAC encoder).
5. Stub, vitest, shots (the tab with inputs, the card menu, the fill picker), docs.

Owner questions (answered with the recommendation, proceeding):
- *Where do logical inputs live in Oriel?* In the bin's Video Inputs tab, above the sources
  (where an operator picks cameras; the board puts Video Inputs there). Not in Screens.
- *Deleting an input that playlists or fills use?* Allowed, like a media file: those uses
  then answer "not found" when triggered (the trigger is refused, nothing breaks live).
- *Editing a live input's audio or alpha?* Takes effect the next time it is put live (the
  layer keeps what it was triggered with), as for a look's media display.

Not verifiable here: a real NDI|HX camera's AAC (its timecodes against its pictures, AAC
priming delay of 1024 samples, about 21 ms, not compensated).
