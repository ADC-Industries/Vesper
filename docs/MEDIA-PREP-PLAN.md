# Media preparation and CCLI usage reports

Batch 16, lane M (2026-11-26). Owner decision: media preparation is **automatic on upload, with
a setting to turn it off**. Lane C after Beta15 (2026-11-26): conversions in a child process of
their own, may run during a service at a lower priority, and the settings below (owner
decisions).

## Media preparation

**Where it runs.** On the box that holds the Codex's media library (the Codex box, or a
standalone Vesper), as part of the media worker (`vesper-codex::prep`). After an upload or a
relink is probed, a video or sound asset whose file wasn't prepared yet is queued (automatic:
the setting `enabled` and the server's `CodexOptions::media_prep`, which `vesper` turns on and
`--no-media-prep` turns off). Oriel's **Prepare** queues one by hand (`again`: from the original).
Each file is prepared once (`prep.analysed` = the SHA-256 prepared). A file that failed is tried
again only by hand.

**A job.**
1. **Analysis** (lowest CPU priority, decoding only): IDN's probe (since Beta14 with the
   average and base frame rates, IDN's variable-rate guess, the field order and the bit rates);
   **loudness** (EBU R128 /
   BS.1770-4 integrated, through IDN's `AudioReader` at 48 kHz stereo; `media::loudness`, unit
   tested against EBU Tech 3341's −23 dBFS tone) and the **gain** to the house target (default
   −16 LUFS, kept within −20…+12 dB and so the peak stays under −1 dBFS); the **poster frame**
   (the first of the frames at 10, 25, 50, 75 and 3 % that isn't black or flat; the stored
   thumbnail of every video uses it now). Saved at once, so a waiting conversion doesn't hide
   the loudness.
2. **Conversion** when the video is hard to play live (`prep::plan`, unit tested): a codec
   other than H.264/HEVC, a codec this box's GPU doesn't decode, 4:2:2/4:4:4 colour, 10-bit
   H.264 or 12-bit anything, interlaced (as the file states; IDN's converter also deinterlaces
   pictures flagged interlaced), variable frame rate (IDN says so, or the average and base rates
   differ by > 0.5 %), above 60 fps, larger than 4K, a bit rate far above its size (~50 Mb/s
   for 1080p30, scaled), sound in an unusual codec, or a video's sound not at 48 kHz. A video is
   always converted whole (IDN's converter encodes the picture again: it has no picture copy),
   also when only its sound needs it. Kept as it is: alpha video (the house format has none),
   with a note, and its sound with it.
3. The copy goes into the media library next to the original (`<name> (prepared).mp4`); the
   asset plays it (`location`) and keeps the original (`prep.original`: location, facts, the
   reasons, the copy's format). The original stays in the library and is marked in use (its
   location is in the document). "Use the original" swaps back and is not converted again by
   itself. Uploading the original's bytes again finds the asset (left as it is: it keeps playing
   its copy). A show box plays the original while the copy is still copying to it (the copy can
   be swapped in during a service when the Codex box itself is idle).

**The house format** (owner choice taken: H.264, HEVC as a setting): **H.264, 8-bit 4:2:0,
constant frame rate (the usual rate nearest the source's average, at most 60), 12 Mb/s for
1080p30 scaled by size and rate (HEVC 0.6 of that), AAC 48 kHz stereo 192 kb/s, MP4.** Why: every GPU
decoder Vesper meets takes H.264 (VA-API on the AMD/Intel church boxes: this machine decodes
H.264 CB/Main/High, HEVC Main/Main10, VP9, AV1; D3D11VA/NVDEC on Windows), IDN encodes it
on the GPU's encoder (VA-API, NVENC, Quick Sync, AMF) or on the CPU (Ubuntu's libx264, OpenH264
in the LGPL Windows build), and it
decodes cheaply in software where a GPU refuses. HEVC halves the size but not every box decodes
it and the LGPL Windows build has no software HEVC encoder. Sound at 48 kHz is the bus rate (no
resampling at play time).

