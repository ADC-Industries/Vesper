# Service-day soak (`vesper-soak`)

A church runs Vesper for hours at a time: a rehearsal, two services, a lobby loop in between. The
soak harness runs a release-built server through a **simulated Sunday on a loop** for as long as
asked and checks that nothing creeps: memory, open files, threads, late frames, A/V sync,
recording, WebSocket round trips. It is a tool, not a test of the server's test binary (an hour
would not fit there): `crates/vesper-soak`, binary `vesper-soak`.

## Running it

```sh
export PATH=$HOME/.cargo/bin:$PATH CARGO_INCREMENTAL=0
cargo build --release -p vesper-server -p vesper-soak
sg render -c "target/release/vesper-soak --duration 1h"
```

Only the build takes the shared build lock; the run doesn't. Options (`--help` for all):

| option | default | |
|---|---|---|
| `--duration` | `10m` | `90s`, `20m`, `1h`, `3.5h` |
| `--outputs` | `1` | audience screens, each with an IDN output (the first embeds the program audio and is probed) |
| `--size` | `1280x720` | every screen, 30 fps (plus one stage screen) |
| `--server` | `$CARGO_TARGET_DIR/release/vesper` | the server binary |
| `--out` | `soak-runs/soak-<time>` | the run's folder |
| `--listen` | `127.0.0.1:5655` | the server's API (its Site Manager takes the next three ports: discovery, API, IDN Voice) |
| `--audio-device` | `""` | `""` = the null output (keeps the audio clock); a card's name plays on it |
| `--scripture-dir` | `target/scripture` if present | Bible translations; without one a synthetic reading is used |
| `--captions-model` | none | a `ggml-<name>.bin` whisper model: captions run on the program audio; skipped without |
| `--nice` | `10` | the server's (and harness's) niceness |
| `--keep-data` | off | keep the data folder and recordings afterwards |
| `--max-…` | see below | the pass/fail limits |

