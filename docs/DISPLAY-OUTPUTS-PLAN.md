# Physical display outputs (G13): plan

A screen can be sent to a monitor or projector attached to the box (HDMI / DisplayPort), next to
or instead of its IDN Open Protocol outputs. This note picks how, for display boxes running
Ubuntu without a desktop and for Windows. Batch 5, lane C. Status of each stage at the end.

## The Linux approach: direct KMS, no compositor

| | (a) KMS (kernel mode setting), no compositor | (b) wlroots kiosk compositor | (c) winit windows under any compositor / X |
|---|---|---|---|
| Extra packages | none | `cage` or `sway`/`labwc`, plus an auto-login session | a desktop, or (b) |
| systemd | the same kind of system service as a network box | auto-login user, getty override, user unit | a user session |
| Multi-head | one CRTC per connector, each with its own mode | cage is single-view: `-m extend` stretches one window over every output, `-m last` uses one; per-output windows need sway/labwc window rules | one window per monitor, placed by the compositor |
| Modes | exactly the connector's modes; we choose | the compositor's config | the compositor's |
| Our device | not involved (see below) | surfaces of the shared device | same |
| Risk | needs DRM master (no desktop on that card) | more moving parts on an unattended box | not headless |

**Evidence (this machine: AMD GPU, RADV, Mesa 26.0, wgpu 30.0.1, no monitor attached).**

