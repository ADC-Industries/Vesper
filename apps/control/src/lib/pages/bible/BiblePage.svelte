<script lang="ts">
  // The Bible page (plan 4.5, round 13; docs/SCRIPTURE-PLAN.md): left, the translation picker,
  // the books (Old / New Testament) and the chapter grid; centre, the reference box (a reference
  // opens it, other words search on Enter) with the server's parse feedback and the recent
  // references, then the verses as toggles (Shift selects a range) or the search hits; right,
  // the slide options (verses per slide, reference, verse numbers, theme), the generated slides
  // (server pictures, with the theme) and Add to playlist / Go live. Needs the server's
  // `scripture` capability (the header button is disabled without it).
  import { onMount } from "svelte";
  import BookOpen from "lucide-svelte/icons/book-open";
  import Search from "lucide-svelte/icons/search";
  import PageFrame from "../PageFrame.svelte";
  import { Button, EmptyState, Segmented, SectionLabel, Select, Switch } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { siteState } from "../../state/site.svelte";
  import { ScriptureClient } from "../../server/scripture";
  import { bible } from "./bibleState.svelte";
  import { booksOf, previewParams, referenceBookName, SECOND_LAYOUTS, SPLITS, verseKey, type SecondLayout, type Split } from "./bible";

  const PREVIEW_W = 320;
  const TESTAMENTS = [
    { id: "old", label: "Old Testament" },
    { id: "new", label: "New Testament" },
    { id: "other", label: "Other books" },
  ] as const;
  let available = $derived(siteState.has("scripture"));

  onMount(() => {
    if (available) void bible.load();
  });

  // Live parse feedback (debounced) and the split preview after every change.
  let parseTimer: ReturnType<typeof setTimeout> | undefined;
  function onInput(e: Event) {
    const q = (e.currentTarget as HTMLInputElement).value;
    bible.query = q;
    clearTimeout(parseTimer);
    parseTimer = setTimeout(() => void bible.typed(q), 150);
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      clearTimeout(parseTimer);
      void bible.submit();
    }
  }

  let req = $derived(bible.request());
  let reqKey = $derived(JSON.stringify(req));
  $effect(() => {
    void reqKey;
    const t = setTimeout(() => void bible.refreshPreview(), 200);
    return () => clearTimeout(t);
  });

  let feedback = $derived.by(() => {
    const p = bible.parsed;
    if (!bible.query.trim() || !p) return null;
    if (p.ok) return { ok: true, text: `${p.label} · Enter to open` };
    if (p.book) return { ok: false, text: p.error ?? "Not a reference" };
    return { ok: null, text: "Enter searches for these words" };
  });

  let book = $derived(bible.bookInfo);
  let tr = $derived(bible.currentTranslation);
  let chapters = $derived(book ? book.chapters.map((_, i) => i + 1) : []);
  let secondOptions = $derived([
    { value: "", label: "None" },
    ...bible.translations.filter((t) => t.id !== bible.translation).map((t) => ({ value: t.id as string, label: `${t.abbreviation} — ${t.name}` })),
  ]);
  let secondTr = $derived(bible.translations.find((t) => t.id === bible.second && t.id !== bible.translation) ?? null);
  let themeOptions = $derived([{ value: "", label: "Plain (no theme)" }, ...bible.themes.map((t) => ({ value: t.id as string, label: t.name }))]);
  let playlistName = $derived(session.playlist?.name ?? null);
  let canCreate = $derived(!!req && !bible.busy);

  function previewUrl(i: number): string | null {
    const conn = session.conn;
    if (!conn || !req) return null;
    return new ScriptureClient(conn.rest).previewUrl(previewParams(req, i, PREVIEW_W));
  }

  function openRecent(r: { translation: string; reference: string }) {
    bible.query = r.reference;
    if (r.translation !== bible.translation) void bible.setTranslation(r.translation).then((ok) => (ok ? bible.submit() : undefined));
    else void bible.submit();
  }

  function hitRef(h: { book: string; chapter: number; verse: number }): string {
    const b = bible.books.find((x) => x.key === h.book);
    return `${b ? referenceBookName(b.name) : h.book} ${h.chapter}:${h.verse}`;
  }
</script>

