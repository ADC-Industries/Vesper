<script lang="ts">
  // Oriel menu ▸ Import ▸ PowerPoint / Keynote / Google Slides…: choose a .pptx (Keynote and
  // Google Slides: export as .pptx first), its name and library; the import runs on the Codex
  // box with its progress here, then the report: slide by slide, what is editable and what
  // came in as a picture of the slide (state/slidesImport, server/slidesImport).
  import FileUp from "lucide-svelte/icons/file-up";
  import Info from "lucide-svelte/icons/info";
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import { Button, Dialog, Field, Select } from "../ui";
  import { EXPORT_HINT, PPTX_ENDINGS, progressFraction, progressText, slideNotes, slideStatus, summary } from "../server/slidesImport";
  import { DEFAULT_LIBRARY, slidesImport as s } from "../state/slidesImport.svelte";
  import { libraryState } from "../state/library.svelte";

  let input = $state<HTMLInputElement | null>(null);
  let dragging = $state(false);

  let libraries = $derived([
    { value: DEFAULT_LIBRARY, label: "Imported Slides (made when missing)" },
    ...libraryState.libraries.filter((l) => l.name.trim().toLowerCase() !== "imported slides").map((l) => ({ value: l.id, label: l.name })),
  ]);
  let job = $derived(s.job);
  let report = $derived(job?.report ?? null);
  const accept = [...PPTX_ENDINGS, "key", "ppt"].map((e) => `.${e}`).join(",");

  function picked(files: FileList | null | undefined) {
    s.choose(files?.[0] ?? null);
  }

  function drop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    picked(e.dataTransfer?.files);
  }

  function size(n: number) {
    return n >= 1024 * 1024 ? `${(n / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(n / 1024))} KB`;
  }
</script>

