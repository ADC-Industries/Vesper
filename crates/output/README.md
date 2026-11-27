# `output` crate: frame sinks

This crate takes finished frames from the render engine and gets them out of the process: as an IDN Open Protocol source (compatible with NDI®; the server's main network video transport), a PNG file, through an `ffmpeg`/`ffplay` pipe, or as an Open Media Transport (OMT) source. It also receives live sources (IDN and OMT) for the video-input layer. No proprietary runtime is used or loaded.

## Frame contract

The render engine and this crate share only `model` types. The server converts each rendered frame into an `output::Frame`:

```rust
pub struct Frame<'a> {
    pub width: u32,
    pub height: u32,
    pub pixels: &'a [u8],        // RGBA8, sRGB-encoded, PREMULTIPLIED alpha, tightly packed
    pub alpha: model::AlphaMode, // the screen's alpha intent
    pub frame_rate: model::FrameRate,
    pub timecode: i64,           // 100 ns units
}

pub trait FrameSink: Send {
    fn name(&self) -> &str;
    fn send(&mut self, frame: &Frame<'_>) -> anyhow::Result<()>;
}
```

Each sink does its own format conversion, using the routines in `output::convert`.

- **Opaque screens** (`AlphaMode::Opaque` or `Unknown`) are flattened over black. With premultiplied input, that just means dropping the alpha byte.
- **Screens with alpha** (`Straight` or `Premultiplied`) are sent in whatever alpha representation the transport defines. The IDN wire format, PNG and ffmpeg's `rgba` all use straight alpha, so these sinks un-premultiply (for IDN the library does it).

### Conversions

| Format | Used by | 1080p, release, one core* |
|---|---|---|
| premultiplied → BGRX / RGBX (opaque) | ffmpeg (`rgb0`, no conversion at all) | ~0.6 ms |
| premultiplied → premultiplied BGRA | available | ~0.6 ms |
| premultiplied → straight BGRA / RGBA | PNG, ffmpeg with alpha | 1–4.5 ms |
| premultiplied → UYVY 4:2:2, BT.709 limited range | `OmtSink` | ~2.7 ms |
| premultiplied → UYVA (UYVY + alpha plane) | `OmtSink` | 1.8–8.4 ms |

`IdnSink` needs none of these: the IDN library takes premultiplied RGBA and converts on its own thread.

\* Measured on the Ryzen 680M box. A range means the time depends on the content: opaque and fully transparent pixels take fast paths, while partially transparent pixels are the slowest case. To reproduce: `cargo run -p output --release --example convert_bench`.

For UYVY:
- The sRGB-encoded values are used directly as R'G'B', which is the usual practice.
- Chroma is the average of each horizontal pixel pair.
- In UYVA that average is weighted by alpha, so a transparent neighbour doesn't darken the edge of a lower third.

## Sinks

### `PngSink`
- `PngSink::snapshot(path)` writes the next frame it receives, then ignores everything after it.
- `PngSink::every_nth(dir, n)` writes frames 0, n, 2n, … as `frame_000000.png`, and so on.

`send` only copies the frame. Encoding happens on a worker thread. If that thread is still busy, the frame is dropped and `dropped()` counts it.

- Opaque screens are written as RGB.
- Screens with alpha are written as straight RGBA.
- Every file carries an sRGB chunk.

`output::write_png(path, &frame)` does the same thing synchronously.

### `FfmpegSink`
Runs the system `ffmpeg` or `ffplay` and pipes raw video into its stdin. The process starts on the first frame:

```
<program> <input_args> -f rawvideo -pixel_format rgb0|rgba -video_size WxH -framerate N/D -i - <output_args>
```

- `FfmpegSink::preview(title)`: an `ffplay` window. It needs a display.
- `FfmpegSink::record("out.mp4")`: libx264, `yuv420p`, CRF 18.
- `FfmpegSink::new(FfmpegConfig::ffmpeg([...]))`: any other ffmpeg output.

Behaviour:

- **It never stalls the render loop.** `send` copies the frame into a pooled buffer, and a writer thread does the conversion and the pipe writes. When ffmpeg falls behind and the queue (`queue_depth`) is full, new frames are dropped and counted. The test suite checks this: 100 sends at 1080p into an ffmpeg throttled to 10 fps finish in about 20 ms.
- **The format is fixed.** A raw video pipe can't change size, frame rate or alpha mode partway through. If a later frame differs from the first, `send` returns an error, and the fix is to create a new sink.
- **Errors are clear.** If the program is missing, the error says so. If the process dies, the next `send` returns an error that includes the last lines ffmpeg wrote to stderr.
- **Finishing depends on the use.** `finish()` (or drop) writes out the queued frames, closes stdin and waits for ffmpeg, so recordings are finalised properly. Previews (`kill_on_finish`) are killed instead.

### `IdnSink`, `IdnFinder`, `IdnReceiver`
[IDN Open Protocol](https://github.com/ADC-Industries/idn-open-protocol) is a separate, open-source (MIT / Apache-2.0) Rust library implementing a network video protocol **compatible with NDI®**: its sources appear in compatible receivers (vMix, OBS, monitors, hardware switchers) and its receiver takes compatible sources. It is compiled in, so there is nothing to install on the server. It is the server's only network video transport.

> NDI® is a registered trademark of Vizrt NDI AB. This project and the IDN Open Protocol library are not affiliated with or endorsed by Vizrt. We describe the IDN backend only as "compatible with NDI®".

**Building.** The library is developed separately and used as a **git dependency** (`github.com/ADC-Industries/idn-open-protocol`, pinned to a release tag in every crate's `Cargo.toml` and recorded in `Cargo.lock`; a public repository, so cargo fetches it like any other git dependency, no credentials needed). It is never vendored, copied or modified here. Move to a new release (a maintainers' decision) by moving every tag, then `cargo update -p idn-open-protocol`. The workspace builds the library with `opt-level = 3` in dev builds too (`[profile.dev.package.idn-open-protocol]`), because it encodes video in software. `/output-transports` reports the library's `idn_open_protocol::VERSION`.

**What it uses from the library** (its public API only):
- `api::Publisher` (one per group list, shared by all sinks of the process) and `api::Output` (one per sink), fed with `api::VideoFrame::rgba(.., AlphaMode::Premultiplied | Opaque)` plus an optional `api::AudioBlock`.
- `discovery::Browser` for `IdnFinder`, `input::Input` (with `rgba: Some(AlphaMode::Straight)`) for `IdnReceiver`.

**Behaviour:**
- **Pixels and alpha.** Our premultiplied sRGB RGBA goes to the library unchanged; the library un-premultiplies and converts to BT.709 limited-range Y'CbCr with straight alpha. Opaque screens are sent without alpha (the alpha byte is ignored, i.e. over black); screens with alpha get an alpha plane.
- **Never blocks.** `send` copies the frame into a pooled `Arc` buffer (an unchanged frame is resent as the same buffer, so the library skips its encode) and hands it to the library's one-slot mailbox; the library converts and encodes on its own thread, and a frame it could not take in time is replaced by the newer one (`IdnSink::stats().replaced`).
- **Alpha and frame rate are fixed per source** by the library. If frames arrive with a different alpha intent or frame rate than the sink was created with, the sink restarts its source (`restarts()`), which receivers see as a short reconnect.
- **Audio.** The library carries audio with each video frame. `IdnAudioSender` (an `AudioSink`, used by the server's embedded-audio worker) queues the samples; the sink attaches one frame's worth (1600 samples at 48 kHz and 30 fps; 1601/1602 at 29.97) to every frame. Audio therefore needs video to keep flowing (the server re-sends static frames every tick); after 0.5 s without video the oldest audio is dropped (`audio_overflowed()`).
- **Names.** Sources are announced as `HOST (name)`; the server keeps IDN source names unique among its outputs.
- **Metadata.** The library's output API sends none, and neither do our sinks.
- **Cost.** Encoding is in software: about **1.8 CPU cores per 1080p30 stream** today, so plan on about **2 outputs per mini PC** until the library's encoder improvements land. `idn_soak` measures it.

**Verification receiver.** `IdnFinder::new()` browses (mDNS) and lists every compatible source (`IdnSourceInfo`: full name, address, groups, whether it announced itself as IDN); `find_by_name` matches `"MACHINE (SOURCE)"` or just `SOURCE`. `IdnReceiver::connect(name or ip:port)` receives straight-alpha RGBA video (`RGBX` without an alpha plane) and planar float audio as `recv::Received` values. It is not the input layer of later milestones.

### Tools
- `cargo run --release -p output --example idn_probe -- --list` lists sources; `idn_probe "<source>" --frames 90 --png out.png` receives one and prints size, FourCC, measured fps and the alpha range, and saves the last frame. `<source>` can also be `ip:port`.
- `idn_pacing "<source>" --seconds 30 [--src-fps 25] [--csv out.csv]`: frame pacing end to end (arrival jitter; repeats and skips from a 12-bit frame counter in the picture).
- `idn_av_probe "<source>" 30`: A/V offset of beeps against white flash frames, by timecode, and the audio level.
- `idn_soak [--seconds 300] [--alpha] [--audio]`: one 1080p30 IDN output with a motion pattern; prints CPU, memory and the library's counters every 10 s. Receive it meanwhile with `idn_pacing` to check for stalls.
- `sink_demo idn "<name>" [--alpha]`: a moving test pattern for 5 s, to look at on a receiver.

Checks against third-party receivers and senders are manual: see [`docs/INTEROP-CHECKLIST.md`](../../docs/INTEROP-CHECKLIST.md).

### OMT: `OmtSink`, `OmtAudioSender`, `OmtInput`
[Open Media Transport](https://openmediatransport.org) is an MIT-licensed network video protocol that vMix and OBS support. It comes from IDN's `idn-omt` crate (OMT written in Rust, the VMX codec compiled in from the vendored libvmx; nothing to install or bundle, a C/C++ compiler at build time).
- `OmtSink`: one source per sink, `"HOST (name)"` on mDNS (`_omt._tcp`), a TCP port from 6400. Opaque screens go out as UYVY, screens with alpha as UYVA, BT.709 limited range, marked BT.709 at every size (`Sender::set_color_space`) (straight colour for a `Straight` screen; for a `Premultiplied` one the colour as rendered, the source marked premultiplied), converted and coded with VMX on the caller's thread (the server's per-output worker). Without a receiver nothing is converted. An odd width loses its last column.
- `OmtAudioSender`: program audio (planar float) on the sink's source, stamped with the program clock like the video.
- `OmtInput`: receives a source with the IDN input's `LiveEvent`s: pictures as NV12 plus the alpha plane (straight, or premultiplied as the sender marks it), in the matrix the frame header gives (BT.601 / BT.709; unmarked: BT.709 from 720 lines, BT.601 below), limited range; sound as planar float. A thumbnail proxy asks for previews (an eighth of each side); a source without them sends whole pictures, used as they come. `omt::sources` is a one-shot mDNS lookup.
- Live-source keys: `omt:<name or host:port>` (or `omt://host:port`) names an OMT source where Vesper takes one string for a source (`omt::source_of`, `omt::key`).

## Tests

```
cargo test -p output
cargo clippy -p output --all-targets -- -D warnings
```

All tests use synthetic frames. The ffmpeg tests run the system `ffmpeg` and are skipped if it isn't installed.

**Loopback** (`tests/idn_loopback.rs`): an `IdnSink` sends 1080p30 synthetic frames and an `IdnFinder` / `IdnReceiver` in the same process receives them: discovery time, size and rate, 75 % colour bars, a straight-alpha checker on a key source, pacing (arrival jitter, repeats and skips from a frame counter in the picture), a 1 kHz tone (frequency, level, continuity) and the A/V offset of beeps against flash frames. It runs with the normal tests; for the printed measurement: `cargo test --release -p output --test idn_loopback -- --nocapture` (`IDN_LOOPBACK_SECS` sets the pacing run, default 10 s).

## Examples
- `cargo run -p output --release --example convert_bench` times every conversion at 1080p.
- `idn_probe`, `idn_pacing`, `idn_av_probe`, `idn_soak`, `sink_demo`: see Tools above.
