# Looks and masks (G3)

Gap G3 of [ORIEL-REDESIGN-PLAN](ORIEL-REDESIGN-PLAN.md) (section 4.8, the Screens & Looks board):
looks can be created, changed, duplicated, reordered and deleted, they go live with their own
transition, and masks are documents the server draws. Section 8 tracks what is built.

## 1. The look document

Unchanged `model::Look` (`crates/model/src/show.rs`), already written by the importer:

- `name`, `sortKey` (list order), `transition` (`None` = cut).
- `screens: ScreenLook[]`, one entry per **audience** screen it changes: `layers`
  (`LayerVisibility`: a missing field means shown), `templateOverride` (the theme override: the
  live slide's text re-rendered through a theme's template slide) and `mask`. A screen the look
  does not list shows every layer with no override and no mask, so a new screen needs no look
  edits, and a deleted screen is removed from every look (as today).
- The layer matrix of the canvas is this document: rows mask, messages, props, announcements,
  theme override ("Presentation" on the board), slide, media, video input; columns the audience
  screens. Stage screens are not in it (they use stage layouts).

The server normalises what it stores: entries for screens that don't exist or are stage screens
are refused (`400`, `field: screens`); an entry that is all defaults is dropped; a theme
override must name an existing theme and template slide; a mask must exist.

## 2. Storage, revisions and events

Looks are **box documents** (`portable/look/<id>.json`), like screens: they stay with the box,
not in a Codex collection, so they are not in a collection's change log. They are in the box's
settings backups (already: `portable/look`, and masks as `portable/mask`). Edits follow the
screen-configuration path (`crate::config::plan` → atomic writes → new `ShowData` → engine):
revisions are content hashes, `PUT`/`DELETE` take `If-Match` and answer `409` with `current`,
and every change sends `documentChanged {kind: "look" | "mask"}`.

## 3. API

| Route | |
|---|---|
| `GET /looks` | `Look[]` (unchanged) |
| `POST /looks` | partial look → `201 LookEntry {…look, revision}`; a free "Look N" name, last in the list |
| `GET`, `PUT`, `DELETE /looks/{id}` | `LookEntry` (`ETag`); `PUT` the whole look (`If-Match`); reorder = `PUT` with a new `sortKey` |
| `POST /looks/{id}/duplicate` | `{name?}` → `201 LookEntry`, right after the original, "<name> copy" |
| `GET /masks`, `POST /masks`, `GET`/`PUT`/`DELETE /masks/{id}`, `POST /masks/{id}/duplicate` | `MaskEntry`, same rules |

`setLook` is unchanged. `serverInfo.capabilities` gains `lookEditing` and `masks`.

## 4. Live and saved looks, transitions

**One copy (owner-level choice, recommended).** The live look is a saved look; there is no
separate "live scratch" copy as in the source program. Editing the live look changes the screens
at once. Oriel shows "Live" at the top of the list (which look is live), and the editor edits that
look; Make Live switches.

