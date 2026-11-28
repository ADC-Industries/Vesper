<script lang="ts">
  // Import ▸ Song from SongSelect… (and Reflow's smart paste): songs a church's subscriber
  // downloaded from CCLI SongSelect (lyrics .txt, ChordPro on Premium, SongShow Plus .usr / .bin),
  // several files at once or dropped here, or SongSelect lyrics pasted as text. The server reads
  // each (`POST /songselect/parse`): sections as groups, the title, writers, CCLI number,
  // copyright and key as the song's metadata (its copyright line then shows under the words when
  // the show's CCLI line is on). The list previews every song before anything is created; a song
  // whose CCLI number is already in the library is flagged: skip it, replace that song (same ID,
  // so playlists keep it) or keep both. Vesper never contacts CCLI (edit/songselect.ts).
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Field from "../ui/Field.svelte";
  import Segmented from "../ui/Segmented.svelte";
  import Upload from "lucide-svelte/icons/upload";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import { shell } from "../state/shell.svelte";
  import { session } from "../state/session.svelte";
  import { libraryState } from "../state/library.svelte";
  import { importLabel, importPlan, importSummary, itemFor, replaceOps, sectionNames, SONGSELECT_ACCEPT, SONGSELECT_MAX_BYTES, toBase64, type DuplicateAction, type SongSelectItem } from "../edit/songselect";
  import type { SongSelectParse, SongSelectParsed } from "$protocol";
  import { onMount } from "svelte";

  const req = $derived(shell.songSelect!);
  // svelte-ignore state_referenced_locally
  let mode = $state<"files" | "paste">(req.text ? "paste" : "files");
  // svelte-ignore state_referenced_locally
  let text = $state(req.text ?? "");
  let perSlide = $state(0);
  let busy = $state(false);
  let dragging = $state(false);
  let error = $state<string | null>(null);
  /** The files read (their bytes kept to read them again with other slide settings). */
  let files = $state<(SongSelectItem & { data: string })[]>([]);
  let pasted = $state<SongSelectItem | null>(null);
  let items = $derived<SongSelectItem[]>(mode === "files" ? files : pasted ? [pasted] : []);
  let ready = $derived(importPlan(items).some((p) => p.kind !== "skip"));
  let licence = $derived(items.map((i) => i.parsed?.licenseNumber).find(Boolean) ?? null);
  let seq = 0;

  const SAMPLE = "Song Title\n\nVerse 1\nFirst line of the verse\nSecond line of the verse\n\nChorus\nThe chorus line\n\nWriter One | Writer Two\nCCLI Song # 1234567\n© 2020 Example Music\nFor use solely with the SongSelect® Terms of Use. All rights reserved.\nCCLI License # 7654321";

  function close() {
    shell.songSelect = null;
  }

  const body = (b: SongSelectParse): SongSelectParse => ({ ...b, linesPerSlide: perSlide || undefined });

  async function read(b: SongSelectParse): Promise<{ parsed: SongSelectParsed | null; error: string | null }> {
    const conn = session.conn;
    if (!conn) return { parsed: null, error: "Not connected" };
    try {
      return { parsed: await conn.rest.parseSongSelect(body(b)), error: null };
    } catch (err) {
      return { parsed: null, error: err instanceof Error ? err.message : String(err) };
    }
  }

  async function addFiles(list: FileList | File[] | null | undefined) {
    const picked = [...(list ?? [])];
    if (!picked.length) return;
    mode = "files";
    for (const f of picked) {
      const key = `f${++seq}`;
      if (f.size > SONGSELECT_MAX_BYTES) {
        files = [...files, { ...itemFor(key, f.name, null, "The file is too large for a song"), data: "" }];
        continue;
      }
      const data = toBase64(new Uint8Array(await f.arrayBuffer()));
      const r = await read({ data, fileName: f.name });
      files = [...files, { ...itemFor(key, f.name, r.parsed, r.error), data }];
    }
  }

  async function rereadAll() {
    const next = [];
    for (const f of files) {
      if (!f.data) {
        next.push(f);
        continue;
      }
      const r = await read({ data: f.data, fileName: f.source });
      next.push({ ...f, parsed: r.parsed, error: r.error });
    }
    files = next;
    await readPaste();
  }

  async function readPaste() {
    if (!text.trim()) {
      pasted = null;
      return;
    }
    const r = await read({ text });
    pasted = itemFor("paste", "Pasted text", r.parsed, r.error);
  }

  function setAction(key: string, action: DuplicateAction) {
    if (pasted?.key === key) pasted = { ...pasted, action };
    files = files.map((f) => (f.key === key ? { ...f, action } : f));
  }

  function remove(key: string) {
    files = files.filter((f) => f.key !== key);
  }

  async function run() {
    const conn = session.conn;
    if (!conn || !ready) return;
    busy = true;
    error = null;
    const library = libraryState.libraryId ?? libraryState.libraries[0]?.id ?? null;
    let made = 0;
    let replaced = 0;
    let skipped = 0;
    let failed = items.filter((i) => !i.parsed).length;
    let last: string | null = null;
    try {
      for (const step of importPlan(items)) {
        const p = step.item.parsed!;
        if (step.kind === "skip") {
          skipped++;
          continue;
        }
        if (step.kind === "replace" && step.target) {
          // Read again styled like the song it replaces, then swap its words and metadata in.
          const src = files.find((f) => f.key === step.item.key);
          const again = await read(src?.data ? { data: src.data, fileName: src.source, like: step.target } : { text, like: step.target });
          const existing = await conn.rest.presentation(step.target);
          const r = await session.edit("Replace Song", (e) => e.editPresentation(step.target!, "Replace Song", replaceOps(existing, (again.parsed ?? p).presentation)));
          if (r === null) {
            failed++;
            continue;
          }
          replaced++;
          last = step.target;
          continue;
        }
        const r = await session.edit("Import SongSelect", (e) => e.importPresentation(p.presentation, library));
        if (!r) {
          failed++;
          continue;
        }
        made++;
        last = r.id;
      }
      close();
      if (library) await libraryState.selectLibrary(library);
      // One song: open it, like the ChordPro import.
      if (made + replaced === 1 && last) {
        await session.openPresentation(last);
        shell.setMode("edit");
      }
      conn.pushNotice(failed ? "error" : "info", importSummary(made, replaced, skipped, failed));
    } catch (err) {
      error = err instanceof Error ? err.message : String(err);
    } finally {
      busy = false;
    }
  }

  /** Files handed over (the SongSelect Downloads watcher's notice): read like picked ones. */
  async function addData(list: { name: string; data: string }[]) {
    mode = "files";
    for (const f of list) {
      const key = `f${++seq}`;
      const r = await read({ data: f.data, fileName: f.name });
      files = [...files, { ...itemFor(key, f.name, r.parsed, r.error), data: f.data }];
    }
  }

  // Opened with pasted lyrics (Reflow's smart paste) or a downloaded file: preview at once.
  onMount(() => {
    if (text.trim()) void readPaste();
    if (req.files?.length) void addData(req.files);
  });

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    void addFiles(e.dataTransfer?.files);
  }
