<script lang="ts">
  // The ChordPro dialog (batch 16). Import: a song from a ChordPro file (.cho, .chordpro, .pro)
  // or pasted text becomes a new presentation in the selected library (sections "Verse 1",
  // "Chorus"…, chords on the lines, the key; the server reads it, `POST /chordpro/parse`), then
  // opens in the editor. Add chords: the file's chords go on the open song's lines with the
  // same words (`applyChordPro`), undoable like any edit. A two-language file (Second language:
  // the convention its translation uses; Auto reads what the file says, e.g. Vesper's own
  // export) gives each slide its second-language text, chords on the first language.
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Field from "../ui/Field.svelte";
  import { shell } from "../state/shell.svelte";
  import { session } from "../state/session.svelte";
  import { libraryState } from "../state/library.svelte";
  import type { ChordProParsed, SecondSplit } from "$protocol";

  const req = $derived(shell.chordPro!);
  let text = $state("");
  let perSlide = $state(0);
  /** How the file writes a second language (`model::chordpro::SecondSplit`). */
  let second = $state<"auto" | "none" | "alternating" | "line" | "slash" | "bar" | "comments">("auto");
  let split = $derived<SecondSplit>(second === "slash" ? { type: "inline", separator: "/" } : second === "bar" ? { type: "inline", separator: "|" } : { type: second });
  let busy = $state(false);
  let error = $state<string | null>(null);
  let preview = $state.raw<ChordProParsed | null>(null);

  const SAMPLE = "{title: Song Title}\n{key: G}\n\n{start_of_verse}\n[G]Words of the [C]first line\n[Em]Words of the [D]second line\n{end_of_verse}\n\n{start_of_chorus}\n[C]Chorus [G/B]words\n{end_of_chorus}";

  /** Stanzas split into slides whose second lines didn't pair line by line (all on the first slide). */
  const unpairedNote = (p: ChordProParsed) =>
    p.unpairedStanzas ? ` In ${p.unpairedStanzas} stanza${p.unpairedStanzas === 1 ? "" : "s"} the second-language lines don't pair with the lines, so they stay on the stanza's first slide: check ${p.unpairedStanzas === 1 ? "it" : "them"}.` : "";
  const secondCode = (p: ChordProParsed) => (p.presentation.sections ?? []).flatMap((s) => s.slides ?? []).find((s) => s.translation?.language)?.translation?.language ?? "";

  function close() {
    shell.chordPro = null;
  }

  async function openFile(e: Event) {
    const f = (e.currentTarget as HTMLInputElement).files?.[0];
    if (!f) return;
    text = await f.text();
    await check();
  }

  // The preview stays until the new one is in: clearing it first moved the footer under the
  // pointer when the text area lost focus to a click on Cancel (the click then missed).
  async function check() {
    const conn = session.conn;
    if (!conn || !text.trim() || req.mode !== "import") {
      preview = null;
      error = null;
      return;
    }
    try {
      preview = await conn.rest.parseChordPro({ text, linesPerSlide: perSlide || undefined, split });
      error = null;
    } catch (err) {
      preview = null;
      error = err instanceof Error ? err.message : String(err);
    }
  }

  async function run() {
    const conn = session.conn;
    if (!conn || !text.trim()) return;
    busy = true;
    error = null;
    try {
      if (req.mode === "import") {
        const parsed = await conn.rest.parseChordPro({ text, linesPerSlide: perSlide || undefined, split });
        const library = libraryState.libraryId ?? libraryState.libraries[0]?.id ?? null;
        const r = await session.edit("Import ChordPro", (e) => e.importPresentation(parsed.presentation, library));
        if (!r) return;
        close();
        if (library) await libraryState.selectLibrary(library);
        await session.openPresentation(r.id);
        shell.setMode("edit");
        conn.pushNotice("info", `Imported “${parsed.presentation.name}”: ${parsed.slides} slides, ${parsed.chordLines} lines with chords${parsed.secondLanguageSlides ? `, ${parsed.secondLanguageSlides} with a second language` : ""}${parsed.capo ? ` (the file says capo ${parsed.capo}: set it on a guitarist's stage layout)` : ""}.${unpairedNote(parsed)}`);
      } else {
        const ok = await session.edit("Add chords", (e) => e.editPresentation(req.presentation, "Add Chords", [{ type: "applyChordPro", text }]));
        if (ok === null) return;
        close();
      }
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title={req.mode === "import" ? "Import a ChordPro Song" : "Add Chords from ChordPro"} onclose={close} width={620} escape>
  <div class="cp" data-testid="chordpro-dialog">
    <p class="intro">
      {#if req.mode === "import"}
        Open a ChordPro file (.cho, .chordpro, .pro) or paste the song. Sections become groups (Verse 1, Chorus…), the chords sit over their words, and <code>{"{key}"}</code> sets the song's key.
      {:else}
        Paste the song in ChordPro (or open the file): every line of this song with the same words gets that line's chords. Lines that match nothing keep theirs.
      {/if}
    </p>
    <div class="row">
      <label class="file">
        <input type="file" accept=".cho,.chordpro,.chopro,.crd,.pro,.txt,text/plain" onchange={openFile} data-testid="chordpro-file" />
        <span>Open File…</span>
      </label>
      {#if req.mode === "import"}
        <label class="per">
          Slides
          <select bind:value={perSlide} onchange={() => void check()} data-testid="chordpro-lines">
            <option value={0}>One per stanza</option>
            <option value={2}>2 lines each</option>
            <option value={4}>4 lines each</option>
          </select>
        </label>
        <label class="per" title="How the file writes a second language (a translation): its lines become each slide's second-language text">
          Second language
          <select bind:value={second} onchange={() => void check()} data-testid="chordpro-second">
            <option value="auto">Auto (as the file says)</option>
            <option value="none">None</option>
            <option value="alternating">Lines alternate</option>
            <option value="line">After a --- line</option>
            <option value="slash">In each line, after /</option>
            <option value="bar">In each line, after |</option>
            <option value="comments">Italic comments {"{ci: …}"}</option>
          </select>
        </label>
      {/if}
    </div>
    <Field label="ChordPro text">
      <textarea bind:value={text} rows="12" spellcheck="false" placeholder={SAMPLE} onblur={() => void check()} onkeydown={(e) => e.key !== "Escape" && e.stopPropagation()} data-testid="chordpro-text"></textarea>
    </Field>
    {#if preview}
      <p class="ok" data-testid="chordpro-preview">
        “{preview.presentation.name}”: {preview.sections} section{preview.sections === 1 ? "" : "s"} ({preview.presentation.sections?.map((s) => s.name).join(", ")}), {preview.slides} slide{preview.slides === 1 ? "" : "s"}, {preview.chordLines} line{preview.chordLines === 1 ? "" : "s"} with chords{preview.secondLanguageSlides ? `, ${preview.secondLanguageSlides} slide${preview.secondLanguageSlides === 1 ? "" : "s"} with a second language${secondCode(preview) ? ` (${secondCode(preview)})` : ""}` : ""}{preview.presentation.songInfo?.originalKey ? `, key ${preview.presentation.songInfo.originalKey}` : ""}.{unpairedNote(preview)}
      </p>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </div>
  {#snippet footer()}
    <Button variant="ghost" onclick={close}>Cancel</Button>
    <Button variant="primary" disabled={busy || !text.trim()} onclick={() => void run()} data-testid="chordpro-run">{req.mode === "import" ? "Import" : "Add Chords"}</Button>
  {/snippet}
</Dialog>

<style>
  .cp {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .intro {
    margin: 0;
    color: var(--text-muted);
    font-size: 13px;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 16px;
  }
  .file input {
    display: none;
  }
  .file span {
    display: inline-block;
    padding: 4px 10px;
    border: 1px solid var(--line);
    border-radius: 4px;
    cursor: pointer;
    font-size: 13px;
  }
  .per {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12.5px;
    resize: vertical;
  }
  .ok {
    margin: 0;
    font-size: 13px;
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: 13px;
  }
</style>
