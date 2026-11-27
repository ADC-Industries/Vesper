# OMT (Open Media Transport) outputs and inputs

Batch 13, lane M. OMT is an open (MIT), royalty-free network video protocol (vMix, OBS). Vesper
uses IDN's `idn-omt` crate (sources and receivers written in Rust, the VMX codec
compiled in from the vendored libvmx): no runtime library to install, nothing to bundle. The
crate's public API is all this plan relies on: `Sender` / `SenderConfig` (a source: a TCP
listener from port 6400, announced on mDNS as `_omt._tcp` "HOST (NAME)", `send_video` of UYVY
with an alpha plane after it, `send_audio` of planar float), `Receiver` (`start(source,
audio)`, `recv` → `Connected` / `Disconnected(why)` / `Video(Picture)` / `Audio{…}` /
`Metadata`, reconnects by itself, finding the source again by name), `receiver::sources(timeout)`
(a one-shot mDNS lookup, sorted full names) and `resolve` ("HOST (NAME)", `omt://host:port`,
`host:port`).

## Model

Nothing new: both halves already exist in the model.
- **Outputs:** `OutputTransport::Omt { name }` (an output binding like an IDN one, `embedAudio`,
  delay, overrides, regions, tiles, Fill/Key all apply).
- **Inputs:** `InputSource::Omt { name }` binds a logical video input to an OMT source (`name`:
  "HOST (NAME)" or an address), next to `InputSource::Idn { sourceName }`.

## The source key: `omt:` + the name

Every place that names a live source by a string (the video-input layer's `triggerLiveSource
{source}`, a `liveSource` element fill's `source`, `GET /sources/thumbnail?source=`, the
registry, the sound feeds) takes an OMT source as **`omt:` followed by what `resolve` takes**
(`omt:STUDIO (Program)`, `omt:192.0.2.10:6400`); `omt://host:port` is accepted as it is. Names
without the prefix stay IDN sources, so nothing stored changes meaning. A logical input bound to
`InputSource::Omt { name }` resolves to `omt:<name>`. `LiveSource` gets `omt: true` for these
entries (the list is keyed by that same string).

## Server

- **Output** (`output::omt::OmtSink`, replacing the placeholder): one `Sender` per binding,
  named as the binding says. The sink runs on the output's worker thread (one slot, frames
  dropped when busy, as for every output); with no receiver connected it converts nothing. A
  frame is converted with the shared `convert` routines: opaque → UYVY, alpha → UYVY plus an
  alpha plane (UYVA), BT.709 limited range; an odd width loses its last column (VMX needs
  pairs). A screen whose alpha type is `straight` goes out with straight colour; one that is
  `premultiplied` goes out with its colour premultiplied (as rendered) and the source marked so
  (`Sender::set_premultiplied`, IDN v0.3.3; the flag is the sender's, so it follows the
  frames). The frame's timecode (program clock, 100 ns) is the OMT timestamp; embedded
  program audio (`OmtAudioSender`) goes out with `send_audio` on the same timeline, like IDN.
  Fill/Key halves are opaque. `GET /output-transports` reports OMT available with the crate
  version.
- **Input** (`output::omt::OmtInput`): a `Receiver` per source in use, with the IDN input's
  `LiveEvent`s, so the registry, the newest-frame mailbox, hold-on-lost, alpha, sound and
  thumbnails work unchanged. Pictures (UYVY, optional alpha plane) are packed as NV12 with the
  alpha plane kept; the matrix is the one the frame header gives (`Picture.color_space`, 601 or
  709; a sender that says nothing is resolved by the library as VMX codes it: BT.709 from 720
  lines, BT.601 below), always limited range; the alpha is straight, or premultiplied when the
  sender marks it so (`Picture.premultiplied`; drawn without premultiplying again). Sound arrives as planar float and feeds the live
  sound lane, paired with pictures by timestamp.
- **Discovery:** the registry (started on first use) also runs an OMT lookup thread:
  `sources(1.5 s)` every 5 s, merged into the list as `omt:` entries (a source missing from two
  lookups in a row drops off). OMT is mDNS only: a site's discovery server lists IDN sources.
- **Thumbnails:** from the receiver in use, else a proxy receiver without sound that keeps one
  picture in 15 and closes after 10 s without a request. The proxy asks for previews
  (`Receiver::set_preview(true)`, IDN v0.3.3): an eighth of each side (1920×1080 → 240×135),
  so it costs a fraction of the bandwidth and decoding. A source without previews goes on
  sending whole pictures (`Picture.preview` false): they are used as they come (thumbnails are
  scaled anyway) and the server logs it once.

## Oriel

- **Screens ▸ output picker:** OMT is selectable once the server says it is available (it was
  "unavailable here"); "Embed program audio" applies to OMT outputs too.
- **Show ▸ bin ▸ Video Inputs:** OMT sources are listed with the IDN ones, marked "OMT"; picking
  one for a logical input stores `{type: "omt", name}`; the manual entry takes `omt:host:port`.
  The fill picker lists them too (same source list).

## Deferred

- Tally both ways (the sender knows when a receiver has it on program; a receiver can tell a
  source): later, with the IDN tally work.
- Quality profile per output (Low / Standard / High; Standard now), metadata.
- OMT through the site's discovery server.

## Owner questions (recommendations taken)

1. *Should OMT outputs be a separate transport or a flag on IDN outputs?* Separate (the model
   already has it): different receivers, different names, can run side by side.
2. *Thumbnails of OMT sources cost a full decode.* Accepted at first (only while the bin shows
   them, closed after 10 s); asked the IDN maintainers for a preview option on the receiver. **Done**
   in IDN v0.3.3: proxies ask for previews (above). The colour matrix and the premultiplied
   flag asked about at the same time are **done** too (receive and send, above). Since IDN
   v0.3.4-Beta10 every output is also marked BT.709 (`Sender::set_color_space(Some(709))`), the
   matrix it is converted with at every size; before, VMX marked pictures under 720 lines
   BT.601.
3. *Licensing:* `idn-omt` is MIT / Apache-2.0 with libvmx (MIT) compiled in; nothing to bundle
   on Windows, a C/C++ compiler at build time (the `cc` crate).

## Stages

- [x] 0. This plan.
- [x] 1. `output::omt`: sink, audio, input, discovery, loopback tests.
- [x] 2. Server: outputs, the `omt:` source key, discovery, codex binding; tests; protocol docs.
- [x] 3. Oriel: output picker, Video Inputs, stub, shots.
- [x] 4. Docs: products, G table, packaging.
- [x] 5. IDN v0.3.3: the received matrix from the frame header, premultiplied alpha both ways,
  previews for thumbnail proxies; loopback tests (premultiplied flag both ways, preview size).