The server runs with its own data folder in the run's folder, `--no-mdns` (it isn't announced on
the LAN), `--no-displays` (it never takes a monitor), the given audio device, and its log in
`server.log`. The run's folder keeps `soak.csv`, `av.csv`, `summary.txt` (and
`summary-so-far.txt`, see below) and `server.log`; the
data folder and recordings are deleted at the end unless `--keep-data` (recording parts are
deleted as each new one starts, so an overnight run doesn't fill the shared disk). Exit status: 0
pass, 1 fail, 2 the harness couldn't run.

## Stopping early, and when there is enough data

**Ctrl-C** (SIGINT) or a SIGTERM to `vesper-soak` stops the run cleanly at any time: the Sunday
being played ends within a tenth of a second, the watcher takes a last sample, the recording
stops, `summary.txt` and `av.csv` are written as at the planned end, and then the harness stops
the server. The server runs in a process group of its own, so the terminal's Ctrl-C reaches only
the harness and the server keeps answering until the summary is in. The summary's first line says
`STOPPED EARLY BY OPERATOR after N of M planned min (SIGINT to the harness)` and the result line
ends `(stopped early by operator)`; "server ran to the end" counts the samples up to the stop,
and operator connections the server closes while it shuts down are not counted as lost. A
server stopped cleanly from outside (exit status 0: only a shutdown signal does that; a crash or
panic never exits 0) is treated the same way. A **second** Ctrl-C stops at once: the server is
killed and no summary is written.

While it runs, `summary-so-far.txt` is written every 10 minutes: the same checks on the samples
so far (`SO FAR, N of M planned min`, `RESULT SO FAR`), plus an **Enough data** line, so the
operator can decide whether to stop early. There is enough data when:

- **every phase of the Sunday has run at least twice** (pre-service, camera, worship, Scripture,
  video, sermon, closing: about 9 minutes; a camera drop and return needs two Sundays as well);
- **the steady numbers are flat**: at least 20 minutes after the warm-up, open files and threads
  back to where they were after it, and late/dropped frames a minute about the same in the
  steady part's first and second halves (the line shows both);
- **the memory slope has settled**: the least-squares slope of the steady part and of its second
  half within 10 MB/h (or a quarter) of each other. A slope still rising, or one far from its
  second half's, needs more time; one under the limit in both halves is a pass.

In practice a 30-minute run gives the late-frame and A/V numbers; the memory verdict needs a
settled slope, which for a run with captions and recording takes about an hour.

## The synthetic library

Built by the harness in the layout `pro-import` writes (the server moves it into its Codex on
first start). Nothing comes from `data/import` or any church's files.

- Screens: `--outputs` audience screens with an IDN output each (`Soak <pid> <n>`), one stage
  screen. Show settings: dissolves (slides 0.5 s, media 1 s), so transitions render every tick.
- Six songs (five sections, nine slides, lorem-ipsum lyrics; a background image on every other
  song's first slide), announcements, a reading, sermon points; the "Soak Sunday" playlist.
- Media (system FFmpeg, generated): three gradient images, a 20 s motion loop (no sound), and the
  **sync clip**: 60 s of black with a white full-frame flash and a 1 kHz beep for 0.1 s every
  2 s, H.264 + AAC. A media bin loops the motion loop.
- The **camera**: a 640×360 IDN source the harness sends itself (a moving bar, a quiet 440 Hz
  tone at −40 dBFS).

## A simulated Sunday (about 4½ minutes, repeated)

Operator A (a WebSocket client) plays it; operator B works beside it the whole time.

1. **Pre-service:** the motion loop, announcements advancing every 6 s.
2. **Camera:** the camera on the video-input layer. Every other Sunday it is **unplugged while
   live** (the harness withdraws the source) and plugged in again 10 s later; the harness times
   how long until the server shows it `live` again.
3. **Worship:** three songs from the playlist, a background image each, `next` every 3 s.
4. **Operator A's network drops** for 3 s (its WebSocket closes) and it connects again.
5. **Scripture:** Psalm 23 from the server's first translation (made through
   `POST /scripture/presentations`), else the synthetic reading; `next` every 5 s.
6. **Video with sound:** the sync clip on the media layer, slides cleared (the A/V probe measures
   this part).
7. **Sermon:** points every 6 s, a flashing stage message.
8. **Closing:** the motion loop, a last song, Clear All.

Throughout: **recording** of the first screen (H.264, 2 Mbps, 10-minute parts) when the box has
an encoder; **captions** when a model was given; **operator B** every 3 s edits a random song
slide's text (`setSlideText`, the live one too), warms thumbnails, fetches a thumbnail and the
playlist, and **drops its connection** for 5 s every 90 s.

## What it watches

Every 5 s (`--sample-secs`) a row of `soak.csv`:

- **The server** (`GET /api/v1/server/stats`, added for this): resident memory, threads, open
  files (`/proc/self` on Linux), WebSocket clients, and per screen clock ticks, **late ticks**
  (frames the clock skipped), **late renders** (video/transition frames finished after they were
  due) and frames the outputs dropped; the live videos' decoder counters; audio underruns;
  the heap in use and held free by the allocator (glibc's `mallinfo2`: resident memory
  growing with the free part growing is fragmentation, with the in-use part growing a leak;
  with the server on mimalloc these count only the C libraries, and `rust_heap_mb` is what
  mimalloc has committed for Rust)
  and the size of every cache (`caches.csv`: the renderer's decoded images, glyphs, slide
  layers, uploaded images and video textures, the thumbnails), which the summary's memory
  line adds up.
- **Recording:** state, bytes (finished parts added), dropped pictures.
- **The camera** as the server lists it (`GET /api/v1/sources`): `live`, `lost`, …
- **The machine's load** (`/proc/loadavg`): other work on the box makes late frames and A/V
  outliers that aren't the server's; the summary says so when the load was high.
- **Operators:** heartbeat (`ping`/`pong`) and command round trips, refusals.
- **The output, end to end:** the probe receives the first audience output like any receiver on
  the network: frames, gaps in its timecodes, and while the sync clip plays every beep paired
  with its flash (audio − video timecode; one row per play in `av.csv`).

## Pass/fail (`summary.txt`)

| check | default limit |
|---|---|
| the server answers to the end | it never exits |
| memory growth after a warm-up (a quarter of the run, at most 15 min: caches fill over the first Sundays), least-squares slope | ≤ 40 MB/h (`--max-rss-mb-per-hour`) |
| open files / threads, end vs after the warm-up | ≤ +20 / ≤ +10 |
| late ticks + late renders + dropped frames, worst minute, every screen | ≤ 30 (`--max-late-per-min`) |
| A/V offset at the output, every paired beep | ≤ ±45 ms (`--max-av-ms`) |
| A/V drift: median of the plays' mean offsets, last third against first third (a play's mean moves within a frame by where the clip's frames fall on the ticks) | ≤ ±20 ms (`--max-av-drift-ms`) |
| WebSocket heartbeat, a sample's median | ≤ 250 ms (`--max-ws-ms`) |
| commands refused or unanswered | ≤ 0.5 % |
| operator connections closed by the server | none |
| recording: growth stalls while live; state at the end | ≤ 30 s; `live` |
| camera back to `live` after it returns | ≤ 20 s, every time |

The limits are a first calibration against this machine (Ryzen iGPU class); tighten them once a
few runs are in.

## Runs

- **1 h, 2026-11-25, batch 15** (`--outputs 2`, captions `base.en`, Psalm 23 from a bundled
  translation, recording; 13 Sundays). The box was shared: other lanes' builds and GPU test runs
  pushed the load average to 18 on 16 CPUs for about 20 minutes in the middle.
  - **Clean:** the server answered throughout; open files 107 → 111 and threads 118 → 121 (they
    rise by about 20 while a video plays and fall back after); WebSocket heartbeat median
    0.2 ms (slowest 73 ms), 1656 commands without a refusal, 50 operator drops and reconnects
    without a connection lost; the camera back `live` 0.8 s after each of 7 returns; the
    recording grew without a stall (875 MB, 10-minute parts); no audio underruns.
  - **A/V:** no drift. While the box was quiet every sync clip play was steady to the
    millisecond (e.g. all 29 beeps at −1.4 ms, or all at +14…15 ms: where the clip's frames fall
    on the output's ticks); while it was loaded, late renders put flashes up to 287 ms after
    their beeps.
  - **Late frames: the video with sound.** In the quiet part, the sync clip (H.264 + AAC,
    720p30, VA-API) gave about 100 late renders a minute over the two audience screens (about
    3 % of their frames) and the next minute about 25, while the motion loop (no sound), songs,
    Scripture and the camera gave about one a minute. Not explained yet; the next thing to look
    at (a run with `RUST_LOG=vesper_server::engine=debug` for the render and decoder
    summaries).
  - **Memory:** 520 MB after the first minute, about 800 MB after 15 minutes, 846 MB at the end
    (peak 883 MB). The slope falls as the run goes on (70 MB/h after 15 minutes, 37 MB/h after
    30): caches filling, or a slow leak; the overnight run decides.
- **Overnight, 3–4 h** (owner approved; after the batch 16 bug run):

  ```sh
  cd <checkout> && export PATH=$HOME/.cargo/bin:$PATH CARGO_INCREMENTAL=0 && \
  cargo build --release -p vesper-server -p vesper-soak && \
  sg render -c "RUST_LOG=info,vesper_server::engine=debug target/release/vesper-soak \
    --duration 3.5h --outputs 2 --out soak-runs/overnight \
    --captions-model $HOME/.local/state/idn-open-protocol/models/ggml-base.en.bin"
  ```

  (`--captions-model` is optional: any `ggml-<name>.bin`; the engine's debug summaries every
  10 s go to `server.log` for the late-frame question above. Since IDN Beta18 whisper runs in a
  child process, `vesper --idn-captions-worker low`: its memory and CPU are not in the server's
  numbers the harness reads; `ps --ppid <server pid>` shows it.) Run it when no other lane builds
  or tests, or the frame numbers say more about the box than about Vesper.

  It holds the GPU (one Vulkan device in the server: three screens rendering, VA-API decode and
  the recording's encoder) for the whole run, 3½ hours plus about a minute to start and stop;
  other GPU tests (`sg render` test runs) should wait until it ends or will run slower and may
  flake. CPU: the server took 3–4 cores in the 1 h run (two 720p IDN outputs encoded in
  software, captions on the CPU, the recording), the harness about 0.15 (its camera and the
  probe's decode). Disk: about 900 MB an hour of recording at a time, deleted part by part.

- **Lane P, 2026-11-26: late frames and memory** (IDN Beta15; the overnight run above was on
  Beta13 and was stopped from outside after 184 min with a SIGTERM to the server, which the
  harness then counted as "the server exited" and as two lost operator connections: both are
  now an operator stop).
  - **Late frames were captions.** The overnight run's 723 late or dropped frames came almost
    all during the video with sound (17 a minute in that phase, ~0 in every other; sermon
    1.7, just after it). The sync clip's beeps start whisper passes: `top -H` showed 15
    `captions` threads at about 85 % each and the load at 8–10 on 16 CPUs while it played.
    Captions then ran on half the CPUs at the lowest priority; since IDN Beta18 whisper runs in
    IDN's worker process below normal priority instead (`docs/CAPTIONS-PLAN.md`, H05-1).
  - **Memory: the allocator, not a cache.** Resident memory rose in steps when a video opened
    (+35…165 MB: decoder queues, whisper's buffers) and fell only partly when it closed; the
    rise slowed over the night (25 MB/h after two hours), and the mappings that grew were
    anonymous heap, not GPU or files. The server now reports the heap in use and held free
    and every cache's size, and trims the heap once a minute (`heap.rs`).
  - **The harness ran the server at nice 19**, not 10 (`nice -n 10` on top of the reniced
    harness): every run before this one. Fixed; the numbers below are at nice 10.
  - **Before / after**, 20 min each, back to back under the build lock (nothing else heavy
    on the box), same harness and settings (`--outputs 2`, captions `base.en`, recording,
    server at nice 10); before = the Beta15 server without these changes:

    | | before | after |
    |---|---|---|
    | late + dropped frames | 73 (3.6/min; 17/min during the video, worst minute 20) | **0** |
    | load average (1 min), mean / during the video / max | 4.2 / 6.5 / 11.7 | 3.0 / 3.7 / 6.2 |
    | A/V worst / drift | −21.7 / −6.7 ms | +16.1 / +14.8 ms |
    | memory slope after the 5-min warm-up | 315 MB/h | 232 MB/h |
    | resident at the 2nd / 3rd / 4th Scripture | 687 / 709 / 737 MB | 602 / 615 / 638 MB |
    | heap in use / held free (after only) | | 938 → 921 MB (flat) / 165 → 246 MB |

    The late frames are gone. Memory: the level is 85–100 MB lower (each trim gives 10–65 MB
    back and takes 3–9 ms on its own thread; no late frame came with one), and **what is in
    use does not grow**: no leak in the caches (flat, 120–130 MB) or anywhere else that holds
    memory. What still grows is memory the allocator holds free in pieces smaller than a page
    or under live chunks, which no trim can return. Twenty minutes is warm-up for this: the
    overnight slope halved each hour (106 MB/h over 3 h, 25 MB/h after two hours). Next: a
    1–2 h run with the trimmer to see where the slope settles; if it stays above the limit,
    another allocator (jemalloc or mimalloc) for the server is the owner's call.

- **Lane M, 2026-11-26: the allocator** (IDN Beta16). A 2 h run on main (`memory-1126`) still
  grew 114 MB/h (62 in its second hour) with the heap in use flat and glibc's held-free memory
  rising 228 → 525 MB. Owner's call: try mimalloc. Two 30-min runs, back to back under the
  build lock, same harness and settings (`--outputs 2`, captions `base.en`, recording, nice
  10), compared with `memory-1126`'s first 30 minutes (slopes over 7.5–30 min, the 30-min
  run's warm-up):
  - **Whose memory:** with Rust on mimalloc, glibc's numbers count only the C libraries
    (FFmpeg, whisper, GPU drivers), and their held-free memory still grew as before (92 → 248
    MB, +354 MB/h): the growth was in C, which a Rust allocator can't move. glibc had raised
    its mmap threshold (dynamic, up to 32 MiB), so the C side's big buffers came from the
    arenas and left holes there.
  - **What's built:** mimalloc for Rust (cargo feature `mimalloc`) and, for the C side, a
    fixed mmap threshold of 256 KiB and at most 4 arenas (feature `glibc-tuning`,
    `heap::tune_glibc`, first thing in `main`; run 2 set the same through `GLIBC_TUNABLES`,
    which has to be set inside `sg render -c "…"`: `sg` is setgid, and glibc drops the
    variable for it). The two features are independent (`default` in
    `crates/vesper-server/Cargo.toml`). **Owner's choice (2026-11-26): `glibc-tuning` on,
    `mimalloc` off** (the growth was on the C side; mimalloc added 100–150 MB resident and
    its runs had worse frame pacing, cause not isolated); a 30-min run of that build confirms.
  - **Confirmed (2026-11-26, `glibc-1126`, IDN Beta17, same settings, first 30 min):** the
    glibc-tuning-only build **passes every check**: resident 589 / 588 / 592 / 589 / 595 MB at
    10–30 min, slope **+9.4 MB/h** (second half +10.9; limit 40), glibc held free 11 → 22 MB,
    heap in use and caches flat; frame pacing 8 late renders in 162k ticks (worst minute 4),
    0 dropped, 0 timecode gaps; A/V worst +11.2 ms, drift +5.3 ms. Lower and flatter than
    every other column, and none of mimalloc's pacing loss.
    The once-a-minute trimmer stays for what is left on glibc.

    | first 30 min | main (glibc) | mimalloc | mimalloc + glibc tuned |
    |---|---|---|---|
    | resident at 10 / 15 / 20 / 25 / 30 min (MB) | 652 / 670 / 670 / 677 / 696 | 807 / 825 / 822 / 832 / 824 | 784 / 769 / 779 / 785 / 786 |
    | resident slope 7.5–30 min / its second half (MB/h) | +158 / +187 | +113 / +90 | **+22** / +63 |
    | glibc held free at 10 / 30 min (MB) | 194 / 302 (Rust and C) | 142 / 248 (C) | **24 / 25** (C) |
    | glibc held free slope (MB/h) | +357 | +354 | **+7** |
    | mimalloc committed at 10 / 30 min (MB) | | 463 / 479 | 447 / 460 |
    | late renders (worst minute), dropped | 13 (3), 0 | 46 (10), 1 | 52 (16), 1 |
    | output timecode gaps; A/V worst | 1; −24.6 ms (over 120 min) | 1; −30.0 ms | 15; −46.4 ms |
    | load average (1 min), median / max | 2.7 / 7.8 | 3.6 / 10.8 | 3.4 / 9.2 |

    The memory check passes with both (21.9 MB/h by the harness's own measure, limit 40), and
    glibc's held free stays flat. Thirty minutes doesn't settle the slope (the second half is
    noisier: each video phase moves resident memory by 30–40 MB), and the **level** is about
    100–150 MB above main's from the first minute on (not measured where: mimalloc's
    `[anon:mimalloc]` mappings held 260–280 MB resident, for 450–480 MB committed). **Frame pacing was worse** in both
    mimalloc runs, all of it during the video with sound while captions ran (load 5–8.5 then
    against main's 1.5–5); the cause isn't isolated (the allocator, or other work on the box
    outside the build lock), and the soak's rule of two runs left no run to separate it. The
    glibc tuning without mimalloc was not measured (no third run). The
    recording FAIL in run 2's summary was the harness: its last sample came after it had
    stopped the recording (fixed: the watcher stops first).

- **Lane G, 2026-11-27: captions' worker on the CPU and on the GPU** (IDN Beta18; whisper in
  IDN's worker process, the `vesper` binary). Two 30-min runs back to back under the build lock
  (`--outputs 2`, captions `base.en`, recording, server at nice 10), compared with the earlier
  run of the worker at IDN's defaults (`Priority::Low`, 14 threads). The worker's CPU and memory
  were sampled from `/proc` (it is a child of the server, outside the harness's numbers), the GPU
  from amdgpu's sysfs (`gpu_busy_percent`, `mem_info_vram_used`); the box's only GPU (a built-in
  Radeon 680M) renders the screens, decodes the video and runs the worker's Vulkan.

  | 30 min each | worker at IDN's defaults (earlier) | CPU, lowest priority, 8 threads (main) | GPU (Vulkan), lowest priority, 4 threads |
  |---|---|---|---|
  | late + dropped frames, whole run | 536 (17.8/min) | **9 (0.3/min)** | 2327 (77/min) |
  | during the video with sound | 521 (84/min) | **7 (1.1/min)** | 2262 (367/min) |
  | worst minute | 103.5 | 3 | 391 |
  | worker CPU (cores), mean / during the video | not measured | 1.8 / 6.4 (max 8.0) | 0.02 / 0.08 |
  | worker resident memory | not measured | 373 MB | 139 MB (+ ~390 MB of VRAM) |
  | GPU busy, mean / during the video | not measured | 6.5 % / 10 % | 7.3 % / 23 % |
  | captions load (work per second of sound), median / max | not sampled | 0.86 / 1.78 | 0.13 / 0.15 |
  | load average (1 min), median / max; during the video | 3.3 / 11.5; 6.2 | 3.1 / 8.8; 4.8 | 1.7 / 4.1; 1.7 |
  | output receiver timecode gaps | 2 | 59 (57 in one burst at 21 s) | 0 |

  - **The CPU at the lowest priority on half the CPUs fixes the late frames** (main c922f66):
    1.1 a minute during the video with sound, against 84 at IDN's defaults; the run passes
    every check. whisper keeps up on average (load 0.86) but falls behind for a moment while
    the sync clip plays (1.78): captions come later then, frames don't.
  - **The GPU costs frames on a box whose screens render on the same GPU:** 367 late renders a
    minute during the video with sound, though the machine was almost idle (load 1.7) and
    whisper was six times faster (0.13). nice and the idle class rank CPU threads, not GPU
    work: each pass's burst of compute (up to 100 % busy) holds the GPU the screens need.
    **The CPU stays the default** (`docs/CAPTIONS-PLAN.md`); the GPU is a choice for a card
    the screens don't use (IDN Beta18 can't be told which GPU to use: the best one, a card of
    its own first).
  - **The 59 timecode gaps** of the CPU run: 57 came in one 5-second sample 16–21 s into the
    run (pre-service: the receiver got 93 frames instead of 150, a stretch of about 1.9 s
    missing), the other 2 single frames, as in earlier runs (0–2). The server counted no late
    or dropped frame then and logged nothing; the GPU run had none. Not attributable from these
    numbers (the probe's receiver in the harness, or the network path at start); the harness
    logs only a total, so it should log each gap's time and length to tell a burst from many.

## Not covered yet

- A sound card unplugged during the run (no removable card on the soak box): the server's lost
  card path is covered by its status (`state.audio`, Oriel's status bar) and IDN's reopen pacing.
- Windows (the harness reads `/proc` through the server; on Windows `process` is empty).
- Physical displays (KMS): `--no-displays` keeps the run off the box's monitors.
