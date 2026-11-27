# Object builds (G18)

Batch 9, lane R. Design note for gap G18 of [ORIEL-REDESIGN-PLAN](ORIEL-REDESIGN-PLAN.md)
(section 4.3, the slide editor's Build tab: "Build order", one row per object with its
trigger and effect). Builds reuse lane T's transitions ([TRANSITIONS-PLAN](TRANSITIONS-PLAN.md)).
Status at the end.

## 1. Model

`Slide.builds: Build[]` (`crates/model/src/build.rs`, omitted when empty): the slide's build
**order**, one entry per build of an element, in the order they play.

| Field | |
|---|---|
| `element` | the element it moves (`ElementId`) |
| `mode` | `in` (hidden until it builds in) or `out` (shown until it builds out) |
| `start` | `onNext` (default), `withPrevious`, `afterPrevious`, `withSlide` |
| `delay` | seconds after its start point (default 0) |
| `transition` | the shared `model::Transition`: kind, duration, easing |

An element can have one `in` and one `out` build. A build that names an element the slide no
longer has is ignored (an undone element delete brings it back); `setSlide` keeps such builds (the
editor sends them back with the order, and undo restores them), at most 1000 builds a slide.

**Steps.** The order splits into steps: step 0 is every `withSlide` build plus the
`withPrevious` / `afterPrevious` builds before the first `onNext`; each `onNext` build opens the
next step, and the `withPrevious` / `afterPrevious` builds after it join that step. Within a
step, an `onNext` / `withSlide` build starts at the step's start plus its delay, a
`withPrevious` one at the previous build's start plus its delay, an `afterPrevious` one at the
previous build's end plus its delay.

**What an element shows** (`model::build::frame(slide, step, t0, t)`, pure, unit tested): with
`step` onNext steps taken, every build of a step before the current one is finished, the
current step's builds are at their own progress, later ones not started. An `in` build not
started hides its element, an `out` build finished hides it. In motion:

- `dissolve` (and cut, fade through colour, vendor kinds): the element's opacity;
- `push` / `slide`: the element moves in from beyond the canvas edge (in the direction of
  motion) to its place, or out the same way, at full opacity;
- `zoom`: the element fades and scales about its centre (bounds scaled, text with it is
  laid out again; `in` grows from 0, `out` shrinks from 2×);
- `wipe`: plays as a dissolve in this version (an element clip is future work).

## 2. Engine (Next steps through builds)

`ShowState.build: Option<BuildState { layer, slide, step, steps, at }>` follows the slide
that went live (`go`): `step` 0, `steps` its number of onNext steps. `next` takes the next
build step while one remains (no layer change: the layer keeps its picture and only the
elements move); `prev` takes one back (the elements cut back to the earlier step). Past the
last step, `next` goes to the next slide as before; at step 0 `prev` goes to the previous
slide. Look-ahead (`cursor`, prefetch) ignores builds. Triggering a slide (by click, hotkey,
playlist) starts it at step 0.

Render: the screen plan carries the build step and its start; the render thread's slide
content keeps them (a new step updates the track's content in place, no layer transition) and
draws `frame(...)` at each frame's time. A screen renders every tick while a build moves, and
once more when it settles; the drawn slide's content hash is the picture fingerprint. Nothing
changes in `crates/render`: a build frame is an ordinary slide.

**Stage.** The current-slide picture shows the live slide at its build step (settled); the
next-slide picture is the next *slide* (as it looks when triggered: step 0 settled).
Thumbnails, `/render` and the editor show every element (as authored).

## 3. API and Oriel

- `Slide.builds` is part of the slide document (Codex collections, change log, bindings);
  `setSlide` gains `builds` (the whole list). README updated.
- The show state gains `build: {layer, slide, step, steps} | null`.
- Oriel: the editor's **Build** tab lists the slide's builds in order (element, In/Out,
  trigger, effect with direction, duration, delay), move up/down, remove, "Add build" for the
  selected element, and a Preview that plays the slide's builds in the editor canvas (client
  side, dissolve and move). The Show page shows "Build 2 / 3" on the live slide's tile and in
  the preview header.

## 4. Imports

`pro-import` maps each element's `buildIn` / `buildOut` (start, delay, transition through
the shared transition mapping) and orders them by the slide's `elementBuildOrder` (builds it
does not list follow in element order). `childBuilds` (per-paragraph builds) and the text
reveal stay in `import.unmapped`.

## 5. Other items of this lane

- Video element sound: see TRANSITIONS-PLAN, "Video element sound".
- Masks on stage screens and the transparent keyhole: see LOOKS-MASKS-PLAN, "Stage masks and
  the transparent keyhole".

## 6. Owner questions (answered with a recommendation)

- *Does Previous replay a build backwards?* No: it cuts back to the earlier step (the source
  program does the same for most effects); Next always animates.
- *Stage "next" during builds?* The next slide, not the next build step: the performers see
  what comes after this slide.

## 7. Status

| Stage | | State |
|---|---|---|
| 0 | This plan | done |
| 1 | Builds: model (`crates/model/src/build.rs`), engine stepping (`show.rs` `BuildState`, `next` / `prev`), drawing (`engine.rs`: the track's slide content follows the step, `model::build::frame` per frame), stage pictures, import, `setSlide {builds}`, `state.build`; tests (model unit tests, an import test, the server test `builds::`); Oriel Build tab (`InspBuild.svelte`, `builds.ts`), step preview in the canvas, "Build n/m" on the Show page's live tile; stub, vitest, shots | done |

| 2 | Sound for video elements | **not built**: planned in TRANSITIONS-PLAN, "Video element sound" (move element players to the actor's registry with a synced clock and a slide-group audio source; `FillKind::Media.audio`; a mute and level in the Shape tab) |
| 3 | Masks on stage screens (`Screen.mask`) and the transparent keyhole | done: LOOKS-MASKS-PLAN section 9 (render and server tests; no Oriel picker yet) |

Touched outside the new files (stage 1): `engine.rs` (`Content::Slide::build`, `Side::Slide`'s built
frame, `Track::building`, same-key updates keep the build), `show.rs` (`ShowState::build`,
`live_build`, `build_step`, `ScreenPlan::builds`), `stage.rs` (`picture_source`); nothing in
`crates/render`.

Not done (later): Previous replaying a build backwards; per-paragraph text builds
(`childBuilds`, kept in `import.unmapped`); a wipe clip per element (plays as a dissolve);
builds on props, messages and theme-override renders (they show every element); the editor's
Preview plays the motion (it shows each step settled).