<Dialog title="Import PowerPoint / Keynote / Google Slides" onclose={() => s.close()} width={680}>
  {#if !job}
    <div class="choose">
      <button
        type="button"
        class="drop"
        class:dragging
        onclick={() => input?.click()}
        ondragover={(e) => {
          e.preventDefault();
          dragging = true;
        }}
        ondragleave={() => (dragging = false)}
        ondrop={drop}
      >
        <FileUp size={28} strokeWidth={1.5} />
        {#if s.file}
          <span class="fname">{s.file.name}</span>
          <span class="meta">{size(s.file.size)} · click to choose another</span>
        {:else}
          <span class="fname">Choose a .pptx file…</span>
          <span class="meta">or drop it here</span>
        {/if}
      </button>
      <input bind:this={input} type="file" {accept} hidden onchange={(e) => picked(e.currentTarget.files)} />
      <p class="hint"><Info size={14} /> <span>{EXPORT_HINT}</span></p>
      {#if s.problem}<p class="problem" role="alert"><TriangleAlert size={14} /> <span>{s.problem}</span></p>{/if}
      <div class="fields">
        <Field label="Name" for="pptx-name"><input id="pptx-name" bind:value={s.name} placeholder="The file's name" /></Field>
        <Field label="Library" for="pptx-library"><Select id="pptx-library" options={libraries} bind:value={s.library} /></Field>
      </div>
      <p class="note">
        Text, pictures, backgrounds and simple shapes stay editable. Charts, tables, SmartArt and other shapes come in as a picture of the slide in
        their place when the server has LibreOffice; animations and video are not imported.
      </p>
    </div>
  {:else if !report && job.state !== "failed"}
    <div class="progress" aria-live="polite">
      <p class="pname">{job.fileName}</p>
      <div class="bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progressFraction(job) * 100)}>
        <span style:width={`${Math.round(progressFraction(job) * 100)}%`}></span>
      </div>
      <p class="meta">{progressText(job)}</p>
      <p class="note">The import runs on the server: you can close this window, Oriel says when it's done.</p>
    </div>
  {:else if job.state === "failed"}
    <p class="problem" role="alert"><TriangleAlert size={14} /> <span>{job.fileName}: {job.error ?? "the import failed"}</span></p>
  {:else if report}
    <div class="report">
      <p class="summary"><strong>{report.name}</strong> · {summary(report)}</p>
      {#if report.rendererProblem && report.slides.some((x) => (x.approximated?.length ?? 0) > 0)}
        <p class="warn"><TriangleAlert size={14} /> <span>Slide pictures: {report.rendererProblem}. Install LibreOffice on the server to bring charts and tables in as pictures.</span></p>
      {/if}
      <ol class="slides" aria-label="Imported slides">
        {#each report.slides as sl (sl.number)}
          {@const st = slideStatus(sl)}
          {@const notes = slideNotes(sl)}
          <li>
            <span class="num">{sl.number}</span>
            <span class="title" class:untitled={!sl.title}>{sl.title || "Untitled slide"}{#if sl.hidden}<em class="hid">(hidden)</em>{/if}</span>
            <span class="status {st.tone}">{st.text}</span>
            {#if notes.length}
              <ul class="notes">
                {#each notes as n, i (i)}<li>{n}</li>{/each}
              </ul>
            {/if}
          </li>
        {/each}
      </ol>
    </div>
  {/if}

  {#snippet footer()}
    {#if !job}
      <Button onclick={() => s.close()}>Cancel</Button>
      <Button variant="primary" disabled={!s.file || !!s.problem || s.busy} onclick={() => void s.start()}>{s.busy ? "Sending…" : "Import"}</Button>
    {:else if !report && job.state !== "failed"}
      <Button onclick={() => s.close()}>Close</Button>
    {:else if job.state === "failed"}
      <Button onclick={() => s.close()}>Close</Button>
      <Button variant="primary" onclick={() => s.reset()}>Try another file</Button>
    {:else}
      <Button onclick={() => s.reset()}>Import another</Button>
      <Button variant="primary" onclick={() => void s.openResult()}>Open presentation</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .choose,
  .progress,
  .report {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .drop {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 22px 16px;
    border: 1.5px dashed var(--line-strong);
    border-radius: 10px;
    background: var(--surface-sunk);
    color: var(--text-2);
    cursor: pointer;
  }
  .drop:hover,
  .drop.dragging {
    border-color: var(--accent-ring);
    color: var(--text);
  }
  .fname {
    font-size: var(--fs-md);
    font-weight: 600;
    color: var(--text);
  }
  .meta {
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .hint,
  .note,
  .problem,
  .warn {
    display: flex;
    gap: 6px;
    align-items: flex-start;
    margin: 0;
    font-size: var(--fs-sm);
    line-height: 1.45;
    color: var(--text-2);
  }
  .hint :global(svg),
  .problem :global(svg),
  .warn :global(svg) {
    flex: none;
    margin-top: 2px;
  }
  .problem {
    color: var(--danger-text);
  }
  .warn {
    color: var(--live);
  }
  .note {
    display: block;
    font-size: var(--fs-xs);
  }
  .fields {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .pname {
    margin: 0;
    font-weight: 600;
  }
  .bar {
    height: 8px;
    border-radius: 4px;
    background: var(--surface-overlay);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--accent);
    transition: width 0.3s ease;
  }
  .summary {
    margin: 0;
    font-size: var(--fs-md);
  }
  .slides {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 340px;
    overflow: auto;
    border: 1px solid var(--line);
    border-radius: 8px;
  }
  .slides > li {
    display: grid;
    grid-template-columns: 32px 1fr auto;
    gap: 2px 10px;
    align-items: baseline;
    padding: 7px 12px;
    border-bottom: 1px solid var(--line);
    font-size: var(--fs-sm);
  }
  .slides > li:last-child {
    border-bottom: none;
  }
  .num {
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
    text-align: right;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hid {
    margin-left: 6px;
    color: var(--text-2);
  }
  .title.untitled {
    color: var(--text-2);
    font-style: italic;
  }
  .status {
    font-size: var(--fs-xs);
    font-weight: 600;
    padding: 1px 8px;
    border-radius: 9px;
    white-space: nowrap;
  }
  .status.ok {
    color: var(--ok-bg-text);
    background: var(--ok-bg);
  }
  .status.picture {
    color: var(--info-text);
    background: rgba(122, 167, 240, 0.16);
  }
  .status.warn {
    color: var(--live);
    background: var(--live-soft);
  }
  .notes {
    grid-column: 2 / -1;
    margin: 2px 0 0;
    padding-left: 16px;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
</style>