</script>

<Dialog title="Import Songs from SongSelect" onclose={close} width={720} escape>
  <div class="ss" data-testid="songselect-dialog">
    <p class="hint">Download songs from SongSelect (lyrics .txt, or ChordPro on Premium), then import them here. SongShow Plus files (.usr) work too. Vesper doesn't connect to CCLI.</p>
    <div class="row">
      <Segmented label="Import from" size="sm" options={[{ value: "files", label: "Files" }, { value: "paste", label: "Paste text" }]} bind:value={mode} />
      <label class="per">
        Slides
        <select bind:value={perSlide} onchange={() => void rereadAll()} data-testid="songselect-lines">
          <option value={0}>One per stanza</option>
          <option value={2}>2 lines each</option>
          <option value={4}>4 lines each</option>
        </select>
      </label>
    </div>
    {#if mode === "files"}
      <div
        class="drop"
        class:dragging
        role="region"
        aria-label="Drop SongSelect files here"
        ondragover={(e) => {
          e.preventDefault();
          dragging = true;
        }}
        ondragleave={() => (dragging = false)}
        ondrop={onDrop}
        data-testid="songselect-drop"
      >
        <Upload size={18} strokeWidth={1.8} aria-hidden="true" />
        <span>Drop SongSelect files here, or</span>
        <label class="file">
          <input type="file" multiple accept={SONGSELECT_ACCEPT} onchange={(e) => void addFiles(e.currentTarget.files)} data-testid="songselect-files" />
          <span>Choose Files…</span>
        </label>
      </div>
    {:else}
      <Field label="SongSelect lyrics (copied from the song's lyrics page or a .txt file)">
        <textarea bind:value={text} rows="9" spellcheck="false" placeholder={SAMPLE} onblur={() => void readPaste()} onkeydown={(e) => e.key !== "Escape" && e.stopPropagation()} data-testid="songselect-text"></textarea>
      </Field>
      {#if !pasted && text.trim()}<p class="muted">Click outside the text to preview the song.</p>{/if}
    {/if}
    {#if items.length}
      <table class="list" data-testid="songselect-list">
        <thead><tr><th>Song</th><th>CCLI #</th><th>Sections</th><th>Duplicate</th>{#if mode === "files"}<th></th>{/if}</tr></thead>
        <tbody>
          {#each items as it (it.key)}
            {@const p = it.parsed}
            <tr class:bad={!p} class:dup={!!p?.duplicates?.length}>
              <td class="name">
                {#if p}
                  <strong>{p.presentation.name}</strong>
                  <span class="sub">{p.presentation.songInfo?.authors ?? it.source}{p.format === "chordPro" && p.chordLines ? ` · ${p.chordLines} lines with chords` : ""}{p.format === "usr" ? " · SongShow Plus" : ""}</span>
                {:else}
                  <strong>{it.source}</strong>
                  <span class="sub err">{it.error}</span>
                {/if}
              </td>
              <td class="num">{p?.presentation.songInfo?.ccliNumber ?? "—"}</td>
              <td class="secs">{#if p}{p.sections} · <span title={sectionNames(p.presentation)}>{sectionNames(p.presentation)}</span>{/if}</td>
              <td>
                {#if p?.duplicates?.length}
                  <span class="flag" title={`Already in the library: ${p.duplicates.map((d) => d.name).join(", ")}`}><TriangleAlert size={13} strokeWidth={1.8} aria-hidden="true" />In library</span>
                  <select value={it.action} onchange={(e) => setAction(it.key, e.currentTarget.value as DuplicateAction)} aria-label={`What to do with ${p.presentation.name}`} data-testid="songselect-dup">
                    <option value="skip">Skip</option>
                    <option value="replace">Replace</option>
                    <option value="keep">Keep both</option>
                  </select>
                {:else if p}
                  <span class="muted">New</span>
                {/if}
              </td>
              {#if mode === "files"}<td><button type="button" class="x" aria-label={`Remove ${it.source}`} onclick={() => remove(it.key)}>×</button></td>{/if}
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
    {#if licence}<p class="muted">{mode === "files" ? "The files carry" : "The text carries"} CCLI Licence # {licence}. The copyright line under the words uses the licence number in Settings ▸ CCLI.</p>{/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
  </div>
  {#snippet footer()}
    <Button variant="ghost" onclick={close}>Cancel</Button>
    <Button variant="primary" disabled={busy || !ready} onclick={() => void run()} data-testid="songselect-run">{importLabel(items)}</Button>
  {/snippet}
</Dialog>

<style>
  .ss {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .hint {
    margin: 0;
    color: var(--text-meta);
    font-size: 13px;
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 16px;
  }
  .per {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  .drop {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 18px 12px;
    border: 1px dashed var(--line);
    border-radius: 6px;
    color: var(--text-meta);
    font-size: 13px;
  }
  .drop.dragging {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 10%, transparent);
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
    color: var(--text);
  }
  textarea {
    width: 100%;
    box-sizing: border-box;
    font-family: var(--font-mono, ui-monospace, monospace);
    font-size: 12.5px;
    resize: vertical;
  }
  .list {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
  }
  .list th {
    text-align: left;
    font-weight: 600;
    color: var(--text-meta);
    font-size: 12px;
    padding: 4px 6px;
    border-bottom: 1px solid var(--line);
  }
  .list td {
    padding: 6px;
    border-bottom: 1px solid var(--line);
    vertical-align: middle;
  }
  .name strong {
    display: block;
  }
  .sub {
    display: block;
    color: var(--text-meta);
    font-size: 12px;
  }
  .num {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .secs {
    max-width: 220px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .flag {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    margin-right: 6px;
    color: var(--warn);
    font-size: 12px;
  }
  .x {
    border: none;
    background: none;
    color: var(--text-meta);
    cursor: pointer;
    font-size: 16px;
  }
  .muted {
    margin: 0;
    color: var(--text-meta);
    font-size: 12.5px;
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: 13px;
  }
</style>
