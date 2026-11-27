# Linked themes (G2)

Approved answer to [ORIEL-REDESIGN-PLAN](ORIEL-REDESIGN-PLAN.md) question 3: **themes are linked**.
A song follows its theme; editing the theme changes every song that uses it; a slide can
override a theme property, and "Reset to theme" clears the overrides. This note is the design;
section 8 tracks what is built.

## 1. What a theme holds

Unchanged document (`Theme`, `crates/model/src/theme.rs`): a name, folder, sort key, canvas size
and **template slides** ("Lyrics", "Title", "Scripture", …). A template slide is an ordinary
`Slide`: background colour, elements (text boxes with their style and placement; a text box with
a `placeholder` is a **slot** for a role such as lyrics, title or reference; other elements are
decoration: logos, bars, pictures), its transition, and media actions (background video). Theme
media stay `MediaAsset`s with `scope: theme`. New themes start with three template slides
(Lyrics, Title, Scripture), white text on black.

## 2. The link

- **Slide.** A slide follows its theme when it has `templateOrigin` (theme + template slide,
  existing field) **and** `themeLink` (new, `{overrides: string[]}`: slide properties it
  overrides: `background`, `transition`, and `removed:<template element id>` for a theme element
  deleted on this slide). Without `themeLink`, `templateOrigin` stays provenance only, as today.
- **Element.** An element that comes from the template has `fromTheme` (new,
  `{element, overrides}`): the template element it follows and the properties it overrides.
  Elements without it are the slide's own (extra text, pictures) and are never touched.
- **Properties** are per field, so a reset clears only what was overridden: `name`, `bounds`,
  `rotation`, `opacity`, `flip`, `hidden`, `locked`, `aspectLocked`, `shape`, `fill`, `stroke`,
  `shadow`, `feather`, `visibility`, `dataLink`, and the text box's `text.defaultStyle`,
  `text.defaultParagraph`, `text.verticalAlign`, `text.fit`, `text.fontScaleFactor`,
  `text.margins`, `text.textShadow`, `text.outline`, `text.lineMask`, `text.fill`.
  **Content** is never a property: a slot's paragraphs (with their inline formatting) and a media
  slot's picture belong to the slide.
- **Overrides are recorded by the server**: after every edit of a linked slide the server sets
  each element's `overrides` to the properties that differ from its template element (and the
  slide's to `background` / `transition` / removed theme elements). Setting a value back to the
  theme's makes it follow the theme again. Clients never write `overrides` themselves.

## 3. Resolution (theme → overrides)

`model::theme_link::resolve(slide, template)` (pure): template elements in template order, each
taking the slide element's ID, content and overridden properties (new template elements are
added with fresh IDs, empty slots; removed ones skipped); then the slide's own elements on top
in their order; a slide element whose template element is gone keeps its content as the slide's
own element, an empty one goes. Background and transition from the template unless overridden.
Label, notes, actions, hotkey, auto-advance: the slide's.

**Where it runs: on write, not per frame.** The server keeps every linked slide resolved: when a
theme changes, the same commit re-resolves every linked slide of the collection's presentations
and writes the ones that changed (one change-log entry each, `documentChanged` each, so open
songs and thumbnails re-render). Render, thumbnails, stage, search, show boxes' caches and Oriel
all keep reading plain slides, so **existing presentations and imports render exactly as before**
(they have no `themeLink`), and nothing else has to know about themes. (Owner-level choice,
recommended: resolving per frame would make every consumer theme-aware and leave a show box
dependent on the theme document at play time.)

## 4. API

- `GET /themes` (unchanged, plus `usedBy`: number of presentations following it).
- `GET /themes/{id}` → `ThemeEntry {theme, revision}` (`ETag`). Lane A reads themes the same
  way as before (`content.themes` inside the server; `GET /themes` outside).
