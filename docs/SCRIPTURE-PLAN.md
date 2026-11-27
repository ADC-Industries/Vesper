# Scripture (G11) and the Bible page (round 13): plan

Inputs: PRODUCTS.md (Bible), ORIEL-REDESIGN-PLAN.md 4.5, row 13, G11 and question 12, the owner's
Bible board. Status lines at the end.

## Model

- **Translation**: an ID (`KJV`), abbreviation, name, language, licence, an optional
  **attribution** line (required wording, e.g. a publisher's notice), the source it was built
  from, and its **versification**: the books it contains (USFM key, English name) with each
  chapter's verse count. Verse text is plain UTF-8, one string per verse.
- **Reference** (parsed from text): a book plus one or more ranges, each `chapter[:verse][-…]`,
  e.g. `Jn 3:16-18`, `1 Cor 13`, `Psalm 23:1–4, 6`, `Gen 1:31-2:3`. English book names and the
  usual abbreviations first (a table in the crate; other languages later). A reference is
  checked against the translation's versification (unknown chapter / verse → a clear message).
- **Generated presentation**: an ordinary `model::Presentation` with `scripture` set (so its kind
  is `scripture`), named by its label ("John 3:16–18 (KJV)"), one section per passage.
  Each slide has a text element (placeholder slot `text1`) and a reference element (slot
  `reference`); with a theme, the chosen template slide is applied (copy semantics, today's
  `Theme::apply`), so the theme is only **read**.

## Storage

- **Translation files** (`<ID>.vbible`): line 1 a JSON header (format `vesper-bible/1`, the
  fields above, versification), then one verse per line in canonical order (an empty line for a
  verse the translation doesn't have). Compact (KJV ≈ 4.4 MB), diff-able, loads in milliseconds;
  verses are indexed by offset at load, full-text search scans a lower-cased copy in memory
  (31 k verses: a few ms).
- **Where**: the bundled folder (`--scripture-dir`; packages put it in
  `/usr/share/vesper/scripture` on Linux and `<install>\scripture` on Windows; default: a
  `scripture` folder next to the UI dir, else none) plus `<data>/scripture` (a church's own
  files, later the online cache). Bundled files are per box, read-only, so the Bible **always
  works offline** on every show box without the Codex. (PRODUCTS.md says "stored in Codex":
  the texts are the same on every box, so shipping them with the server is that, without
  copying 18 MB per collection. A licensed text the church imports later goes in the Codex.)
- **Generated passages** live in the active collection like any presentation, in a library
  (shelf) named **Scripture**, created on first use (question 12's recommendation).

## API (`/api/v1/scripture/…`)

Read-only routes in `crates/vesper-scripture` (generic over `FromRef`, like `vesper-site`):

| Route | |
|---|---|
| `GET /scripture/translations` | `ScriptureTranslation[]` (no verse text) |
| `GET /scripture/translations/{id}/books` | `ScriptureBook[]`: key, name, testament, verse count per chapter |
| `GET /scripture/parse?translation=&q=` | `ScriptureParse`: the normalised label and ranges, or an error message (live feedback) |
| `GET /scripture/passage?translation=&ref=` | `ScripturePassage`: label, attribution, verses |
| `GET /scripture/search?translation=&q=&limit=` | `ScriptureSearch`: hits (all words, case-insensitive), total |

Server routes (`crates/vesper-server/src/api/scripture.rs`, they need themes, the Codex writer
and the show):

| Route | |
|---|---|
| `POST /scripture/slides` | `ScriptureSlideRequest` → `ScriptureSlides`: the split text (preview, no write) |
| `GET /scripture/preview?…&slide=&w=` | PNG of one generated slide with the chosen theme |
| `POST /scripture/presentations` | `ScriptureCreate` (the request + `then`: `none` / `playlist` / `show`) → `ScriptureCreated` |

`then: show` creates it and triggers its first slide without adding it to a playlist (question
12); `then: playlist` appends it to the given playlist.

**Split rule** (measured since batch 6, see the section at the end): as many whole verses as
fit the theme's text box, at most `versesPerSlide` (1–3 on the board; "Auto" = no bound); a verse
too long for a slide breaks between words and continues on the next (" …" / "… "). Verse numbers optional (superscript runs). The reference is on `each`
slide, the `last` one, or `none`; the attribution line (when the translation has one) is on the
last slide.

## Oriel (round 13)

The board: translation picker, book list (Old / New Testament), chapter grid; reference box
with parsing feedback, which also runs a word search when the text isn't a reference; verses as
toggles; slide options (verses per slide, reference on slide, theme), generated previews
(server PNGs); **Add to playlist** and an amber **Go live**; recent references. The selection
becomes a reference string ("Psalm 23:1–4, 6") so the server sees one format. Settings ▸ Bible:
default translation, verse numbers, slide split (Oriel preferences, like other view options).

## Deferred

Online providers (API.Bible, ESV API) with the church's key stored server-side, never sent to
Oriel, caching within each provider's terms and their attribution lines (stage 5); importing a
licensed translation into the Codex. (Non-English book names and re-splitting a passage when
its theme changes: done in batch 9, see the end.)

## Owner questions (answered with the recommendation, proceeding)

1. *Where do generated passages live; can Go live skip the playlist?* Question 12: a
   **Scripture** shelf in the active collection; Go live creates and shows without the playlist.
2. *Bundled texts in the Codex or with the server?* With the server (offline on every box);
   imports of licensed texts into the Codex later.
3. *Box-wide or per-operator Bible settings?* Per Oriel (preferences backup covers them); the
   server takes every option per request.

## Stages and status

0. This plan. **Done.**
1. `crates/vesper-scripture`: format, loading, parsing, passages, search, attribution, the
   read-only routes; unit tests on a made-up mini translation. **Done.**
2. `scripts/fetch-scripture.sh` + `vesper-scripture-convert`: KJV, ASV, WEB (ebible.org VPL)
   and BSB (bereanbible.com), pinned by URL and SHA-256, converted into
   `$CARGO_TARGET_DIR/scripture/`; the `.deb` (`/usr/share/vesper/scripture`) and the Windows
   installer (`<install>\scripture`); PACKAGING.md. **Done** (all four reachable on
   2026-11-23; the `.deb` and installer builds themselves weren't run here, only the script
   changes' syntax: a release build is lane C's ground).
3. Slide generation (`vesper_scripture::slides`), the server routes
   (`crates/vesper-server/src/api/scripture.rs`), server test `scripture::`. **Done.**
4. The Oriel Bible page, stub routes and fixtures, vitest, shots; Settings ▸ Bible. **Done.**
5. Online providers. **Not started.** What's left, precisely:
   - a provider setting per box (`PUT /scripture/providers/{apiBible|esv}` with the church's
     key; admins; stored in `machine/`, never returned to clients, only `configured: true`);
   - an `online` origin in `ScriptureTranslation`, translations listed from the provider's
     catalogue (API.Bible: the church's allowed Bibles; ESV: one);
   - passage fetches through the official APIs only, cached per box within each provider's
     terms (both limit how much may be stored and require usage reporting or notices: read the
     current terms when building it),
     search through the provider's search endpoint;
   - the provider's required notice as the translation's `attribution` (so it lands on the
     last slide automatically), and its copyright line in Oriel's passage view;
   - Oriel: Settings ▸ Bible gets a key field per provider (write-only) and a status line.

## Batch 6 (lane S): passages that fit the theme's text box

Design (stage 0, owner questions answered with the recommendation, proceeding):

- **Split by measuring.** `slides::plan` takes a *fits* check instead of counting characters:
  a candidate slide is built exactly as it will be stored (plain layout, the reference and
  attribution lines it carries, the theme's template applied) and every text element is laid
  out with the renderer's own text engine (cosmic-text shaping, the element's font, size,
  bounds, margins, line spacing) at its authored size; it fits when the block's height fits the
  box (margins off) and no line is wider. Greedy: as many **whole** verses as fit, with the
  user's `versesPerSlide` as an upper bound ("Auto" = no bound). `maxChars`, when a request
  gives it, caps the characters on top; [`chars_fit`] alone is the check of the crate's own
  tests.
- **A verse too long for one slide** breaks at a word boundary (the longest prefix that fits,
  found by binary search over word counts) and continues on the next slide, where following
  verses may join it if they fit. **Continuation mark** (recommendation): the broken part ends
  with " …" and the continuation starts with "… "; the verse number only on the first part; the
  reference shows the verse range as usual (both parts say "Genesis 1:1").
- **Never overflow, never auto-shrink.** Measurement is at fit scale 1 whatever the box says, so
  a theme's shrink-to-fit box never has to shrink; the plain layout keeps its shrink-to-fit only
  as a safety net. A single word wider than the box is the one case that can't fit: it goes on
  its own slide (logged).
- **Where the measuring runs.** Not on the render thread (a split can lay out a few hundred
  boxes; live frames must not wait): a GPU-free `render::TextMeasurer` (same font database,
  fallback family and substitution table as the renderer, built once, lazily, behind a mutex in
  the server) and `spawn_blocking` in the scripture routes. The preview (`POST
  /scripture/slides`, the PNGs) and `POST /scripture/presentations` use the same split, so the
  Bible page preview shows the slides that will be created.
- **Tests**: made-up verse text only (the crate's synthetic `TST` translation): a long verse
  breaks between words with the marks, never more verses than asked, a measured split that
  differs from the character rule (a narrow theme box), and a server test `scripture::` with a
  theme whose box fits two short lines.

## Status (batch 6, lane S)

- **Stage 0** (this section and ORIEL-REDESIGN-PLAN 4.1 "The grid in play order"): **done.**
- **Stage 1** (Show page in arrangement order: `SlidePosition.step`, `triggerSlide.step`, the
  grid in play order with repeats and a collapsed "Not in arrangement" part): **done.**
- **Stage 2** (passages that fit): **done.** `render::TextMeasurer` (GPU-free, the renderer's
  font setup shared through `text_engine`; `fits` measures at fit scale 1), built lazily in
  `Shared::measurer` from the renderer options; `vesper_scripture::slides::plan` takes a
  `Fits` check (`slide_with_text` builds the candidate as stored, the server measures the
  elements holding the passage's text), breaks a long verse between words with " …" / "… ";
  the scripture routes split on the blocking pool. Tests: render
  `the_measurer_lays_out_like_the_renderer_without_a_gpu`, crate tests (made-up text), server
  `scripture::` (the fixture's one-line lower-third theme breaks every verse; the plain layout
  keeps two a slide). Oriel: the Bible page preview shows the server's split as before (the
  stub mimics it with a character budget), Settings ▸ Bible wording.
- Left for later: none of the brief. A passage already created doesn't re-split when its
  theme changes (the linked-themes item under Deferred).

## Batch 9 (lane S): re-split on theme change, book names in other languages

Design (stage 0, owner questions answered with the recommendation, proceeding):

- **Re-split.** A passage created on the Bible page records its request and a digest of its
  sections (`scripture.generated`). A theme edit (`POST /themes/{id}/edits`) queues the theme
  for a background job on the Codex box (debounced 300 ms, one job at a time, so a drag's burst
  of edits ends in one split with the theme's last state): every passage made with that theme
  whose sections still match the digest is split again with the same request (one presentation
  edit: its sections replaced, `scripture.generated` updated; undo restores the old split).
  **The operator's edits are kept** (recommendation): an edited passage isn't touched, only
  marked `stale`, and Oriel's song view offers "Split again for the theme" (`resplit` with
  `force`, after a confirmation that the edits are replaced). Renaming or deleting a theme
  doesn't re-split (the layout is unchanged / passages keep their look). Passages made before
  batch 9 have no record and stay as they are.
- **Book names.** A table per language (`vesper_scripture::names`: `es`, `pt`, `fr`, `de`, the
  66 books in canonical order). A translation whose file names its books in English (or not at
  all) and whose `language` has a table shows them in its language (books list, parse feedback,
  labels, generated slides; "Salmo 23" for one psalm); names the file gives in its language stay.
  References parse in the translation's names, its language's table and English (with the
  usual abbreviations), accents optional. More languages: add a table.
- Tests (made-up verse text only): crate tests for the tables, localised names, parsing with and
  without accents, a file's own names kept; server test
  `scripture::passages_split_again_when_their_theme_changes` (a shrinking text box splits the
  passage again, an edited one is kept and marked stale, `resplit` 409 then `force`).

Status: **done** (stage 3 of batch 9 lane S). Not done: a per-language table for every
language of the bundled or online translations (only `es`, `pt`, `fr`, `de`), localised
abbreviations beyond unique prefixes ("Hch", "Mt" work only where they are a prefix or an
English abbreviation), and a Bible page control to pick a passage's theme again after it was
made (the passage keeps the theme it was made with).