**The converter** (lane I, IDN Beta14; in a process of its own since lane C). IDN's
`media::convert` (through Vesper's `media::convert`) on the machine's FFmpeg libraries (Ubuntu's
on Linux, the LGPL DLLs next to `vesper.exe` on Windows); no `ffmpeg` / `ffprobe` tool is needed
or run any more. Each conversion runs in a **child process** of the server's own binary: `vesper
media-convert --config <job>.json` (hidden; `vesper_codex::prep::child`), which writes its
progress as JSON lines on stdout and its result (or failure) as the last line. The server starts
it from a thread at its own priority (the job threads run at the lowest, which Linux children
inherit and can't raise again), reads the lines, and **cancels by killing it** (by its handle:
the server stops, or the conversion must wait while live), then removes the partial output
(`<name>.part`). A child that ends without its last line (a **crash** inside FFmpeg, killed by
the system) fails the job with the note "the converter crashed (signal 11 (SIGSEGV)); the server
and the show weren't affected", and its last stderr line; the asset keeps its analysis and the
failure shows on it (Prepare again by hand). The child stops by itself when its stdin closes (the
server holds it open), so a server that went away leaves no conversion behind. On Linux the
server starts `/proc/self/exe` (the same program even after a package update replaced the file);
on Windows `vesper.exe` itself, without a console window. Embedders and most tests without the
binary run the conversion on a thread in their own process (`CodexOptions::converter`). It decodes in software, deinterlaces what is interlaced, turns the picture
upright, fits it inside 3840×2160 when larger, puts it on the constant rate Vesper asks for, and
encodes on the GPU's encoder where the machine has one for the codec (VA-API, NVENC, Quick Sync,
AMF), else on the CPU (H.264: OpenH264, or on Linux the system FFmpeg's libx264; HEVC:
Kvazaar, which Ubuntu's FFmpeg doesn't have: there HEVC needs a GPU encoder; accepted, owner
2026-11-26), with AAC 48 kHz stereo. It writes `<name>.part` and renames it once complete. What IDN couldn't keep (an HDR picture made 8-bit) goes
into the asset's notes. Settings ▸ Media shows "IDN media conversion" with the FFmpeg version
and the encoder (`VA-API`, `OpenH264 (CPU)`); without an encoder for the house codec media are
analysed and the asset says why it wasn't converted. IDN's documentation doesn't say whether
the copy has its index first and a keyframe every 2 s (what the `ffmpeg` tool's arguments asked
before, and what live playback wants: a quick start, clean seeks and loops); checked on Beta16
with a synthetic clip whose source has its index last and keyframes 10 s apart: `moov` before
`mdat`, keyframes every 2.0 s, with libx264 and VA-API (`crates/media/tests/convert.rs`, a
regression test; the Windows encoders and NVENC/Quick Sync/AMF aren't checked here). Should it
change, the requirement goes to the IDN maintainers; the file is never fixed up afterwards.

**Priorities.** The child runs at the job's priority from its start: **Low** (the default):
Linux nice 19 and the idle I/O class (set by the child on its first thread before any other
starts, so IDN's decoder and encoder threads have them too), Windows `BELOW_NORMAL_PRIORITY_CLASS`
(its creation flags), and IDN's `low_priority`; **Normal**: as the server (nice 0 / normal class,
IDN's threads left as they are), for a box that has nothing else to do; **the lowest** while
something is live: Linux as Low, Windows `IDLE_PRIORITY_CLASS`. A conversion running when
something goes live is **lowered** to the lowest (every thread of the child on Linux, its class
on Windows) and stays there until it ends (Linux doesn't let a process raise a priority again
without privileges). Analysis (probe, loudness, poster) stays in the server's process on the job
thread at nice 19 / the lowest thread priority: it decodes for seconds with the same decoders
playback uses in-process, so a child process would add little safety for its cost.

**While live** (owner decision 2026-11-26, changed from batch 16's "wait"): media imported late
must still get ready during a service without stopping the show, so by default
(`whileLive: lowPriority`) a conversion **goes on at the lowest priority** while something is
live on the box (`live_items`: a slide, media, audio, props, messages): one running is lowered,
one starting starts there, and with the encoder on Auto one starting while live takes the CPU's
encoder where there is one for the codec (a GPU's encoder has no priority to give live work; on
Linux HEVC has no CPU encoder, so there it stays on the GPU). The option **"wait until nothing
is live"** (`wait`) stops a running conversion (killed, its partial output removed) and starts it
again once nothing is live; analysis still runs (seconds, decoding only, lowest priority; a
waiting conversion takes no job slot). Changing the setting to `lowPriority` lets a waiting
conversion start at once. Jobs not finished are kept in `codex/media-prep-queue.json` and picked
up again after a restart.

**Settings** (`codex/media-prep.json` on the Codex box; Oriel's Settings ▸ Media). Settings
saved before load with the new fields' defaults (a `whileLive` saved before is kept).

| Group | Setting | Default | What it does |
|---|---|---|---|
| When | `enabled` | on | **Automatically** on upload (analysis and conversion), or **only when asked** (Prepare on a media item) |
| | `transcode` | on | Convert videos hard to play live (off: analyse only) |
| Priority | `priority` | `low` | `low` / `normal`, while nothing is live (above) |
| | `whileLive` | `lowPriority` | `lowPriority` (go on at the lowest) / `wait` (until nothing is live) |
| Quality | `quality` | `standard` | IDN's bit rate by size and rate (1080p30 H.264 7.5 Mb/s) × 1.6 (`standard`, 12 Mb/s), × 2.67 (`high`, 20), × 4 (`maximum`, 30); 2–60 Mb/s; HEVC 0.6 of each (`ConvertConfig::video_kbps`) |
| | `videoKbps` | none | An explicit bit rate for every conversion, 1–60 Mb/s, instead of the preset |
| | `video` | `h264` | The codec, `h264` / `hevc` (`ConvertConfig::codec`) |
| | `maxSize` | `keep` | `keep` (larger than 4K: fitted inside 3840×2160), `1080p`, `2160p` (`ConvertConfig::max_size`; a portrait video's box turned). A video larger than the limit is a reason to convert it |
| Encoder | `encoder` | `auto` | `auto` (the GPU's where there is one for the codec, the CPU's while live), `gpu`, `cpu` (`ConvertConfig::gpu`; none here for the choice: analysed, not converted, with a note). The status names what this box has (`gpuEncoder`, `cpuEncoder`) |
| Sound and jobs | `loudnessTarget` | −16 | LUFS, −30…−5 |
| | `maxJobs` | 1 | Jobs at once, 1–4 |

Everything the owner asked for maps onto IDN's `ConvertConfig` (`codec`, `max_size`,
`video_kbps`, `gpu`, `low_priority`) plus the child's own process priority; nothing went to the
IDN maintainers.

**Playback.** The asset's trim is the default range of every use (a use with its own in/out
point keeps it); the gain is added to the use's level when a video or sound starts (media
layer, foreground video, audio layer). A use that sets its own in or out point keeps its whole
range. Video elements on slides (every player: other screens and signage too) take the trim, not the gain
(leftover).

**Oriel.** Library ▸ a video or sound: a Preparation card (state and progress, loudness, gain,
poster time, why it was converted and the original's format, notes or the error, Prepare /
Prepare again / Use the original) and a trim bar (in and out on the clip's timeline, with the
times; Clear). Badges on media tiles in the Library while a job runs or failed. Settings ▸
Media: the settings in four groups (when, priority, quality, encoder; then sound and jobs) with
the quality presets' bit rates and the encoders this box has next to the choice, the converter
and the encoder in use now, live state and the jobs.

## CCLI usage reports

Each time a song (a presentation with song metadata: CCLI number, title or authors) goes live
on this box, one use per **service** is logged (`reports/ccli-usage.jsonl` in the data folder):
the service is the playlist it was shown from (by name), else the day; the same song again in
the same service and day isn't counted again. A use records date, time, service, presentation,
title (the song's, else the presentation's), CCLI number, authors and whether it was
**projected** (audience screens on), **streamed** or **recorded** (a stream / recording running
when it went live or later while it stayed live). `GET /reports/ccli` counts uses per song in a
date range; `…/ccli.csv` exports them in the columns CCLI's reporting asks for. Oriel: Reports ▸
CCLI Usage (in the Oriel menu; the Settings page's Reports section): a date range, the songs and
counts, songs without a CCLI number flagged, Export CSV, and a note that reporting is done on
CCLI's site.

**Not done.** A site-wide report across boxes (each box logs its own; the Codex box could
collect them later); SongSelect / CCLI API submission (CCLI's reporting is manual for most
licences).