- `POST /themes` `CreateTheme {name?, document?}` → `201 DocumentRevision` (three starter
  template slides); `POST /themes/{id}/duplicate` `{name?}` → `201`; `PUT /themes/{id}`
  `UpdateTheme {name}` (`If-Match`); `DELETE /themes/{id}` (`If-Match`): `409 inUse` with the
  users when presentations follow it, unless `?reassign=<theme>` (they follow that theme, template
  slides matched by name, else its first) or `?unlink=true` (they keep their look as copies).
  Deleted themes go to the item trash.
- **Editing a theme** works like props and stage layouts: `GET /themes/{id}/document` (a
  presentation wrapper: one section, each template slide a slide with the template's name as its
  label), `POST /themes/{id}/edits` (the presentation edit ops; slides can be added, duplicated,
  moved and deleted; sections, arrangements, split / merge and slide actions are refused),
  `GET /themes/{id}/slides/{slide}/render?w=` and `…/thumbnail?w=` (template thumbnails, G19).
  Every theme change re-resolves the songs that follow it (section 3).
- **Songs**: presentation edit ops `applyTheme {theme, template?, slides?}` (links the slides,
  all when omitted: the template slide of the same name as the slide's current one, else
  `template`, else the theme's first), `resetToTheme {slide, element?, properties?}` (clears
  overrides: the slide's and all its elements', one element's, or some properties) and
  `unlinkTheme {slides?}` (keeps the look as a copy). They are applied as a new primitive
  `replaceSlide {slide}` (its inverse is another `replaceSlide`), so undo works as for every edit.
- `GET /themes/{id}/used-by` → `DocumentUse[]` (presentations following it, messages using its
  template slides).

## 5. Imports

Imported presentations stay **unlinked** (recommended): they keep `templateOrigin` as
provenance and render exactly as before. ProPresenter copies themes into slides, so matching an
imported slide's elements to template elements is a guess; a wrong guess would change how a
real song looks. Linking is the operator's choice: "Apply theme" in the song view links (and
restyles) a song; since batch 9, "Link to theme, keeping the look" (and `pp-import
--link-themes`) links it without changing how it looks (section 9).

## 6. Storage and the Codex

Themes are collection documents (`portable/theme/<id>.json`), written by the Codex writer with
the rest of the content: one commit per change (the theme and every re-resolved presentation),
one change-log entry per document, `documentChanged` kind `theme` / `presentation`. Search keeps
indexing theme names and template names. Show boxes get theme edits as write-through edits like
other content.

## 7. Oriel

Library ▸ Design ▸ Themes becomes editable: "+ New theme", duplicate, rename, delete (with the
used-by list and a reassign choice), template thumbnails, "Edit" opens the theme in the slide
editor (like a prop). The song view shows its theme with "Change theme…" (apply) and "Reset to
theme"; the editor's inspector marks overridden properties with a dot and a per-element "Reset
to theme".

## 8. Status and what is deferred

- [x] Stage 0: this note.
- [x] Stage 1: model (`themeLink`, `fromTheme`, `model::theme_link`: resolve, link, override
  refresh, reset, unlink); model tests; unlinked slides and copy application unchanged (the
  render and import suites pass untouched).
- [x] Stage 2: API (section 4, plus trash restore, write-through, `themeEditing`), server tests
  (`tests/server/themes.rs`).
- [x] Stage 3: Oriel: Library themes view (new, duplicate, rename, delete with used-by and
  reassign / keep the look, template thumbnails, "Edit in slide editor"), the song view's theme
  picker and "Reset to theme", override markers in the inspector (Edit page and Library edit);
  stub, vitest, screenshots.
- [x] Stage 4: PRODUCTS.md, the protocol README, ARCHITECTURE, the redesign plan.