**Transition.** Making a look live (`setLook`, a slide's look action) plays the look's
`transition` on every layer whose picture changes because of the look (a layer shown or hidden,
a theme override, a mask); `None` is a cut. A saved edit of the live look plays it too. It uses
the **same `model::Transition` and the same engine path** as layer changes (`Track::update`), so
the kinds lane T adds (push, wipe, zoom, slide) apply to looks once both merge. In the engine:
`ShowState.look_change` (a `LayerChange`: seq, transition, at) is recorded by `set_look` and by a
config change of the live look; a track whose content changed without its own layer change uses
it when its seq is new for the screen. Layer changes keep their own transition.

## 5. Deleting what is in use

- **The live look** can't be deleted (`409`, "make another look live first"); a look that slide
  actions name can: those actions are then skipped and counted
  (`state.diagnostics.skippedActions`, as today for a missing target); a startup look that no
  longer exists starts as every layer (as today).
- **A mask that looks use** is refused (`409`, `current: {looks: [ids]}`) unless
  `?detach=true`, which removes it from those looks (live ones included) in the same write.

## 6. Masks

**Document** (`model::Mask`, existing, extended additively): `name`, `sortKey`, `size` (canvas),
`slide` (shapes, images, text: the existing slide model, so rectangles, ellipses, polygons and
pictures with alpha are already elements, each with its own feather), plus
- `mode`: `overlay` (default: the slide is drawn over the screen, as authored) or `keyhole`
  (the screen shows only through the slide's shapes; outside them is `color`): the "invert";
- `color` (keyhole only, default opaque black; transparent cuts the picture out for alpha
  outputs);
- `feather` (px at the mask canvas, default 0): softens the mask's edge as a whole.

**Compositor.** Per screen, after all layers (`Layer::COMPOSITE_ORDER` already documents this),
scaled from the mask canvas to the screen. The server renders the mask's slide on a transparent
target at the screen's size, `render::mask::finish_mask` feathers it (a blur of the whole mask)
and, for a keyhole, turns it into its complement (the colour where the shapes are not); the
result is cached per mask and screen size and drawn as one more image over every layer. Overlay:
the slide drawn on top. Keyhole: `out = picture·a + color·(1−a)` for an opaque colour; a
see-through colour cuts the picture out (section 9). A mask change cuts (deferred: playing it with the look's transition).

**Imports.** `pro-import` already writes masks (base slide → `slide`) and looks' `mask` links,
so imported looks show their masks now; imported masks are overlay masks (the source format's
keyhole mode lives in another message the importer doesn't read; noted, not mapped).

**Oriel.** The matrix's Mask row is a picker (None, the masks, "Edit masks…", "New mask"); a
simple mask editor: a canvas preview, add rectangle / ellipse / polygon, pick an image, move and
size by numbers, mode, colour and feather.

## 7. Deferred

Per-look "Live" scratch copies; a mask drawn under the messages layer;
`templateOverride` mapping options in Oriel (the server keeps them; Oriel sets theme and slide);
mask changes played with the look's transition (they cut); the importer's keyhole mode (the source's mask mode lives in a message it doesn't read); free
polygons and pictures in the mask editor (the server draws any slide element; the editor adds
rectangles, ellipses and regular polygons, and keeps other elements as they are); dragging
shapes on the preview (numbers only).

## 8. Status

| Stage | | State |
|---|---|---|
| 0 | This plan | done |
| 1 | Looks API (create, update, duplicate, delete, reorder), look transition in the engine; Oriel matrix editor, theme override, transition, Make Live, new / duplicate / rename / move / delete; stub, vitest, shots | done |
| 2 | Masks: model (`mode`, `color`, `feather`), storage, API, render (overlay, keyhole, feather), mask choice in the matrix, mask editor (rectangles, ellipses, polygons); render unit tests and server frame tests | done |
| 3 | Stage masks (`Screen.mask`), the transparent keyhole (section 9); Oriel's stage mask picker in Configure Screens | done |

## 9. Stage masks and the transparent keyhole (batch 9, lane R)

- **Stage masks.** Looks don't route stage screens, so a stage screen names its own mask:
  `Screen.mask` (box document, set with `PUT /screens/{id}`; refused on audience screens and
  for a missing mask, `field: mask`). The engine draws it over the stage layout exactly as a
  look's mask over an audience screen (`engine::mask_picture` picks the look's for audience
  screens, the screen's for stage screens). Deleting a mask a stage screen uses is refused
  (`409`, `current.screens`) unless `?detach=true`, which clears it from those screens in the
  same write (`MaskDeleted.screens`). A per-layout mask was the other option: layouts live in
  Codex collections while masks are box documents, so the screen (a box document too) keeps
  the reference in one store.
- **Transparent keyhole.** For a keyhole whose colour is not opaque the engine first draws an
  erase picture (`render::mask::keyhole_erase`, a new `LayerSource::Erase` drawn with the
  existing destination-out blend) of alpha `1 − a / (1 − (1 − a)·c.a)`, then the usual
  complement: together `picture·a + colour·c.a·(1−a)` for every colour alpha. With `c.a = 0`
  the screen is transparent outside the shapes (an output with alpha keys it; an opaque output
  shows its background there).
- Tests: `render::mask` unit tests (erase alpha inside / outside, opaque needs none), the
  server test `masks::stage_masks_and_transparent_keyholes` (validation, a red stage mask in
  the frame, delete refused and detached, a transparent keyhole on the alpha "Key" screen:
  picture on the left, alpha 0 on the right).
- **Oriel: stage mask picker: done** (batch 10b). Configure Screens shows a Mask picker for
  stage screens only (the role row): None, the site's masks, "Edit masks…" (the mask editor,
  as on the Looks tab). A mask deleted since shows as "Missing mask" (red); the list reloads on
  `documentChanged {kind: "mask"}` and after the mask editor's changes; a save the server
  refuses (`field: mask`) shows its message under the row. Logic in `state/screens.ts`
  (`maskChoices`, `withScreenMask`); shots `screens-stage-mask-picker`, `screens-stage-mask`,
  `screens-stage-mask-refused`.
- Not done: a mask per stage layout.

