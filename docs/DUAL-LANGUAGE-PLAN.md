# Dual-language slides (batch 16, lane D)

A slide can carry a **second-language text**; each screen's look decides whether it shows the
first language, the second, or both; Scripture passages can be made with **two translations**
verse by verse; and (still to come) an optional **offline machine translation** fills the second
language in a pinch. Owner decisions (2026-11-25): a second text per slide, parallel Bible
translations, auto-translate with offline models downloaded on first use, clearly labelled
"machine translation, check your licence", never saved unless the operator keeps it. Section 6
tracks what is built.

## 1. Model (`crates/model/src/language.rs`)

- **A translation layer per slide** (`Slide.translation: SlideTranslation`), not a field in
  each text box: `{language, texts: [{element, paragraphs}], layout?}`. The language is a BCP 47
  code; each text element that has a second text names it by element ID; `layout` is the slide's
  own preference where a screen says `auto`.
  - Why a layer: every slide algorithm keeps working on the primary text unchanged (template
    application, linked themes, a look's template override, builds, Reflow's primary text, the
    stage display, search); one slide has one second language; a second text has no geometry or
    box style of its own (how it looks comes from the screen and the theme at render time).
  - IDs follow the elements: duplicating a slide remaps them; applying a template keeps the
    text source's ID and moves a merged leftover's second text into the last text slot; a text
    whose element is gone is ignored when drawing.
- **Per screen** (`ScreenLook.language: LanguageDisplay`): `auto` (default: the slide's
  `layout`, else stacked), `primary`, `second` (the second text in the primary's place and
  style), `stacked` (under the primary in the same box), `sideBySide` (the box split in two,
  primary left, second right; the right half has a stable derived ID so caches and transitions
  see the same element every frame).