- **wgpu's own DRM path doesn't work with our shared device on Mesa.** wgpu 30 has
  `SurfaceTargetUnsafe::Drm` (feature `drm`): it enables `VK_EXT_acquire_drm_display` (and so
  `VK_KHR_display`) on the instance, then calls `vkAcquireDrmDisplayEXT` and creates a
  display-plane surface. Tried here: with `VK_KHR_display` enabled, RADV opens the card's primary
  node (`/dev/dri/card0`) itself when the device is created (seen in `/proc/self/fd`), which
  makes the *driver* the DRM master, and every `vkAcquireDrmDisplayEXT` then fails with
  `ERROR_INITIALIZATION_FAILED`, on every connector. wgpu-hal also `.expect()`s that call (a
  panic, caught in the experiment). wgpu offers no way to hand it a `VkSurfaceKHR` made with
  plain `VK_KHR_display` (its `create_surface_from_vk_surface_khr` is private). So the GPU-surface
  variant of (a) is not available with wgpu 30; the `drm` feature is **not** enabled (it would
  also make every server process, network boxes included, the card's DRM master through RADV).
- **Plain KMS works for a user process.** With the renderer (shared wgpu device) up, the server
  process opens `/dev/dri/card0` and is its DRM master (no desktop running: the first opener
  becomes master; checked with the master-only auth call), as a non-root user in the `video`
  group. Connectors, status and EDID come from sysfs; exact modes from `DRM_IOCTL_MODE_GETCONNECTOR`
  (no master needed).

**Recommendation (taken): (a) KMS with dumb buffers.** The screen's frame is already on the CPU
(the renderer reads every frame back for the IDN outputs). A display output converts it to
XRGB8888 into one of three dumb buffers (letterboxed to the mode), and page-flips to it at the
next vertical blank (triple buffering: the buffer written is neither on screen nor waiting). No
Vulkan surface, so the one shared device is untouched; any KMS driver works (amdgpu, i915, nouveau,
simpledrm, vkms). Cost: a 1080p frame is one 8 MB copy and swizzle on the output's own thread.
The owner installs the same `.deb` and enables `vesper-display.service` (the same server, see
Packaging): no compositor, no auto-login user. (b) stays the documented fallback; (c) is the
**Windows** path and the path for a Linux box with a desktop (stage 1b). A GPU-direct present
(no CPU copy) is later work: a wgpu with a public `VK_KHR_display` surface, or GBM/dma-buf export.

## Model

`OutputTransport::Display { selector, fullscreen, mode? }` (exists since schema 1; `mode` is new
and optional): `selector` names the monitor by `connector` (`"HDMI-A-1"`), then EDID `name`, then
`index`, as `DisplaySelector` already documents. `mode` is `{ width, height, refreshMhz }`
(`null` = the display's preferred mode). Bindings are machine documents
(`machine/outputBinding/<id>.json`), stored like IDN bindings: not portable, not in backups of
the show. A display may be bound to one output only (validation), and a binding may name a
display that is unplugged (it stays configured, its output is unavailable until it's back).

## Server

- `displays` module: a `DisplaySource` (list + open). Linux: `Kms` lists
  `/sys/class/drm/cardN-*` (connector, status, EDID: manufacturer, model) and reads modes with
  the DRM ioctls (a full probe, through our master descriptor when we hold one, so a re-plugged
  monitor's modes are fresh); opens a KMS sink. Tests: `FakeDisplays` (settable list, sinks that
  count frames), injected through `ServerConfig::displays`. Other platforms: an empty list until
  stage 1b.
- **Presenting**: a display output is an ordinary `FrameSink` on its own worker thread (same
  delay line, overrides, stats as IDN). The KMS sink picks a free CRTC for the connector, sets
  the mode with three black dumb buffers, then flips one per frame; while a flip is pending the
  worker waits (at most one frame) and the one-slot channel in front of it drops what the
  display can't take, so IDN outputs are never slowed. Pacing: the screen clock and the
  display's refresh are free-running; at equal rates that's an occasional repeated or skipped
  frame. Locking a screen's clock to a display's vblank is deferred (it would make the display
  the master clock of that screen and its IDN outputs).
- **Hot-plug**: a monitor thread re-lists every 2 s (sysfs reads, cheap). A change updates
  `state.displays` (broadcast) and rebuilds the display outputs whose display came or went: an
  unplugged display's output becomes `unavailable: "HDMI-A-1 is not connected"`; plugging it back
  restarts it. A present error in between shows as `error` on that output.
- **API**: `GET /api/v1/displays` → `DisplayInfo[]` (`id` = connector, `name`, `manufacturer`,
  `connected`, `modes`, `preferred`, `boundTo`), the same list in `state.displays`;
  `output-transports` reports `display` as supported/available with the backend in `status`.
  Identify uses `identifyScreens` (the label lists the screen's outputs, so the display shows its
  screen's name).
- Server fix from batch 4: an empty data folder gets its `portable/` created at start.

## Oriel (Screens page)

In a screen's outputs, transport "System display": a display picker (connector, model, size,
connected or not, which output already uses it), a mode list (Preferred + the display's modes),
status (sending / not connected / error). Stub routes and fixtures for `/displays`.

## Packaging

`packaging/linux/vesper-display.service`: the same server, user, hardening and
`/etc/default/vesper` as `vesper.service`, plus `After=plymouth-quit-wait.service` (the boot
splash holds the displays while it runs) and `Conflicts=vesper.service`. Shipped, not enabled;
the `.deb` scripts stop, restart (whichever is enabled) and purge both. `--no-displays` turns
display handling off. Windows needs nothing extra (stage 1b).

## Deferred (G13's rest)

- ~~1b: windows (winit)~~: done in batch 6 (see "Stage 1b").
- Clock locking to vblank; adaptive sync; HDR. (Placeholders, output arrangements and the
  identify overlay for an unused display: batch 12, see "Screen layouts" below.) (Colour correction and corner pin: done per screen in batch 6;
  Fill/Key on two connectors: batch 11, see "Fill/Key on two connectors" below.)

## Screen layouts: arrangements, placeholders, identify (batch 12, lane W)

The rest of G13 that needs no new hardware. The board (Screens, `docs/ORIEL-REDESIGN-PLAN.md`
4.8) shows an arrangement per screen (Single / Mirror / Grouped / Edge Blend), an output canvas
and Placeholder in the output picker.

**Model.**
- Portable `Screen.arrangement` (`ScreenArrangement`, default Single, not stored when default):
  `kind` `single` | `mirror` | `grouped` | `edgeBlend`; `columns`, `rows` (1–8 each); `bezel`
  `{x, y}` (Grouped: canvas px hidden behind the frames between two tiles); `overlap` `{x, y}`
  (Edge Blend: canvas px two neighbouring projectors both show); `blend` `{gamma, curve}`
  (Edge Blend: the projectors' gamma, default 2.2, and the ramp's shape, 1 = linear, default 2).
  The **canvas is the whole wall** (`Screen.size`); a tile is `(size − (n−1)·gap) / n` with
  `gap = bezel` (Grouped) or `−overlap` (Edge Blend). Portable because the canvas size it
  explains is portable, and the importer has it at the screen level.
- Machine-local `OutputBinding.tile` (row-major cell, optional): the cell an output shows in a
  Grouped / Edge Blend screen. `OutputBinding.region` (already in the model, the importer
  fills it) now works: an explicit crop of the canvas, it wins over the tile's cell.
- `OutputTransport::Placeholder`: an output slot without hardware yet (a wall tile whose
  projector isn't bought or cabled). Kept in the config, never runs, listed as `placeholder`
  with the detail "No hardware yet"; Oriel shows it as a placeholder tile.

**Single vs Mirror.** On the server both mean "every output shows the whole screen" (what
every existing config does, so no config breaks); the difference is the operator's intent,
which Oriel shows (adding a second output to a Single screen makes it Mirror).

**Pixels (server).** Each output's worker: crop the cell / region out of the screen's frame
(integer pixels, rounded), scale it to the output's size (its resolution override, else the
crop's size), then, for Edge Blend, multiply every **inner** edge of the tile by the ramp
(each edge that has a neighbour gets its own ramp, as wide as the overlap in that direction:
columns use `overlap.x`, rows `overlap.y`; outer edges are untouched). The ramp is
`f(t) = ½(2t)^p` for t < ½, `1 − ½(2(1−t))^p` above (p = `curve`), so the two projectors'
`f(t) + f(1−t) = 1` everywhere in the overlap; it is applied **gamma-aware**: the
sRGB-encoded value is multiplied by `f^(1/gamma)`, so the *light* adds up to one; in a corner
the two ramps multiply (four tiles still sum to one). Colour only, alpha untouched. Cost: a
crop copy, plus a multiply over the overlap strips only. Done on the CPU in the output worker
(where scaling and Fill/Key already happen), so IDN, display, window and PNG outputs all get
it; no render pass changes.

**Validation.** Columns / rows 1–8; bezel 0–2000; overlap ≥ 0 and smaller than a tile; a tile
must come out at least 16 px; gamma 1–3, curve 1–4; Single / Mirror keep 1×1 (Oriel resets
them). An output's region must lie inside the canvas and be at least 16 px; its tile must be a
cell of its screen's arrangement (checked when the output is saved; a screen re-arranged to
fewer cells later leaves such outputs unavailable: "tile 4 is outside the 2×1 arrangement").

**Identify an unbound display.** `POST /api/v1/displays/{id}/identify`: a display no output
uses shows its own name, connector and mode full screen for 5 s (the same label as Identify
screens, rendered on the render thread like a thumbnail, sent on a one-off display sink).
Refused for a display an output uses ("use Identify screens") or one that isn't connected.

**Oriel.** The screen editor gets an Arrangement row (Single / Mirror / Grouped / Edge Blend),
columns, rows, tile size (sets the screen size), bezel or overlap and blend settings, and the
output canvas: the grid with each cell's output (name, or "Placeholder", or "Empty"), click a
cell to add or pick its output. Outputs get a Tile picker; the transport picker gets
Placeholder. The display picker gets Identify for displays no output uses.

**Import.** `arrangementCombined` → Grouped (rows, columns, tiles in sub-screen order);
`arrangementEdgeBlend` → Edge Blend (one row of `screenCount` columns, tiles in order; the
overlap from the sub-screens' bounds, the curve from the blend mode). Sub-screens of those
arrangements without an output display become placeholders. Single screens: as before.

**Mosaic** (`docs/PRODUCTS.md`, decided 2026-11-24) reuses this for one-box walls: a wall is a
Grouped (bezel compensation) or Edge Blend screen; linked-box walls later reuse the same
per-output crop (`region`) and blend on each box.

**Later.** Per-seam blend widths and per-projector black-level compensation (uplift of the
non-overlap areas); a mesh warp; a GPU blend pass if a wall ever needs it at 4K60 per output.

Stages (batch 12, lane W): 0 this note; 1 server (model, crop and blend in the output worker,
validation, placeholders, identify display, import, protocol README, bindings); 2 tests
(pixels each output gets for Grouped and Edge Blend on PNG outputs, validation, placeholder,
identify); 3 Oriel (arrangement, canvas, tile picker, placeholder, identify) with stub and
shots; 4 docs. Status at the end of this section as they land.

**Status (batch 12, lane W).** All stages done.
- Server: model (`ScreenArrangement`, `OutputBinding.tile`, `OutputTransport::Placeholder`),
  crop / scale / blend in the output worker (`outputs::Delivery::for_screen`, `apply_blend`),
  outputs rebuilt when their crop changes, validation, `POST /displays/{id}/identify`,
  capability `screenLayouts`, import (combined → Grouped, edge blend → Edge Blend, empty tiles →
  placeholders), protocol README, bindings.
- Tests: `model` (tiles, bezels, overlaps, the ramp sums to one), `outputs` (crop, blend in
  light), `pro-import` (Grouped and Edge Blend imports, placeholders), server
  `screen_layouts::` (the exact pixels of a Grouped wall's two displays against the whole
  canvas, the light of two Edge Blend projectors summing to one across the overlap; validation,
  placeholders, a re-arranged wall, identify). `VESPER_LAYOUT_PNGS=<dir>` saves what each
  simulated display got, to look at.
- Oriel: the Arrangement row (Single / Mirror / Grouped / Edge Blend, across, down, each
  output's size, bezel or overlap, gamma, curve), the output canvas (tiles with their outputs,
  placeholders dashed, blend zones hatched; a click on an empty tile adds a placeholder),
  "Shows:" tile picker, Placeholder in the output picker, "Identify this display" for a display
  no output uses; a second output makes a Single screen Mirror. Stub `screensStub.wall`; shots
  `screens-layout-{blend,grouped,placeholder,identify}`.
- **Not verified here:** real projectors (the blend on a real overlap, a real projector's
  gamma), real displays behind bezels, the identify label on a real monitor (KMS or a window);
  checklist: build a two-projector overlap, set the overlap in canvas px to what the projectors
  overlap, adjust gamma until the seam disappears on white and on 50 % grey.

## Fill/Key on two connectors (batch 11, lane K)

A hardware keyer (a vision mixer's downstream keyer, an SDI keyer behind HDMI/DP converters)
takes the graphics as two signals: the **fill** (the colour) and the **key** (the alpha as a
grey picture: white = opaque). Vesper sends a screen with alpha as such a pair.

**Model: two outputs of the screen, `target` `fill` and `key`.** `OutputBinding.target` (until
now only `full`) gains `fill` and `key` (additive; an older server reads them as `unknown` and
sends `full`, the documented fallback). A Fill/Key pair is two ordinary outputs of the same
screen, each with its own display (or any transport), status, counters, delay and enable, as
the board's "Output Target: Full / Fill / Key" shows. Both get the same frame of the screen at
the same tick (the screen clock offers every output the same `Arc` frame); nothing in the
model ties the two together, so a pair is also possible on two windows, two IDN sources or a
mix. The existing `alphaOverride: keyFill {fillDevice, keyDevice}` (SDI keyers, imported)
stays as it was (treated as straight alpha on one output).

**Pixels (the display path, not the render pass).** The output worker converts the screen's
frame (premultiplied sRGB RGBA8) after its scaling and before the sink, cached per frame like
the scaled copy:
- **key** = alpha as grey, opaque: `(a, a, a, 255)`. An opaque screen has a white key.
- **fill** = the colour over black, opaque. Which colour follows the output's alpha type (its
  `alphaOverride`, else the screen's `alpha`): `straight` (the default for a screen with alpha,
  and what most keyers call "unshaped" / not pre-multiplied) un-premultiplies (`c * 255 / a`,
  black where `a = 0`); `premultiplied` ("shaped" fill, a keyer with "Pre Multiplied Key" on)
  sends the premultiplied colour as it is. The receiving keyer must be set to match (the
  Oriel hint says so).
- Both frames go to the sink as `opaque`: an IDN or PNG sink then sends no alpha plane.

**KMS: two connectors, flipped together where the hardware allows.** Each output is its own
KMS sink (its own CRTC, triple buffer, worker). The fill and key KMS sinks of one screen on
the same card form a **flip pair**: each draws its buffer, then the pair page-flips both
CRTCs back to back, once both have drawn the frame of the same tick (a bounded wait of a
quarter frame; a sink whose partner hasn't come flips alone, so a stalled partner never
holds the other one up). Each half gets its own flip's result, and one CRTC's failed flip
never skips the other's. A sink that found its role taken (fill and key swapped one output
at a time) joins the pair as soon as the role is free. Every clock (a screen's, an output's
own with a frame rate override) ticks on one process-wide grid, so halves at one rate stamp
equal ticks and pair. **Limit:** the two CRTCs scan out on their own timing: unless the
driver keeps identical modes in step (amdgpu can synchronise CRTCs with identical timings;
not guaranteed, not visible to us), the two flips land on vblanks up to one refresh apart, so
on a change the key can lead or lag the fill by one frame. Use the same mode on both
connectors, and for frame-exact keying an SDI card with a genlocked fill/key pair (later). A
single atomic commit of both CRTCs would remove the time between the two flip calls but not
the scan-out phase, so it is not worth the risk without hardware to test on.

**Windows (desktop): two windows.** Two outputs, two borderless windows on their monitors,
each presenting on its own worker (no pairing: `Mailbox` presents at each monitor's next
vblank).

**IDN: a key output is possible but usually unnecessary.** IDN Open Protocol (compatible with
NDI®) carries alpha in the frame itself (the library sends an alpha plane; a receiver gets
RGBA, verified in batch 6), and vision mixers and software that take IDN read the alpha
directly. **Recommendation: one IDN output with straight alpha**, not a pair. A `key` target
on an IDN output still works (a grey picture, no alpha) for a converter or keyer that only
takes separate fill and key signals; Oriel's hint says so.

**Oriel (Configure Screens).** The output card's "Output Target" select (Full / Fill / Key,
until now disabled) is enabled. Choosing Fill offers "Add the key output": a second output of
the same transport targeting Key, on the next free display for display outputs; each card
keeps its own display picker, so the operator picks the fill and the key display. The canvas
boxes say "Fill" / "Key". Hints: an opaque screen's key is solid white; IDN carries alpha.

**Imports.** A ProPresenter screen output with an alpha device (a display) becomes two
binding suggestions: the fill on the output's display, the key on the alpha device, each
with its target. The SDI external-key case stays `keyFill`.

**Owner questions (answered with the recommendation, proceeded).**
1. One output with a key display, or two outputs? **Two outputs** (the board's Output Target,
   the reserved `target` field; each keeps its own status and display picker).
2. Fill un-premultiplied or shaped? **Follow the output's alpha type**: straight (unshaped,
   the default) or premultiplied (shaped); no new setting.
3. A Fill/Key pair over IDN? **Not recommended** (IDN carries alpha); allowed.

**Stages.** 0 this section; 1 server (model, worker conversion, KMS flip pair, labels,
bindings, protocol README) and tests (pixels at a fake display pair for a half-transparent
picture; the flip-pair rendezvous; validation); 2 Oriel (target select, add key output,
hints, stub, shots); 3 import; 4 docs and the hardware checklist.

**Status (batch 11): all stages done.** Server, tests, import, Oriel (shots
`screens-fill-key-add`, `screens-fill-key`), docs and the checklist ("Hardware test
checklist, Fill/Key" below). **Verified here:** the fill and key pixels of a half-transparent
picture at two fake displays (straight and shaped fill), the flip-pair rendezvous with
threads, the import mapping. **Not verified here** (no monitors, no keyer, no Windows): a
real modeset and paired page flips on two connectors, how far apart two CRTCs flip, two
output windows on a real desktop. Later: a single atomic commit for both CRTCs, an SDI
card's genlocked fill/key pair.

## Colour correction and corner pin (batch 6, lane P)

**Where.** Per **screen**, not per output: the adjustments are applied once to the screen's
picture after compositing and before the frame fans out, so IDN, display and PNG outputs all
get them and the display presenters are untouched. They ride on the screen's existing
**output pass** (the draw that turns the linear `Rgba16Float` composite into the RGBA8 frame
that is read back): a second pipeline, `fs_adjust`, replaces `fs_final` while a screen has
adjustments. No extra pass, texture or copy; the same one full-screen draw per frame, with a
few more ALU ops per pixel (a 3×3 projective transform and one bilinear fetch for the pin,
about 20 multiply-adds and one `pow` for the colour) — well under a millisecond at 1080p on
the iGPU baseline. **Identity settings** (or a disabled pin) keep `fs_final`: frames bit for
bit as before (tested in `render` and in the server).

**Settings.** `Screen.color` (`ColorCorrection`): brightness, contrast, gamma, saturation,
red / green / blue gain, plus the board's "Black" as `blackLevel` (lift / crush). Applied to the
straight, sRGB-encoded colour (where operators expect these controls to act, like a video
processor), in a fixed order: gains → saturation (Rec. 709 luma) → contrast about mid grey →
brightness → black level → clamp → gamma; then premultiplied again. **Shader maths, not a
LUT:** eight uniforms, exact, a change is just a uniform (no texture upload or bake), and it
reuses the existing uniform block; a 3D LUT (`.cube` import) can later sit in the same pass as
one more texture. `Screen.cornerPin` (`CornerPin`): `enabled` plus four corner offsets **in
screen pixels, positive = away from the centre** (the board's convention, so a symmetric
keystone is symmetric numbers). The CPU solves the unit-square → quad homography (Heckbert)
and inverts it; the shader maps each output pixel back into the picture, samples bilinearly
and anti-aliases the edge; outside the quad: alpha 0 (black on opaque screens). The server
rejects offsets larger than the screen and non-convex (crossed / folded) quads.

**Live.** `PUT /screens/{id}` or the narrow `PUT /screens/{id}/color` / `/corner-pin` (no
`If-Match`, last change wins: for sliders). The engine calls
`ScreenTarget::set_adjustments` on the same render target; the screen re-renders on the next
change; no output restarts.

**Does transparency survive end to end? Yes, for IDN; not for displays (by design).**
- Screen texture → output pass: a screen with `alpha` straight / premultiplied keeps its
  alpha through `fs_final` and `fs_adjust` alike (frames are premultiplied sRGB RGBA8; the
  colour correction never touches alpha, a fully transparent pixel stays (0,0,0,0) whatever
  the brightness, and the corner pin's outside is alpha 0).
- → IDN: the sink sends frames with alpha as `Premultiplied` RGBA, the library sends an alpha
  plane, a receiver gets `RGBA`. **Verified** by the server test
  `screen_adjust::corner_pin_transparency_reaches_an_idn_receiver` (straight-alpha screen,
  solid white, pinned: the IDN receiver sees alpha 0 outside the quad, 255 white inside).
  An output's `alphaOverride: opaque` drops alpha (over black); `keyFill` is treated as
  straight today (no separate fill / key signals yet).
- → Displays: KMS scans out XRGB8888, alpha is dropped (premultiplied = over black). That is
  right for a single display; **Fill/Key on two connectors** needs new work in the display
  path (a fill frame = colour un-premultiplied over black, a key frame = alpha as grey), not
  in the render pass.
- → PNG: written with straight alpha.

**Owner questions (answered, proceeded).** (1) Per screen or per output? **Per screen** (the
brief; one pass for all outputs; a per-output adjustment would need a pass per output, later
if needed). The Oriel tabs sit on the output cards as the board shows and say that they apply
to the whole screen. (2) Pixels or normalised offsets? **Pixels** of the screen (the board),
positive outwards. (3) LUT? **Not now**; `.cube` import later in the same pass.

## Stage 1b: windowed outputs (Windows, Linux desktops). Batch 6, lane W

Most small churches run the projector from the operator's own Windows PC, so a display output
can also be **a borderless window covering the chosen monitor** (or, `fullscreen: false`, a
plain movable window for rehearsal), on Windows and on Linux with a desktop session.

- **winit 0.30** (raw-window-handle 0.6, like wgpu 30), no default features: X11 and Wayland
  loaded with dlopen, so a headless box needs neither library. **One event loop** per process
  (winit allows one), on its own thread (`with_any_thread` on Windows, X11 and Wayland),
  started on first use, never stopped; other threads send it commands through its proxy
  (list monitors, open, close). If it can't start (no desktop), the window source lists no
  monitors and `output-transports` says "no desktop session" with the reason.
- **Presenting from the shared device**: `render::present::WindowPresenter` makes the window's
  surface from the process's one instance and the renderer's device and adapter (never a
  second device; `is_surface_supported` is checked, a GL-only or wrong-GPU adapter fails with a
  clear error). The output's worker thread uploads the screen's final CPU frame (the same frame
  the IDN and KMS outputs get, so a later per-screen post-pass applies to windows too) and
  draws it letterboxed. Pacing: `Mailbox` when offered (never waits; the newest frame at the
  next vblank), else `Fifo` (only the output's worker waits; its one-slot channel drops what
  the window can't take). The engine never waits for a window; the screen clock stays master.
- **Monitors**: from winit's list: the OS name is the binding's `selector.connector`
  (`\\.\DISPLAY2` on Windows, shown as "Display 2"; the output name under X11/Wayland),
  position, current mode (the only mode listed: a borderless window never switches modes),
  main monitor. `DisplayInfo` gains `source` (`kms`/`window`/`fake`), `position`, `primary`
  (additive, optional).
- **Firm rules for output windows (owner, batch 6)**: never take keyboard focus or grab the
  pointer (on open and on reopen after hot-plug); pointer hidden only while it is over the
  window; cover only the bound monitor, never appear on another; stay out of the taskbar and
  Alt-Tab. How, per platform (`displays/window.rs`, `placement`, unit-tested):
  - Windows: *not* winit's full screen (winit activates a window created full screen); a
    borderless, topmost window at the monitor's exact rectangle, created hidden, given
    `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` (no focus on click, not in the taskbar or Alt-Tab),
    then shown with `SW_SHOWNOACTIVATE`.
  - X11: an override-redirect window at the monitor's rectangle (unmanaged: no focus, no
    taskbar, no Alt-Tab).
  - Wayland: the compositor decides; the window asks for full screen on that output (the only
    way to place it). App ID `vesper-display`, so the desktop's window rules can keep it from
    taking focus (documented limitation: GNOME may focus it when it opens).
  - A full screen window that leaves its monitor (the OS moves windows off an unplugged
    monitor) is put back over it, or hidden if the monitor is gone, at once; the 2 s hot-plug
    poll then rebuilds the output (unavailable: "not connected"; the binding stays).
  - The desktop's close button / Alt+F4 doesn't close an output window (it belongs to its
    output).
- **Choosing the source**: `--displays auto|kms|window|none` (`--no-displays` = `none`).
  `auto`: windows when the server runs in a desktop session (always on Windows; `DISPLAY` or
  `WAYLAND_DISPLAY` set on Linux), else KMS. The systemd services have no desktop session, so
  display boxes keep KMS. On Windows in session 0 (the SYSTEM startup task, a service) the
  window backend lists no displays and reports why ("no desktop session"), instead of opening
  windows on a desktop no one sees.
- **Windows packaging**: a SYSTEM task at startup cannot draw on anyone's desktop. Installer
  choice "Show screens on this PC's monitors": the task runs as the installing user at logon,
  in their session (headless at startup stays the default).
- **Oriel**: display outputs get "Show: Full screen / In a window" (window displays; KMS displays keep their Mode list instead); the picker shows each
  monitor's position and "main screen"; binding a full screen output to the primary monitor,
  or to the monitor Oriel itself is on (Oriel on the server's PC), asks for confirmation first.
  (The owner dropped the Oriel F11 change: Oriel's own F11 stays as it is.)

Owner questions (answered, work proceeded): exclusive full screen with a mode switch? **No**:
borderless at the desktop's mode (no flicker, no focus change; the screen is scaled).
Present from the GPU texture directly instead of the CPU frame? **Later**: the CPU frame is
already read back for IDN, and an upload of it is cheap and keeps one code path.

## Owner questions (answered with the recommendation, work proceeded)

1. KMS or a compositor on display boxes? **KMS** (no extra packages, same service model).
2. A desktop session on the same box as a display output? **Not supported by KMS** (the
   desktop holds DRM master); such boxes use 1b's windows. The output says why it's unavailable.
3. Should a display's mode follow the screen size by default? **No: the display's preferred
   mode**, the frame is scaled and letterboxed; the operator can pick a mode.

## Hardware test checklist (owner)

1. Ubuntu box, no desktop, one monitor on HDMI: install the `.deb`,
   `sudo systemctl disable --now vesper && sudo systemctl enable --now vesper-display`.
2. Oriel ▸ Screens ▸ a screen ▸ Add output ▸ System display: the monitor is listed with its
   model and modes. Pick it, Apply: the screen appears full screen; Identify Screens shows the
   label on it.
3. Trigger slides and a video: smooth, no tearing; IDN outputs of the same screen unchanged
   (check the frame counters in the output status).
4. Pick another mode (e.g. 1280×720), Apply: the monitor switches.
5. Unplug the cable: within 2–3 s the output shows "not connected", the server keeps running,
   IDN keeps sending. Plug it back: the picture returns by itself.
6. Two monitors (HDMI + DP), two screens: each on its own display.
7. Reboot: the screens come back on their displays without anyone logging in.
8. With a desktop session running on the box: the display output says it's unavailable (the
   desktop owns the screens); nothing crashes.
9. `journalctl -u vesper-display` for warnings.

## Hardware test checklist, windows (owner)

A. **Windows PC with a projector as the second monitor** (server installed with "Show screens on
   this PC's monitors", `docs/PACKAGING.md` steps 13–16; Oriel on the same PC, on the main
   monitor):

1. Oriel ▸ Screens ▸ the audience screen ▸ Add output ▸ System display: the picker lists
   "Display 1 … main monitor" and "Display 2", with their positions; a new output picks the
   projector, not the main monitor. Apply: the projector shows the screen edge to edge, no
   frame, no taskbar over it.
2. **Focus**: while it opens (and after every Apply), the text cursor stays where you were
   typing in another program (Notepad on the main monitor); the keyboard never goes to the
   output window. Click on the projector picture: nothing takes focus (the click does nothing).
3. **Pointer**: move the pointer onto the projector: it disappears over the picture only; move
   back: it is there again. Nothing confines the pointer.
4. **Own monitor only**: the picture never appears on the main monitor: not when it opens, not
   after Win+Shift+Arrow, not after changing the projector's resolution or arrangement in
   Display settings (the window follows the projector).
5. **Alt-Tab and the taskbar**: no "Vesper" entry in Alt-Tab or in the taskbar; Alt-Tab and
   Win+Tab work normally with the output up; the taskbar on the main monitor stays usable.
6. **Main monitor confirmation**: on another screen, choose the main monitor: Oriel warns under
   the picker; Apply asks "Cover your own monitor?". Cancel: nothing changes. (Show it full
   screen would cover your work: try "Show: In a window" instead, which opens a movable window on
   that monitor.) Drag Oriel onto the projector and pick the projector for an output: the same
   question ("the monitor Oriel is on").
7. Show: In a window on the projector output, Apply: a normal window (title "Vesper: Display 2")
   that can be moved and resized; the picture follows its size, letterboxed. Back to Full
   screen: it covers the projector again.
8. **Hot-plug**: unplug the projector: within 2–3 s the output shows "not connected" and nothing
   appears on the main monitor; Windows' own windows move as usual. Plug it back: the picture
   returns, still without taking focus (repeat check 2 right after).
9. Play a video on that screen: smooth; IDN outputs of the same screen unchanged; the log names
   the presenting adapter and `Mailbox` or `Fifo`.
10. Sign out: the server stops (this mode runs in your session); sign in: it starts and the
    projector shows the screen again.

B. **Linux desktop** (Ubuntu with GNOME on Wayland; also an X11 session if available): run
   `vesper --data … --displays auto` from a terminal in the session (or a user unit).

1. `GET /api/v1/displays` lists the monitors with `source: "window"`; output-transports says
   "windows on this desktop". Without a session (from SSH, no `DISPLAY`), `auto` picks KMS.
2. Steps A1, A3, A6, A7, A8, A9 as above.
3. X11: the output window is unmanaged (override-redirect): no focus, not in the dock or
   Alt-Tab (A2, A5). Wayland: the compositor decides focus and the dock; note what GNOME does
   when the output opens (known limitation; a window rule on app ID `vesper-display` can help).
4. With a GPU that can't present to the session (e.g. the render GPU is not the display GPU),
   the output says why ("cannot present to this window") and nothing crashes.

## Hardware test checklist, Fill/Key (owner)

Not verifiable here (no monitors, no keyer): the pixels of both halves are tested with fake
displays (server test `displays::fill_and_key_outputs_split_a_half_transparent_picture`), the
flip pairing with threads (`displays::tests::fill_and_key_flip_together_or_alone`).

1. Display box (KMS, no desktop), two outputs of one card to a keyer's fill and key inputs
   (HDMI/DP → SDI converters if the keyer is SDI), both at the same mode.
2. Oriel ▸ Screens ▸ a screen with **Alpha: Straight key** ▸ Add output ▸ System display ▸ the
   fill connector ▸ Output Target **Fill** ▸ **Add the key output** (lands on the next free
   display: check it is the key connector) ▸ Apply. Both outputs show "Showing …".
3. Lower-third over camera on the keyer (keyer's Pre Multiplied Key **off**): text edges clean,
   no dark fringe; a 50 % box shows the camera through at half.
4. Output's Alpha Key tab ▸ **Premultiplied**, keyer's Pre Multiplied Key **on**: the same result.
   (Mismatched: dark or bright fringes, which shows the setting matters.)
5. Fades and moving text: watch for a one-frame fill/key mismatch at changes (a flash of the
   fill's black edge). Note how often; `journalctl -u vesper-display` at debug level counts
   "fill/key flips" together / alone when an output stops.
6. Unplug the key connector: the key output shows "not connected", the fill keeps running;
   plug back: both run.
7. Windows PC: the same pair as two full screen windows on two monitors (or converter
   outputs): steps 2–4.
8. An IDN output with Straight alpha into a mixer that takes IDN (compatible with NDI®) alpha:
   the same key without a pair (the recommendation).

## Stages

0. This note. **Done.**
1. Display output kind in the server: KMS sink, enumeration (sysfs + DRM ioctls), API and
   state, hot-plug, validation, persistence; tests with `FakeDisplays` (server test
   `displays::`). **Done.** Verified on this machine (AMD GPU, connectors, no monitor): the
   real enumeration lists the card's connectors as disconnected; with the renderer up the
   server process becomes the card's DRM master as a non-root `video` user (hardware probe
   `cargo test -p vesper-server --lib kms_probe -- --ignored --nocapture`); a binding to an
   unplugged connector is `unavailable`. **Not verified here:** an actual modeset and page
   flips (no monitor, and no vkms without root): the owner's checklist above.
2. Packaging (`vesper-display.service`, `.deb` hooks, PACKAGING.md) and the `portable/` fix.
   **Done** (the `.deb` and its podman smoke test were not rebuilt in this batch).
3. Oriel: display picker, mode, status, stub and shots. **Done.**
4. **1b, windowed outputs (batch 6, lane W): done** (server, packaging, Oriel; see "Stage 1b"
   and the hardware checklist for windows). Colour correction and corner pin: done (batch 6,
   lane P). The rest of G13: not started; see "Deferred". In order: an "identify this display"
   overlay for an unbound display (show the connector name full screen for a few seconds, a
   `POST /api/v1/displays/{id}/identify`); placeholders and arrangements; Fill/Key;
   vblank-locked screen clocks.

## Colour correction and corner pin: stages (batch 6, lane P)

0. Design (the section above). **Done.**
1. Colour correction and 2. corner pin, server side (one output-pass shader, so built
   together): model, render pass, engine, validation, `PUT /screens/{id}/color` and
   `/corner-pin`, capability `screenAdjust`, bindings, protocol README; render tests
   (`screen_adjust`: bit identity, colour vs a CPU reference, corners land where asked, alpha)
   and server tests (`screen_adjust::`: live, persisted, no restart, alpha at an IDN
   receiver). **Done.**
3. Oriel: Color and Corner Pin tabs on the output cards (`ColorTab.svelte`,
   `CornerPinTab.svelte`, logic in `adjust.ts` with vitest), live with a throttle, staged
   screen edits kept; stub routes; shots. **Done.**

Later: a `.cube` 3D LUT in the same pass; per-output adjustments (one pass per output); a
mesh warp / edge blend (arrangements); a grid test pattern shortcut on the Corner Pin tab.

## Resume notes (lane P, batch 6)

All three stages are committed on `feat/output-color-pin`; nothing is left in progress.

## Stage 1b status (batch 6, lane W)

- **Server: done.** `render::present::WindowPresenter`, the window source
  (`displays/window.rs`), `--displays`, truthful `output-transports`, additive `DisplayInfo`
  fields. Tests: placement rules and monitor guard (unit, headless); `fullscreen: false`
  reopening an output as a window and the `source` field (server test `displays::`).
  **Verified here:** a real present under Xvfb on the software adapter (`window_probe`, see
  its doc comment): monitor listed, 60 frames full screen and windowed. **Not verified
  here:** Windows (no Windows machine), a hardware GPU presenting to a real desktop, Wayland,
  the focus / taskbar / pointer behaviour on a real desktop: the checklist above.
- **Windows packaging: done.** Installer page "Screens" with "Show screens on this PC's
  monitors" (`/DESKTOP`, remembered), `vesper-task.ps1 -Install -Desktop [-User]`,
  `docs/PACKAGING.md` (description and checklist steps 13–16). The installer builds; it was
  not run on Windows.
- **Oriel: done.** "Show: Full screen / In a window" for window displays, positions and "main
  monitor" in the picker, a new output avoids the main monitor, a note and an Apply-time
  confirmation before covering the main monitor or the one Oriel is on. Stub: the Screens
  stub's `source = "window"` variant; shots `screens-window-*`.
- Known limits: Wayland focus is up to the compositor; a full screen window on Windows relies
  on `WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW` set after winit creates it (winit could reset the
  extended style if Vesper ever changed the window's decorations or level later; it doesn't).