<PageFrame page="bible" subtitle={tr ? tr.name : undefined}>
  {#if !available}
    <div class="center">
      <EmptyState icon={BookOpen} title="This server has no Bible yet">Update the Vesper server to look up passages and put verses on screen.</EmptyState>
    </div>
  {:else if bible.ready && !bible.translations.length}
    <div class="center">
      <EmptyState icon={BookOpen} title="No translations installed">
        {bible.problem ?? "The server has no Bible translations. Install the server's scripture files, or put translation files in its data folder's scripture/."}
      </EmptyState>
    </div>
  {:else}
    <div class="body">
      <aside class="books" aria-label="Books">
        <label class="field">
          <span class="section-label">Translation</span>
          <Select
            options={bible.translations.map((t) => ({ value: t.id, label: `${t.abbreviation} — ${t.name}` }))}
            value={bible.translation}
            onchange={(v) => void bible.setTranslation(v)}
            data-testid="bible-translation"
          />
        </label>
        <div class="booklist">
          {#each TESTAMENTS as t (t.id)}
            {@const list = booksOf(bible.books, t.id)}
            {#if list.length}
              <h2 class="section-label">{t.label}</h2>
              <ul>
                {#each list as b (b.key)}
                  <li>
                    <button type="button" class:cur={b.key === bible.book} aria-current={b.key === bible.book ? "true" : undefined} onclick={() => bible.pickBook(b.key)}>{b.name}</button>
                  </li>
                {/each}
              </ul>
            {/if}
          {/each}
        </div>
        {#if book}
          <h2 class="section-label">Chapter</h2>
          <div class="chapters" role="group" aria-label="{book.name} chapters">
            {#each chapters as c (c)}
              <button type="button" class:cur={c === bible.chapter} aria-pressed={c === bible.chapter} onclick={() => void bible.openChapter(book.key, c)}>{c}</button>
            {/each}
          </div>
        {/if}
      </aside>

      <main class="reader">
        <div class="lookup">
          <label class="refbox" class:bad={feedback?.ok === false}>
            <Search size={16} strokeWidth={2} aria-hidden="true" />
            <input
              type="text"
              value={bible.query}
              oninput={onInput}
              onkeydown={onKey}
              placeholder="Ps 23:1-6, John 3:16 or words to search"
              aria-label="Reference or words"
              data-testid="bible-reference"
            />
          </label>
          <span class="hint" class:ok={feedback?.ok === true} class:err={feedback?.ok === false} aria-live="polite">
            {feedback?.text ?? "Type a reference (Ps 23:1-6) or search words"}
          </span>
          {#if bible.recent.length}
            <div class="recent" role="group" aria-label="Recent references">
              <span class="meta">Recent</span>
              {#each bible.recent as r (r.translation + r.reference)}
                <button type="button" class="chipbtn" title="{r.reference} ({r.translation})" onclick={() => openRecent(r)}>{r.reference}</button>
              {/each}
            </div>
          {/if}
        </div>
        <div class="verses">
          {#if bible.problem}<p class="err">{bible.problem}</p>{/if}
          {#if bible.results}
            <SectionLabel>
              {bible.results.total}
              {bible.results.total === 1 ? "verse" : "verses"} with “{bible.searched}”{bible.results.total > bible.results.hits.length ? ` · first ${bible.results.hits.length}` : ""}
              {#snippet actions()}<Button size="sm" variant="ghost" onclick={() => (bible.results = null)}>Back to the passage</Button>{/snippet}
            </SectionLabel>
            {#each bible.results.hits as h (h.book + h.chapter + ":" + h.verse)}
              <button type="button" class="hit" onclick={() => void bible.openChapter(h.book, h.chapter, [h.verse])} data-testid="bible-hit">
                <span class="ref">{hitRef(h)}</span>
                <span>{h.text}</span>
              </button>
            {:else}
              <p class="meta">No verse has all of these words.</p>
            {/each}
          {:else if bible.verses.length}
            <div class="vhead">
              <h1>{bible.heading} <span class="meta">{tr?.abbreviation ?? ""}</span></h1>
              <Button size="sm" variant="ghost" onclick={() => bible.selectAll()}>Select all</Button>
              <Button size="sm" variant="ghost" onclick={() => bible.selectNone()} disabled={!bible.selected.size}>None</Button>
            </div>
            {#each bible.verses as v, i (verseKey(v))}
              {@const on = bible.selected.has(verseKey(v))}
              <button type="button" class="verse" class:on aria-pressed={on} onclick={(e) => bible.clickVerse(i, e.shiftKey)} data-testid="bible-verse">
                <span class="n">{v.chapter !== bible.chapter ? `${v.chapter}:${v.verse}` : v.verse}</span>
                <span>{v.text}</span>
              </button>
            {/each}
            <p class="meta tip">Click verses to choose them; Shift+click selects a range.</p>
          {:else if bible.loading}
            <p class="meta">Reading…</p>
          {:else}
            <div class="center">
              <EmptyState icon={BookOpen} title="Choose a passage">Pick a book and a chapter, or type a reference such as “Ps 23:1-6”.</EmptyState>
            </div>
          {/if}
        </div>
      </main>

      <aside class="slides" aria-label="Generated slides">
        <div class="opts">
          <h2 class="section-label">Slide options</h2>
          <Segmented label="Verses per slide" size="sm" options={SPLITS} value={bible.split} onchange={(v: Split) => (bible.split = v)} />
          <div class="optrow">
            <span>Show reference on slide</span><Switch label="Show reference on slide" checked={bible.showReference} onchange={(c) => (bible.showReference = c)} />
          </div>
          <div class="optrow"><span>Verse numbers</span><Switch label="Verse numbers" checked={bible.verseNumbers} onchange={(c) => (bible.verseNumbers = c)} /></div>
          <label class="optrow"><span>Theme</span><Select options={themeOptions} value={bible.theme} onchange={(v) => (bible.theme = v)} data-testid="bible-theme" /></label>
          <label class="optrow" title="Parallel Bible: the same verses in a second translation on every slide"><span>Second translation</span><Select options={secondOptions} value={secondTr ? bible.second : ""} onchange={(v) => bible.setSecond(v)} data-testid="bible-second" /></label>
          {#if secondTr}
            <Segmented label="Second translation layout" size="sm" options={SECOND_LAYOUTS} value={bible.secondLayout} onchange={(v: SecondLayout) => bible.setSecondLayout(v)} />
          {/if}
        </div>
        <div class="previews" data-testid="bible-previews">
          {#if !req}
            <p class="meta">Choose verses to see their slides.</p>
          {:else if bible.previewProblem}
            <p class="err">{bible.previewProblem}</p>
          {:else if bible.preview}
            <p class="meta small">{bible.preview.name} · {bible.preview.slides.length} {bible.preview.slides.length === 1 ? "slide" : "slides"}</p>
            <div class="grid">
              {#each bible.preview.slides as s, i (i)}
                {@const url = previewUrl(i)}
                <figure>
                  {#if url}<img src={url} alt="Slide {i + 1}: {s.text}" loading="lazy" />{/if}
                  <figcaption>{s.reference ?? `Slide ${i + 1}`}</figcaption>
                </figure>
              {/each}
            </div>
            {#if tr?.attribution}<p class="meta small">{tr.attribution}</p>{/if}
            {#if secondTr?.attribution}<p class="meta small">{secondTr.attribution}</p>{/if}
          {/if}
        </div>
        <div class="actions">
          <Button
            disabled={!canCreate || !session.playlistId}
            title={session.playlistId ? `Add the passage to ${playlistName ?? "the open playlist"}` : "Open a playlist on the Show page first"}
            onclick={() => void bible.create("playlist")}>Add to playlist</Button
          >
          <Button variant="live" disabled={!canCreate} title="Create the passage and show it now" onclick={() => void bible.create("show")}>Go live</Button>
        </div>
        {#if !session.playlistId && req}<p class="meta small hintline">Open a playlist on the Show page to add passages to it.</p>{/if}
      </aside>
    </div>
  {/if}
</PageFrame>

<style>
  .center {
    flex: 1;
    display: grid;
    place-items: center;
    padding: 24px;
  }
  .body {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  .section-label {
    margin: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: var(--label-tracking);
    text-transform: uppercase;
    color: var(--text-label);
  }
  .books {
    flex: 0 0 230px;
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    background: var(--surface-panel);
    border-right: 1px solid var(--line);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .booklist {
    flex: 1 1 auto;
    min-height: 120px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .booklist ul {
    list-style: none;
    margin: 0 0 8px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .booklist button {
    width: 100%;
    padding: 5px 8px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text);
    text-align: left;
    font-size: var(--fs);
    cursor: pointer;
  }
  .booklist button:hover {
    background: var(--surface-raised);
  }
  .booklist button.cur {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .chapters {
    flex: none;
    max-height: 40%;
    overflow-y: auto;
    display: grid;
    grid-template-columns: repeat(6, minmax(0, 1fr));
    gap: 4px;
  }
  .chapters button {
    padding: 5px 0;
    border: 0;
    border-radius: var(--r-xs);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .chapters button:hover {
    background: var(--surface-overlay);
  }
  .chapters button.cur {
    background: var(--accent);
    color: var(--text-strong);
  }
  .reader {
    flex: 1 1 auto;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface-sunk);
  }
  .lookup {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 12px;
    padding: 12px 16px;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--line);
  }
  .refbox {
    flex: 1 1 280px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    border-radius: var(--r-card);
    border: 1px solid var(--accent);
    background: var(--surface-raised);
    color: var(--text-label);
  }
  .refbox.bad {
    border-color: var(--danger);
  }
  .refbox input {
    flex: 1;
    min-width: 0;
    padding: 9px 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-title);
    outline: none;
  }
  .hint {
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .hint.ok {
    color: var(--ok-text);
  }
  .hint.err,
  .err {
    color: var(--danger-text);
  }
  .recent {
    flex: 1 1 100%;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-sm);
  }
  .chipbtn {
    padding: 3px 9px;
    border-radius: var(--r-pill);
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    color: var(--text);
    font-size: var(--fs-sm);
    cursor: pointer;
  }
  .chipbtn:hover {
    border-color: var(--line-hover);
  }
  .verses {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .verses > :global(*) {
    flex-shrink: 0;
  }
  .vhead {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-bottom: 6px;
  }
  .vhead h1 {
    flex: 1;
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }
  .vhead h1 .meta {
    font-size: var(--fs);
    font-weight: 400;
  }
  .verse,
  .hit {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    padding: 9px 12px;
    border-radius: var(--r-card);
    border: 1px solid var(--line);
    background: transparent;
    color: var(--text);
    text-align: left;
    font: inherit;
    font-size: var(--fs-md);
    line-height: 1.5;
    cursor: pointer;
  }
  .verse:hover,
  .hit:hover {
    border-color: var(--line-hover);
  }
  .verse.on {
    background: #24223a;
    border-color: #4b3fa6;
  }
  .verse .n {
    flex: none;
    min-width: 22px;
    color: var(--accent-link);
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .hit {
    flex-direction: column;
    gap: 2px;
  }
  .hit .ref {
    color: var(--accent-link);
    font-weight: 600;
    font-size: var(--fs);
  }
  .meta {
    color: var(--text-meta);
  }
  p {
    margin: 0;
  }
  .tip {
    margin-top: 6px;
    font-size: var(--fs-sm);
  }
  .slides {
    flex: 0 0 clamp(340px, 24vw, 460px);
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface-panel);
    border-left: 1px solid var(--line);
  }
  .opts {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border-bottom: 1px solid var(--line);
  }
  .optrow {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
    color: var(--text-label);
  }
  .optrow :global(select) {
    max-width: 190px;
  }
  .previews {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 10px;
  }
  figure {
    margin: 0;
    display: flex;
    flex-direction: column;
    border-radius: var(--r);
    overflow: hidden;
    border: 1px solid var(--line);
  }
  figure img {
    display: block;
    width: 100%;
    aspect-ratio: 16 / 9;
    object-fit: cover;
    background: #000;
  }
  figcaption {
    padding: 2px 6px;
    background: var(--group-scripture);
    color: var(--text-strong);
    font-size: 10px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .small {
    font-size: var(--fs-sm);
  }
  .actions {
    flex: none;
    display: flex;
    gap: 8px;
    padding: 12px;
    border-top: 1px solid var(--line);
  }
  .actions > :global(*) {
    flex: 1;
  }
  .hintline {
    padding: 0 12px 10px;
  }
</style>