- **Per theme** (`Theme.secondLanguage: SecondLanguageStyle`): `sizeScale` (0.8), `italic`
  (true), `color` (pale gold), `fontFamily` (the primary's), `gap` (24 canvas px). The slide's
  `templateOrigin` theme's style is used, else its presentation's theme's, else the defaults.
- Edit op `setSlideTranslation {slide, translation}` (the whole layer; inverse: the old one; at
  most 64 element texts).
- **A presentation's own language** (owner decision, lane L 2026-11-26): `Presentation.language`
  (optional BCP 47 code, the language its text is written in; checked by `model::language_code`,
  stored in its usual case), set with `setPresentation {language}`, edited in the slide editor's
  Inspector ▸ Presentation ▸ Language (common languages, "Other…" for a code). Imports fill it
  where the source says: ChordPro `{meta: language xx}` (exported back), a Bible passage (its
  translation's language), a PowerPoint deck (`dc:language`, else the language most of its
  slides' text runs are marked with). The other library format carries no language, so its
  imports leave it unset. It is not the second language: that stays per slide.

## 2. Import

- **ProPresenter** (checked in the community schemas, `crates/pro-import/PROTO.md`): a presentation has no translation
  feature. The only translation fields are on the Bible reference (`translation_name`,
  `…_abbreviation`); ProPresenter's multi-translation Bible slides are stored as separate text
  boxes, which import as ordinary text elements. Nothing to map.
- **Pasted text** (Oriel, Reflow ▸ Second language ▸ Paste two languages): stanzas separated
  by blank lines (one slide each), the languages alternating line by line, split by a `---`
  line, or split inside each line by a separator (`/`, `|`). The stanzas fill the
  presentation's slides in order (optionally replacing the first-language text too); the
  dialog previews the pairs and says how many are left over. (`apps/control/src/lib/edit/
  language.ts`, unit tested.)
- **ChordPro** (`model::chordpro::parse_two`, built 2026-11-26): ChordPro has no translation
  feature of its own (none in the ChordPro 6 directives), so a two-language file is read with
  one of these conventions, chosen in Library ▸ Import ChordPro ▸ "Second language" (or
  "Auto": what the file says) and in "Paste two languages" (pasted ChordPro is recognised and
  read by the server with the dialog's split):
  - the paste's three splits (the TS split is the reference): **lines alternate** in each
    stanza; a **`---` line** (first language above, second below); a **separator inside each
    line** (`[G]Morning light / Luz de la mañana`, outside the chords: `[G/B]` is a chord);
  - **italic comments**: `{ci: …}` / `{comment_italic: …}` lines inside a stanza are its second
    language (the style printed bilingual chord sheets use to set a translation in italics under
    the lyrics; other ChordPro programs show them as such).
  Chords stay on the first language's lines (a chord line above a lyric line belongs to it;
  chords written on second-language lines are dropped). `{meta: second_language es}` gives the
  language code; `{meta: second_language_split comments|alternating|line|inline /}` names the
  convention for Auto (else a `---` line in a stanza means "line", else one language). A stanza
  split into several slides (lines per slide) splits its second lines the same way when they
  pair line by line, else in proportion. The paste puts the chords back on the first-language
  lines with `applyChordPro` (only when it replaces the first-language text).
  **Export ChordPro** writes each slide's second text as `{ci: …}` lines after the slide's
  lines, with both meta lines in the header, so the file reads back the same way (Auto).

## 3. Rendering

The server rewrites the live slide per screen (`model::language::show_language`) before
drawing it (and before a look's template override restyles it), so every renderer feature
(fit, outlines, line masks, shadows) applies to the second text as to the first. Stacked text
is appended as paragraphs with explicit run styles (size × `sizeScale`, italic, colour,
family) and `gap` before the first; side by side duplicates the element into two halves
(the right half keeps the element's visibility conditions and builds with it). A screen whose
look pours the slide into a template override draws side by side as stacked: a template takes
one text per slot, and a right half would land in another slot.
Thumbnails show the slide as authored (first language), as before; the Scripture preview PNG
shows a parallel passage as its layout draws it.

## 4. Parallel Bible (`crates/vesper-scripture/src/slides.rs`)

`secondTranslation` (+ `secondLayout`, side by side by default) on the slide request: the same
book (by USFM key) and ranges in the second translation; the primary decides the split; each
slide's `text1` element gets the second translation's verses (verse numbers too) as its second
text; a verse broken across slides is broken in the second in proportion of words; verses only
the second has are left out. The fits check lays each slide out as its layout draws it, in the
theme's second-language style. Reference and attribution name both; `scripture.generated`
records both, so a re-split after a theme change keeps the pair. Oriel's Bible page: "Second
translation" (any bundled or installed translation but the first) and its layout.

## 5. Machine translation (built: lane AT, 2026-11-26)

The operator flow, behind one trait (`vesper_translate::Translator { fn translate(&self, from,
to, lines) -> Result<Vec<String>> }`):

1. Reflow ▸ Second language ▸ **Translate to…** (every slide), a slide's "…" menu ▸
   **Translate this slide…**, or the slide editor's Inspector ▸ Slide ▸ Second language ▸
   **Translate to…** (the slide being edited; the same dialog, preview and Keep / Discard,
   shown in that section; another slide discards it): the **source language** (the languages
   the box's models translate from; default: the presentation's own language
   (`Presentation.language`), else what the operator last picked for it in this browser, else
   its Scripture translation's language, else English), the target language (those the models write from that source; a model not on
   the box says "downloads first"), the model, its licence and size.
2. `POST /api/v1/translate/preview {presentation, slides?, to, from?}` → a job
   (`GET /translate/jobs/{id}`: `queued` → `downloading` (bytes) → `translating` (slides) →
   `done`); its answer is the proposed second-language layer per slide, **not saved**.
3. Oriel shows the proposals in the second-language column in a "machine translation" style
   (dashed, tinted, read-only) under a banner **"Machine translation: check it, and check your
   licence allows a translation of this song"** with **Keep N slides** / **Discard**. Keep sends
   one `setSlideTranslation` per slide, the layer marked `machine: {engine, model}`; kept
   slides show an "MT" tag in Reflow until someone edits the second text (Oriel's edit sends
   the layer without the mark). Discard writes nothing.

### What was built

- **`crates/vesper-translate`** (its own crate: candle and `tokenizers` are a large build, and
  the store, tokenizer and engine test without linking the server):
  - `catalogue`: the pairs offered, each file of each model pinned by commit and SHA-256
    (table below). Every licence was read from the model card at the pinned revision.
  - `store`: **Vesper's own small model store** (owner, 2026-11-26, option (b) below):
    `<data>/cache/translate/<pair>/` (Linux package: `/var/lib/vesper/cache/translate/`;
    Windows: `%ProgramData%\Vesper\cache\translate\`), the same layout idea and status shape as
    captions' (`notDownloaded` / `downloading` + bytes / `ready` / `failed` + why). Files are
    fetched one by one into `*.part`, hashed as they arrive and renamed only when size and
    SHA-256 match; `ready.json` is written last; a failure writes `failed.txt` (so a failure
    survives a restart): a network failure keeps the files already checked (a retry goes on
    from them), a size or SHA-256 mismatch removes the pair's files; a stopped or cut-off
    download leaves the pair "not downloaded" (leftover `*.part` files are removed when the
    server starts). HTTPS with `ureq` (rustls/ring, the web's roots) with no total time limit
    (a 300–465 MB weights file on a slow church line takes as long as it takes): a request
    fails when the answer stalls for 60 s (no response headers, or no byte of the body).
    Stop, Delete and a preview's Cancel never wait on a stalled request (the pair is "not
    downloaded" at once; the stuck thread ends on its own and leaves the files alone). A
    preview waiting on a download that is stopped fails; it never starts it again. Never
    bundled in packages.
  - `spm`: **the tokenizer.** The Helsinki-NLP repositories ship SentencePiece models
    (`source.spm`, `target.spm`) and `vocab.json`, no `tokenizer.json`; candle's Marian example
    downloads tokenizers converted by a Python script. Instead Vesper reads the SentencePiece
    model itself (a small reader of the public protobuf fields: pieces, scores, types, the
    normaliser's precompiled character map) and builds the same Unigram tokenizer with
    Hugging Face's `tokenizers` crate (Apache-2.0, already a dependency of candle-core):
    precompiled normaliser, collapsed spaces, `▁` word marks. Pieces are numbered by
    `vocab.json`; decoding joins the target pieces (special pieces and `>>xx<<` left out).
    Multi-target models get their sentence-initial token (`>>por<<`, `>>pob<<`,
    `>>cmn_Hans<<`, `>>cmn_Hant<<`) from the catalogue.
  - `marian`: candle's Marian model on the CPU (`f32`; `model.safetensors` memory-mapped, else
    `pytorch_model.bin` through candle's pickle reader, which runs no Python code). One model
    loaded at a time, dropped after 5 minutes idle; each line encoded on its own and decoded
    greedily (padding banned, at most 3× the source length); a line longer than 200 pieces (a
    long Scripture paragraph) is translated in parts cut after a sentence, nothing left out. It computes on its own rayon pool
    of a quarter of the cores (1–4 threads) at the lowest thread priority (Linux nice 15,
    Windows `THREAD_PRIORITY_LOWEST`), never on the async runtime.
  - `propose` / `translate_lines`: slide structure kept: one input line → one output line
    (a paragraph's soft line breaks, U+2028 from the ProPresenter and PowerPoint imports, each
    split a line and are put back), blank lines stay blank and never reach the engine, a
    translated line never spans lines;
    every text element of the slide in one engine call; the slide's `layout` kept.
- **Server** (`crates/vesper-server/src/translate.rs`, `api/translate.rs`): the routes above
  plus `GET /translate/models`, `POST /translate/models/{id}/download|stop`,
  `DELETE /translate/models/{id}` (site admin, audited), `DELETE /translate/jobs/{id}`; one
  preview translates at a time (at most 32 open ones; ended ones make room); capability
  `translate`. Who may trigger a first-use download:
  any client that can preview (see the report's owner question).
- **Oriel:** Reflow (dialog, banner, proposal column, "MT" tags, slide menu entry); Settings ▸
  Translation (each pair with its licence and attribution, size, status with a progress bar,
  Download / Stop / Delete or Retry).
- **Tests:** store (checked download and its status after a restart; a SHA-256 mismatch →
  failed, nothing kept, remembered over a restart; a network failure keeps the checked files
  and a retry resumes; stop midway → not downloaded, no `.part`; stop never waits on a stalled
  request; no total time limit, only a stall limit, against a local HTTP server; leftovers
  cleared on open), the SentencePiece reader and tokenizer on a made-up model, the
  vocabulary, line structure and per-slide proposals with a fake engine, the catalogue's pins;
  the server routes with a fake engine and files served from memory (`translate::`); one real
  model test, ignored by default (`cargo test -p vesper-translate --release --test real_model
  -- --ignored`).

### Pairs offered (licences checked on each model card, 2026-11-26)

| Pair | Model | Revision | Licence | Download | Weights |
|---|---|---|---|---|---|
| en → es | `Helsinki-NLP/opus-mt-en-es` | `5bc4493d463c` | Apache-2.0 | 315 MB | pytorch_model.bin |
| es → en | `Helsinki-NLP/opus-mt-es-en` | `c96e2c5399eb` | Apache-2.0 | 315 MB | pytorch_model.bin |
| en → fr | `Helsinki-NLP/opus-mt-en-fr` | `dd7f6540a7a4` | Apache-2.0 | 304 MB | pytorch_model.bin |
| fr → en | `Helsinki-NLP/opus-mt-fr-en` | `c4aed37b318c` | Apache-2.0 | 304 MB | model.safetensors |
| en → de | `Helsinki-NLP/opus-mt-en-de` | `6183067f769a` | CC-BY-4.0 | 301 MB | pytorch_model.bin |
| de → en | `Helsinki-NLP/opus-mt-de-en` | `1a922f3b32a8` | Apache-2.0 | 301 MB | pytorch_model.bin |
| en → pt, pt-BR | `Helsinki-NLP/opus-mt-tc-big-en-pt` | `9f2863d807ec` | CC-BY-4.0 | 468 MB | model.safetensors |
| ko → en | `Helsinki-NLP/opus-mt-ko-en` | `e42d1f41b661` | Apache-2.0 | 315 MB | pytorch_model.bin |
| en → zh (zh, zh-Hans, zh-Hant) | `Helsinki-NLP/opus-mt-en-zh` | `408d9bc410a3` | Apache-2.0 | 315 MB | pytorch_model.bin |
| zh → en | `Helsinki-NLP/opus-mt-zh-en` | `cf109095479d` | CC-BY-4.0 | 315 MB | pytorch_model.bin |

Attribution shown with each pair (Settings ▸ Translation): "`<model>` by Helsinki-NLP (OPUS-MT,
University of Helsinki), `<licence>` licence, from `https://huggingface.co/<repo>` at revision
`<rev>`", plus the OPUS-MT citation (Tiedemann & Thottingal, EAMT 2020). CC-BY-4.0 allows use and
redistribution with attribution; Apache-2.0 likewise. Not offered: Portuguese → English (no
public OPUS-MT model) and **English → Korean**: its only OPUS-MT model,
`opus-mt-tc-big-en-ko` (CC-BY-4.0), gives nonsense (tried here: "Grace and peace to all of you"
→ unrelated words; its model card's discussions report the same), so it is left out until a
working model exists. English → Portuguese has no small model: the larger "tc-big" one is used.
NLLB stays excluded (CC-BY-NC).

### Measured (2026-11-26, this build machine: `nproc` 16; release build)

The real-model test (`tests/real_model.rs`, run with the models downloaded): en → es, four
synthetic slides of one or two lines: 1.2–2.1 s for the first slide (loading the model, ~1 s),
then **0.3–0.65 s per slide** on the engine's 4 low-priority threads, e.g. "Morning light is
breaking / over the quiet hills" → "La luz de la mañana se está rompiendo / sobre las
tranquilas colinas", "Grace and peace to all of you" → "Gracia y paz a todos ustedes", blank
lines kept. Every other pair loaded and translated (0.95–2.5 s with loading): en → pt-BR / pt
("A luz da manhã está quebrando", tc-big, safetensors), en → zh, en → de, en → fr, fr / es / zh /
ko → en. A 315 MB model downloaded here in about 4 s. Debug builds are much slower (candle
unoptimised): use a release server to judge speed.

Never slowing a live show: the engine computes on at most a quarter of the cores (1–4
threads) at the lowest thread priority, one preview at a time, off the async runtime; a model
takes ~300 MB of RAM (~470 MB for tc-big) while loaded and is dropped after 5 idle minutes.
Not measured here: frame pacing on a box playing video while translating (the GPU test boxes
were busy); worth one look on a show box before relying on it mid-service.

### Engine choice (licences, size, CPU speed)

| Option | Code licence | Model licence | Size per pair | CPU (Ryzen H, one slide) | Notes |
|---|---|---|---|---|---|
| **OPUS-MT (Marian) on `candle`** | candle Apache-2.0 / MIT; `tokenizers` Apache-2.0 | Helsinki-NLP OPUS-MT: CC-BY 4.0 for the original releases; the Hugging Face model cards of the `opus-mt-*` pairs say Apache-2.0 (check each pair before offering it) | ~300 MB fp32 (~75 MB quantized) | well under a second | **Recommended.** Pure Rust (no C++ build, same on Windows); candle ships a Marian model; one model per language pair (en→es, es→en, …), downloaded on first use; attribution shown in Settings. |
| CTranslate2 + `ct2rs` | MIT (C++ library + Rust bindings) | same OPUS-MT models, converted | ~75 MB int8 | fastest | Needs a C++/CMake build on both platforms and a converted-model download; more packaging work. |
| Firefox Translations (Bergamot) | MPL-2.0 (C++ Marian fork) | MPL-2.0 | ~15–20 MB | fast | Small models, but a C++ engine (no maintained Rust crate); fewer pairs. |
| M2M100-418M | MIT (fairseq) | MIT | ~1.9 GB | seconds | One model for all pairs, but large and slow on an iGPU box. |
| NLLB-200 | MIT code | **CC-BY-NC 4.0** | — | — | **Not usable** (non-commercial licence). |

**Recommendation:** OPUS-MT via `candle`, per-pair models fetched on first use (SHA-256
pinned, resumable downloads not needed), behind the `Translator` trait so another engine can
replace it. Only the download step is ours to redistribute: the models are fetched from their
publisher at first use, never bundled.

**Owner question (settled 2026-11-26: option (b), Vesper's own store):** captions' model store and download flow are
IDN's (`idn-captions`: `ensure_model_stop`, `MODEL_LIST`, progress), not Vesper's. Reusing
"the same store/flow" means either (a) asking the IDN maintainers for a translation
engine (`idn-translate`) or a generic "fetch this model (URL, SHA-256) into the store, with
progress and stop" API, or (b) Vesper keeping its own small model store for translation (same
folder layout and status shape as captions'). Recommendation: (a) a generic model-fetch API
from the IDN maintainers (one store, one download behaviour, Windows and Linux alike), with the
engine itself in Vesper (`candle`); (b) only if the IDN maintainers decline.

## 6. Status

- [x] Model: translation layer, per-screen display, theme style, `setSlideTranslation`
  (unit tests; templates and duplicates keep second texts).
- [x] Rendering: the server draws each screen's slide in its look's mode (GPU test: stacked,
  side by side, first language only, per screen).
- [x] Looks: "Second language" row per audience screen (Oriel).
- [x] Reflow: second-language column, language code, "Paste two languages".
- [x] Parallel Bible: server split with two translations, the Bible page's picker; preview PNGs.
- [x] Themes: `PUT /themes/{id} {secondLanguage}`; Library ▸ Themes ▸ a theme ▸ "Second
  language" (size, italic, colour, gap).
- [x] Machine translation (section 5): model store, OPUS-MT on candle, preview/keep routes,
  Reflow and Settings ▸ Translation.
- [x] ChordPro two languages: import (five conventions, Auto), "Paste two languages" accepts
  ChordPro, export keeps the second language (`{ci: …}`).
- [x] Slide editor: Inspector ▸ Slide ▸ Second language with "Translate to…"; the Translate
  dialog picks the source language.
- [x] A presentation's own language (`Presentation.language`): set in Inspector ▸ Presentation,
  filled by ChordPro, Bible and PowerPoint imports, the Translate dialog's first default.
