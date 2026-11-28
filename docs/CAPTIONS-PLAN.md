# Live captions (batch 13, lane C)

Speech from a sound source turned into caption text by IDN's `idn-captions` (whisper.cpp on
this machine, on its GPU or CPU; nothing leaves the box but the text), shown on screens as a **captions layer**
and optionally sent on the network as an IDN text-only captions source. Section 7 tracks what
is built.

Inputs: the `idn-captions` public API (`Engine` / `EngineConfig`: push sound at any rate,
`step()` about once a second gives `CaptionLine`s, a draft corrected in place and final at a
pause; `ensure_model_stop`, `auto_choose_stop`, `MODEL_LIST`, `model_speaks`), `idn_open_protocol::text` (`TextSource`
announcing `text/1`, `send_caption`) and `idn_open_protocol::cea608` with
`Output::caption_next` (closed captions that reach an output's taps, a stream's or
recording's encoder, never receivers).

## 1. Model

- **`Layer::Captions`**, the top-most visual layer (above messages, under the screen's mask);
  `LayerVisibility.captions` (a missing field means visible, so old looks show captions); no
  transition of its own (lines replace each other with a cut, as captions do).
- **`CaptionsConfig`**, a machine-local singleton document (`machine/`, like
  `AudioDeviceConfig`: the model that suits this box and its sound sources are this box's):
  - `enabled` (default off), `source`: `program` (the program audio bus, default),
    `liveSource { name }` (a network source's sound) or `external` (no engine: lines only from
    `POST /captions/lines`); `language` (`en` default), `processor` (`cpu` default, or
    `gpu`), `words`
    (names to spell right: whisper's prompt), `model` (`auto` default, or one of the crate's
    `MODEL_LIST`), `keepMusic` (caption sung lines too; default off: whisper invents text
    over music);
  - `maxLines` (1–4, default 2), `style` (font family, size, text and box colours, box
    opacity, position bottom / top, margin): a theme-able text box drawn on the layer;
  - routing: `audience` (the captions layer on audience screens, per look like any layer)
    and `stage` (also on stage screens, under their layout); `network` (send the lines as an
    IDN text-only source called "Captions").

## 2. API (`crates/protocol/README.md`)

- `GET /api/v1/captions/config`, `PUT /api/v1/captions/config` (validated, saved, applied
  live: the engine restarts only when its source, model, language or words change; a new
  engine waits for the one before it to end, so two never download or measure at once. That
  wait is short: the old engine's stop reaches IDN's fetch and measurement (IDN v0.3.4-Beta11,
  `ensure_model_stop` / `auto_choose_stop` with the worker's stop flag), which end within
  about a second, also mid-download, while connecting, in the hash check or a measuring pass;
  their `STOPPED` is a normal stop, not an error. A stopped download leaves nothing under the
  model's name and starts again at the next fetch (no resume).
- `POST /api/v1/captions/lines` `{ text, final }`: a line from another captioner (a person
  typing, a CART service, a test). Lines from it and from the engine go the same way.
- `clearLayer { layer: "captions" }` takes the lines off the screens (captions keep running).
- State: `ShowState.captions` `{ enabled, status, detail, model, device, gpu, note, load, lines }`
  (`gpu`: the device is a GPU; `note`: why whisper isn't on the processor chosen);
  `status` is `off`, `starting`, `downloading` (with `progress`), `running` or `error`;
  `lines` are the last `maxLines` lines (the last may be a draft).
- `GET /api/v1/captions/models`: the models with their size and what they suit, and which
  are on this box.

## 3. Server

- `captions.rs`: a hub in `Shared` (config, status, lines) and one worker thread while
  enabled: it reads the source (a program-audio tap; or the live source's sound feed, kept
  open), mixes to mono, pushes into `Engine`, calls `step()` every 500 ms and hands the lines
  to the hub; the hub wakes the actor, which copies the shown lines into the show state
  (screens re-render, clients get the state). Final lines are kept (the last `maxLines`);
  a draft replaces the previous draft of its line; lines go off screen after 8 s of quiet.
- Models: `<data>/cache/whisper/` (outside backups), fetched on first use from the crate's
  pinned Hugging Face commit with size and SHA-256 checked (`ensure_model_stop`); progress is
  the `downloading` status with `progress` (%), `downloaded` and `downloadSize` (bytes), shown
  as a progress bar in Settings ▸ Captions. `auto` measures this box once
  (`auto_choose_stop`; since IDN v0.3.3 the models it tries report the same progress).
  An English-only model (`.en`) is refused for another language: when the settings are saved
  (`model_speaks`), and by the worker for a settings file from before that check (IDN Beta11's
  `Engine::new` refuses it too), saying to choose `auto` or `large-v3-turbo`.
- Drawing: the layer is a slide built from the style (a box with the lines), on the screen's
  canvas; on stage screens under the layout when `stage` is on.
- Network: an IDN `TextSource` "Captions" while `network` is on, each line sent as it comes.

## 4. Oriel

- Show page: a **Captions** chip in the right column's layer controls: on / off, the status
  (downloading %, running, error), the last line as a live caption preview, and a link to
  the settings.
- Settings ▸ Captions: on / off, source (program audio or a live source), language, model,
  words, max lines, style (size, colours, box, position), audience / stage / network.
- Screens ▸ Looks: the looks matrix gets a "Captions" row (from `LayerVisibility`).

## 5. Owner questions (answered with the recommendation; easy to change)

1. **Where do models come from?** Downloaded on first use from whisper.cpp's own files at a
   pinned commit (what the crate does), into the data folder's cache; not in the
   installers (75 MB to 1.6 GB, and the right one depends on the box). An offline box takes
   the file copied into `cache/whisper/` by hand (it is checked against its pin).
   **Recommended default: `auto`** (measures the box once); on a typical church PC (CPU,
   8+ threads) that is `small.en` (466 MB), on a small one `base.en` (142 MB).
2. **CPU or GPU?** **The CPU by default; the GPU only when chosen** (`processor`, Settings ▸
   Captions ▸ Processor: CPU / GPU). Owner, 2026-11-27: try the GPU; the soak (lane G,
   `docs/SOAK-PLAN.md`) then showed that on a box whose screens render on the same GPU (here a
   built-in Radeon 680M) whisper's GPU passes cost about 370 late frames a minute during video
   with sound, against about 1 on the CPU at the lowest priority: nice and the idle class rank
   CPU threads, not GPU work. So the CPU stays the default (coordinator, 2026-11-27) and the
   settings warn under the GPU choice. The old reason to stay on the CPU, a second
   Vulkan instance beside the renderer's in the server process (the loader race), is gone:
   whisper runs in IDN's worker process (below), so its Vulkan instance and device live there,
   never in the server; the server never calls `idn_captions::gpus()`. The device is chosen by
   the worker (`EngineConfig::gpu`: the best GPU, a card of its own first) and reported as
   `Engine::device` (the GPU's name, or "CPU (N threads)"), shown in the settings as "In use".
   IDN's public API (Beta21) can't pick a particular GPU (`EngineConfig::gpu` is on/off; `Gpu`'s
   `index` is only reported), so "a second card for captions" can't be chosen here; the renderer
   too prefers a card of its own, so on a box with one card both use it.
   - **Fallback, never a stop:** a GPU that can't load the model (no memory, a driver error) is
     `Engine::new`'s error: the engine loads the model on the CPU at once and the status's
     `note` says why; a GPU worker that ends is started again on the CPU by IDN (Beta18), and
     the note says so when the device changes. Either holds until the engine restarts (captions
     off and on, or the settings changed); then the GPU is tried again. A load that fails on
     the CPU too is the model's problem: the usual retries (below), each starting on the GPU.
     `processor: gpu` on a box without a usable GPU runs on the CPU with a note. The `auto`
     model's choice is measured per device (IDN keys `auto.txt` on the device's name), so a
     fallback to the CPU may measure and pick a smaller model.
   - **Priority and threads:** the same `Priority::Lowest` worker; on the GPU whisper's CPU
     threads only prepare each pass (the spectrum, the decoding around the GPU's work), so it
     gets at most 4 (half the CPUs on the CPU, at most 8).
   - **Building:** on Linux always (`libvulkan-dev`, `glslc`; at run time the Vulkan loader and
     driver the renderer needs anyway); on Windows the `gpu-captions` feature of
     `vesper-server`, not built yet (the MinGW cross build would need the Vulkan SDK's headers
     and loader library and `glslc`): the Windows server stays on the CPU (`docs/PACKAGING.md`).
   - Measured in the soak (`docs/SOAK-PLAN.md`, lane G).

   *Before GPU captions (kept for the CPU path):* Cost on the CPU: the crate's own
   guide is `base.en` for a small machine, `small.en` for 8 threads or more. **whisper runs in
   IDN's worker process at the lowest priority** (IDN v0.3.8-Beta18, `Priority::Lowest`: Windows'
   idle priority class, nice 19 elsewhere, for every thread whisper starts), on half the CPUs
   (at most 8). The worker's defaults (`Priority::Low`, all CPUs but two) brought back 84 late
   frames a minute during video with sound in a 30-min soak (2026-11-27). The worker is the `vesper` binary itself
   (`vesper --idn-captions-worker lowest`, served by `idn_captions::worker::serve_if_asked()`
   first thing in `main`; `EngineConfig::worker` is the running program, `/proc/self/exe` on
   Linux), so packages carry nothing more. A pass runs about once a second while someone
   speaks, nothing over silence or music, and the screens' frames always come first: before
   this, whisper on all but two threads at the server's priority gave the soak's video with
   sound about 100 late frames a minute (lane P, 2026-11-26), and Vesper's own workaround
   (half the threads, the worker thread reniced) couldn't reach whisper's threads on Windows
   (H05-1, fixed upstream in Beta18; the workaround is gone). Under systemd a worker the server
   starts is in the server's own cgroup, so nice ranks it below the server's frames. The status
   shows the engine's `load` (work per second of sound; below 1 it keeps up); `auto` measures
   in the worker at its priority under the box's load at the time, so it picks a model that
   keeps up. **Failures:** a model the worker can't load (it ended twice, was too slow, or
   refused the file) is `Engine::new`'s error: the status shows it ("… (trying again in N s)")
   and the engine tries again after 5 s, 30 s, 60 s, then every 5 minutes; a step that fails
   (a pass over IDN's 120 s timeout is skipped, a worker that ended is started again by IDN
   after 1 s, 5 s, 30 s, then every minute) shows as the status's error until a step works
   again. While there's no sound to step with (no source, or a network source that sends
   nothing) the engine calls `Engine::tend()` each half second instead (IDN Beta21), so a
   worker that ended starts again on that schedule then too, not only once sound comes. A
   step runs as `Engine::step_stop` with the worker's stop flag: a stop or a settings change
   ends a pass under way (IDN kills a worker's pass that doesn't end within a second) instead
   of waiting for it. Neither stops the server or the show. whisper runs in the server's process only when
   no worker program starts and says IDN's hello (logged once; with the GPU it would then open
   Vulkan in the server, so the worker is what makes GPU captions safe).
   *The IDN maintainers' guidance (2026-11-25), for when GPU captions are taken up:* ggml's Vulkan
   creates one `VkInstance` on the first touch of its registry (the first `gpus()` call, or
   `Engine::new` with a GPU) and never destroys it, and one `VkDevice` per GPU kept for the
   process's life (dropping an `Engine` frees only buffers and pools). Call
   `idn_captions::gpus()` once at startup on the main thread, not at the same time as wgpu
   creates its instance. That is supported by the code but not lab-tested next to wgpu; a
   separate captions process stays the safe fallback. That is the path taken: the worker.
3. **Languages other than English** need a multilingual model; the crate offers only
   `large-v3-turbo` for that (GPU class). The language field is there; non-English picks
   `large-v3-turbo` (asked of the IDN maintainers: smaller multilingual models; not approved in
   the 2026-11-25 round, so `large-v3-turbo` stays).
4. **Closed captions in streams and recordings** (CEA-608 through `Output::caption_next`):
   deferred to when recording and streaming land (lane R): the hub's final lines are what
   a `cea608::Captioner` takes.

## 6. Deferred

Hardware audio inputs as a source (the server has no input capture yet), CEA-608 in
streams and recordings, captions as a stage layout element (the stage option draws the
captions box under the layout), GPU whisper on Windows, a captions history / transcript export.

## 7. Status

- [x] Stage 1: model, protocol, server (hub, worker, layer, API). A third source, `external`
  (lines only from `POST /captions/lines`), makes the layer testable without a model and
  serves a person typing or a CART service. Tests: the layer with injected lines (draft, final,
  max lines, clear, looks, the hold, persistence) and the engine on the program bus with a real
  `base.en` model when `VESPER_TEST_WHISPER_DIR` names one (no speech synthesiser on the build
  box, so no line is checked from real speech).
- [x] Stage 2: Oriel (the Show page's captions strip, Settings ▸ Captions, the looks matrix's
  Captions row, the Oriel menu's Settings ▸ Captions…), stub routes, shots.
- [x] Stage 3: docs (protocol README, PACKAGING, PRODUCTS, ORIEL-REDESIGN-PLAN).
- [x] IDN v0.3.3: `auto_choose_with`, so the models `auto` downloads to measure report their
  progress; the state's `downloaded` / `downloadSize`, and a progress bar under the status in
  Settings ▸ Captions (stub control `__stub/captions`, shot `settings-captions-download`).
- [x] IDN v0.3.4-Beta11: a stopped engine ends within about a second, also inside a model
  download, its hash check or `auto`'s measurement (`ensure_model_stop`, `auto_choose_stop`
  with the worker's stop flag; `STOPPED` is a normal stop); an English-only model for another
  language is refused with a clear message. Unit test: a stopped worker inside IDN's hash check
  ends at once, the model file left as it was. (`Engine::measure_stop` isn't needed: Vesper
  measures only through `auto_choose_stop`.)
- [x] IDN v0.3.8-Beta18: whisper in IDN's worker process at low priority, the `vesper` binary
  as the worker (H05-1 fixed upstream), at `Priority::Lowest` on half the CPUs (the defaults let late
  frames back in during video with sound). A failed model load is shown and tried again; failing steps are shown until one
  works. Tests: the engine on the program bus runs whisper in a `vesper --idn-captions-worker`
  child (with `VESPER_TEST_WHISPER_DIR`); a worker that can't load its model (a damaged file,
  always run) leaves the server running, shows the error, tries again and leaves no worker
  behind when captions turn off.
- [x] IDN v0.3.8-Beta21: `Engine::tend()` while there's no sound (a dead worker restarts on
  time in silence), `Engine::step_stop` (a stop never waits for a pass). The CPU device label
  now counts the threads a pass uses ("CPU (8 threads)" on a 16-thread box, was the machine's
  count): the settings show it as it comes, and since IDN keys `auto`'s measurement on that
  label, the first start after the update measures the box once more. Test (with
  `VESPER_TEST_WHISPER_DIR`): a worker killed while its network source sends nothing is started
  again.
- [x] GPU captions (lane G, IDN Beta18): IDN's `gpu` feature built on Linux; `processor` (CPU
  default, GPU) in the settings with what is in use, a GPU failure falling back to the CPU with
  a note. Measured: the CPU at the lowest priority keeps frames on time; the GPU shared with the
  screens doesn't (`docs/SOAK-PLAN.md`). Tests: the engine on the CPU and on the GPU (or the CPU
  with a note) with a real model (`VESPER_TEST_WHISPER_DIR`); the processor's default.
- Not done here: the captions configuration in settings backups (a machine document the
  backups don't list yet), a test of the network text source, a stage-screen pixel test.