Deferred: theme media actions (background video) following the theme after linking (copied at
link time only); z-order changes of theme elements on one slide (resolution restores the
theme's order); per-group styles (the Library
board's Groups tab) and the board's sample-content preview with a fit readout; a theme's own
assets on duplicate (the copy shares them); per-field override dots inside each inspector
control (today one list per slide / object). Done in batch 9 (section 9): template slides added,
duplicated, reordered and deleted in the slide editor, slot roles, "link, keeping the look".

## 9. Batch 9 (lane S): template slides in the editor, linking imported songs

Design (stage 0; owner questions answered with the recommendation, proceeding).

**Template slides in the theme editor.** The theme document already takes `addSlide`,
`duplicateSlide`, `moveSlide`, `deleteSlide` and `setSlide {label}` (the label is the template's
name). What was missing:

- **Roles.** A template slide's role is what its text slots hold: lyrics, title, scripture,
  reference, … (`Placeholder.slot`). `setElement` takes `placeholder` (new, `null` clears: the
  box becomes decoration), so the inspector can make any text box a slot or a plain one. The
  editor's "+" offers the starter layouts (Lyrics, Title, Scripture with its reference,
  Blank), built like `themes::starter` on the theme's canvas.
- **A slot that stops being a slot** must not overwrite the songs' text with the template's
  sample text: resolution keeps the slide's paragraphs as an override when the element held
  them as a slot before (`text.paragraphs` recorded), so nothing a song shows is lost.
- **Delete** of a template slide that songs follow: they keep their look, unlinked (today's
  rule, now tested and shown before deleting). `GET /themes/{id}/used-by?template=<id>`
  (new) lists the presentations with a slide following that template slide, so the editor can
  say who is affected. Undoing the delete brings the template slide back; songs unlinked by the
  delete stay unlinked (relinking is "Apply theme").
- Oriel: in theme mode the slide list gets "+" (role menu), right-click Duplicate / Delete
  (with the affected songs), drag to reorder; the inspector shows the text box's slot role.

**Imported songs linked to a theme, keeping their look.** `model::theme_link::link_keeping_look`:

- **Matching**, per slide and template slide: slide elements bottom-up are paired with
  template elements in template order: text slots by slot name, other elements by name and
  kind. Pairing stops at the first element that can't be paired in order (resolution puts
  theme elements first and the slide's own on top, so this keeps the z-order). Unpaired
  template elements are recorded as `removed:` (they don't appear), the rest of the slide's
  elements stay its own.
- The link records every difference as an override (`refresh_overrides`), then the result is
  **verified**: `resolve(linked)` must equal the slide (ignoring the link fields). A paired
  element whose resolution differs (a field outside the property list) is made the slide's own
  and the check repeats; a slide that can't be verified, or keeps no paired element, stays
  unlinked. So nothing on screen changes.
- **Closest theme**: the slide's `templateOrigin` theme and template slide when the collection
  has them (imports keep that provenance), else the template slide with the most paired
  elements and then the fewest overrides, over every theme (or the one asked for).
- **Where**: `pp-import --link-themes` links the bundle's presentations against the bundle's
  themes; for existing songs the presentation op `linkTheme {theme?, slides?}` (new; applied as
  `replaceSlide`, so undo works) and Oriel's song view "Link to theme, keeping the look".
- **Golden test**: a synthetic song with styled slots, decoration and an own element renders
  byte-identical PNGs before and after linking (server test), plus model tests on the
  matching rules.

Status (batch 9, lane S):

- [x] Stage 0: this section.
- [x] Stage 1: `setElement {placeholder}`, a cleared role keeps the songs' text,
  `used-by?template=`; Oriel theme mode ("+" with the starter layouts, roles under each slide,
  Duplicate / Delete naming who follows, drag to reorder, "Role in the template"); server test
  `themes::template_slides_roles_and_delete`, model test, vitest, shots
  `page-edit-theme-add` / `-role` / `-delete`.
- [x] Stage 2: `model::theme_link::link_keeping_look` / `link_to_closest` (model tests on
  pairing, z-order, removed elements, closest), `linkTheme {theme?, slides?}`, `pp-import
  --link-themes` (`ImportBundle::link_themes_keeping_look`, unit test), the song view's "Link to
  theme, keeping the look"; golden server test `themes::link_keeping_the_look_renders_identically`
  (byte-identical `/render` PNGs before and after linking, then a theme edit restyles the linked
  slides only); shots `page-library-song-link-keep-look` / `-linked`.
- Not measured: how many slides of a real import link (the church's backup isn't used here);
  run `pp-import --link-themes` on a scratch copy to see the count it prints.
