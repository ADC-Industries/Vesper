<script lang="ts">
  // The Library's "what" column (the board's second part): the folder's items as a sortable
  // table with a filter / search box (Codex search: this collection or all collections) and an
  // A–Z rail when sorted by title; media as a tile grid; Recently deleted with Restore.
  // Click selects; double-click or Enter adds a song to the toolbar's playlist. Round 4: media
  // Import, the Missing filter and relinking (MediaTools), the storage line, uploads in progress
  // (UploadsTray), the all-collections view (CollectionsList), and while a song is in Edit its
  // slides grouped by section (the slide editor's EditSlides).
  import Search from "lucide-svelte/icons/search";
  import { presence } from "../../state/presence.svelte";
  import { personLabel } from "../../state/presenceLogic";
  import RotateCcw from "lucide-svelte/icons/rotate-ccw";
  import ArrowUp from "lucide-svelte/icons/arrow-up";
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import type { SnippetPart } from "$protocol";
  import type { Connection } from "../../server";
  import { CodexClient, assetBlob } from "../../server/codex";
  import { Button, Segmented, nextSort, sortRows, type SortValue } from "../../ui";
  import { libraryPage, libraryState, type SearchScope } from "../../state/library.svelte";
  import { session } from "../../state/session.svelte";
  import { mediaPrep } from "../../state/mediaPrep.svelte";
  import { prepBadge } from "./prep";
  import { shell } from "../../state/shell.svelte";
  import { RAIL, compareTitles, daysLeft, formatBytes, formatDuration, matchesFilter, mediaKind, plural, railIndex, railTarget, shortTime } from "./logic";
  import Plus from "lucide-svelte/icons/plus";
  import MediaTools from "./MediaTools.svelte";
  import UploadsTray from "./UploadsTray.svelte";
  import CollectionsList from "./CollectionsList.svelte";
  import EditSlides from "../editor/EditSlides.svelte";
  import { mediaFiles } from "./upload";
  import { KIND_DOT } from "../show/sections";
  import BinItems from "./BinItems.svelte";
  import GroupsList from "./GroupsList.svelte";
  import { Select } from "../../ui";
  import { binsPage, type BinTarget } from "../../state/bins.svelte";
  import { binLabel, mediaPlaylists } from "./bins";
  import { binDrag } from "./binDrag.svelte";

  let { conn, readOnly }: { conn: Connection; readOnly: boolean } = $props();

  /** A table row: a document of this collection (or, searching every collection, another's). */
  interface Row {
    id: string;
    title: string;
    c2: string;
    c3: string | number;
    /** Songs only: the shelf (the songs table has four columns). */
    c4?: string;
    /** Songs only: the kind's dot colour (song, scripture, text, media). */
    dot?: string;
    note: SnippetPart[] | null;
    /** Another collection's (a search over every collection): it can't be added here. */
    collectionName: string | null;
  }

  let view = $derived(libraryPage.view);
  let q = $derived(libraryPage.query.trim());
  let shelfName = $derived(new Map(libraryState.libraries.map((l) => [l.id, l.name])));
  let activeCollection = $derived(libraryPage.loadedCollection);

  // Search: 150 ms after the last key (and again when the scope or the folder kind changes).
  $effect(() => {
    void libraryPage.query;
    void libraryPage.scope;
    void view.kind;
    const t = setTimeout(() => void libraryPage.search(), 150);
    return () => clearTimeout(t);
  });

  /** The server's hits for the current view (null: no search, or not answered yet). */
  let hits = $derived(q ? libraryPage.hits : null);
  let hitById = $derived(new Map((hits ?? []).map((h) => [h.id, h])));
  const visible = (id: string, name: string) => (hits ? hitById.has(id) : matchesFilter(name, q));
  const noteOf = (id: string) => {
    const h = hitById.get(id);
    return h && h.field !== "name" ? h.snippet : null;
  };

  let rows = $derived.by((): Row[] => {
    if (view.kind === "songs") {
      const shelf = view.shelf;
      const own = libraryPage.songs
        .filter((p) => (shelf ? p.library === shelf : true) && visible(p.id, p.name))
        .map((p) => ({
          id: p.id,
          title: p.name,
          c2: p.authors ?? "",
          c3: p.copyrightYear ?? "",
          c4: p.library ? (shelfName.get(p.library) ?? "") : "",
          dot: KIND_DOT[p.kind ?? "text"],
          note: noteOf(p.id),
          collectionName: null,
        }));
      if (libraryPage.scope !== "all" || !hits) return own;
      const others = hits
        .filter((h) => h.collection !== activeCollection && h.kind === "presentation")
        .map((h) => ({ id: h.id, title: h.title, c2: "", c3: "", c4: h.collectionName, note: h.field !== "name" ? h.snippet : null, collectionName: h.collectionName }));
      return [...own, ...others];
    }
    if (view.kind === "themes") {
      return libraryPage.themes
        .filter((t) => visible(t.id, t.name))
        .map((t) => ({ id: t.id, title: t.name, c2: String(t.slides.length), c3: t.usedBy ?? 0, note: null, collectionName: null }));
    }
    if (view.kind === "props") {
      return libraryPage.props
        .filter((p) => visible(p.id, p.name ?? ""))
        .map((p) => ({ id: p.id, title: p.name ?? "Untitled prop", c2: "Props layer", c3: "", note: null, collectionName: null }));
    }
    return [];
  });

  let heads = $derived(
    view.kind === "themes"
      ? [
          { key: "title", label: "Theme" },
          { key: "c2", label: "Slides" },
          { key: "c3", label: "Used by" },
        ]
      : view.kind === "props"
        ? [
            { key: "title", label: "Prop" },
            { key: "c2", label: "Layer" },
            { key: "c3", label: "" },
          ]
        : [
            { key: "title", label: "Title" },
            { key: "c2", label: "Author" },
            { key: "c3", label: "Year" },
            { key: "c4", label: "Shelf" },
          ],
  );

  let sorted = $derived.by(() => {
    const s = libraryPage.sort;
    if (!s) return rows;
    if (s.key === "title") {
      const out = [...rows].sort((a, b) => compareTitles(a.title, b.title));
      return s.dir === "asc" ? out : out.reverse();
    }
    return sortRows(rows, (r) => (r[s.key as "c2" | "c3" | "c4"] ?? "") as SortValue, s.dir);
  });

  let rail = $derived(libraryPage.sort?.key === "title" && view.kind === "songs" && sorted.length > 12 ? railIndex(sorted, (r) => r.title) : null);

  let missing = $derived(view.kind === "media" ? libraryPage.media.filter((m) => mediaKind(m.asset) === view.media && m.availability === "missing").length : 0);
  let media = $derived(
    view.kind === "media"
      ? libraryPage.media
          .filter((m) => mediaKind(m.asset) === view.media && visible(m.asset.id, m.asset.name ?? "") && (libraryPage.mediaFilter === "all" || m.availability === "missing"))
          .sort((a, b) => compareTitles(a.asset.name ?? "", b.asset.name ?? ""))
      : [],
  );

  /** A bin, media playlist or the groups (G17): their own content. */
  let binView = $derived(view.kind === "bin" || view.kind === "mediaPlaylist" || view.kind === "groups");
  let binTitle = $derived(
    view.kind === "bin"
      ? (binsPage.rows.find((r) => r.id === view.id)?.name ?? "Bin")
      : view.kind === "mediaPlaylist"
        ? (mediaPlaylists(session.playlistTree).find((p) => p.id === view.id)?.name ?? "Media playlist")
        : "Groups",
  );
  let binCount = $derived(
    view.kind === "bin"
      ? (binsPage.bin?.items.length ?? 0)
      : view.kind === "mediaPlaylist"
        ? (binsPage.playlist?.items.length ?? 0)
        : session.globalGroups.length,
  );
  /** "Add to" in the Images / Videos views: the bins and media playlists. */
  let addTargets = $derived([
    ...binsPage.rows.filter((r) => r.kind === "manual").map((r) => ({ value: `bin:${r.id}`, label: binLabel(r) })),
    ...mediaPlaylists(session.playlistTree).map((p) => ({ value: `playlist:${p.id}`, label: `${p.name} (playlist)` })),
  ]);
  let addTarget = $state("");
  $effect(() => {
    if (!addTargets.some((t) => t.value === addTarget)) addTarget = addTargets[0]?.value ?? "";
  });
  function addSelected() {
    const m = /^(bin|playlist):(.+)$/.exec(addTarget);
    const file = libraryPage.media.find((x) => x.asset.id === libraryPage.selected);
    if (!m || !file) return;
    void binsPage.addMedia({ kind: m[1], id: m[2] } as BinTarget, [file]);
  }

  let title = $derived(
    binView ? binTitle : view.kind === "songs"
      ? view.shelf
        ? (shelfName.get(view.shelf) ?? "Songs")
        : "All songs"
      : view.kind === "media"
        ? view.media === "image"
          ? "Images"
          : "Videos"
        : view.kind === "themes"
          ? "Themes"
          : view.kind === "props"
            ? "Props"
            : view.kind === "collections"
              ? "Collections"
              : "Recently deleted",
  );
  // Someone else holding the song's edit lock keeps its slides read-only here too (D09-5).
  let editing = $derived(
    view.kind === "songs" && libraryPage.mode === "edit" && !!libraryPage.editState && !presence.lockedBy("presentation", libraryPage.selected),
  );
  let count = $derived(
    binView ? binCount : view.kind === "media" ? media.length : view.kind === "trash" ? libraryPage.trash.length : view.kind === "collections" ? libraryPage.collections.length : rows.length,
  );
  let importOpen = $state(false);
  let dropping = $state(false);
  let storageLine = $derived.by(() => {
    const st = libraryPage.storage;
    if (!st) return null;
    const mine = st.collections.find((c) => c.collection === activeCollection);
    return {
      main: `${formatBytes(st.disk.available)} free of ${formatBytes(st.disk.total)}`,
      detail: `Media library ${formatBytes(st.library.bytes)} in ${plural(st.library.items, "file")}${mine ? ` · this collection uses ${formatBytes(mine.mediaBytes)}` : ""}`,
      low: st.disk.available < st.disk.keepFree * 1.5,
    };
  });

  function onDrop(e: DragEvent) {
    dropping = false;
    if (view.kind !== "media" || readOnly || !e.dataTransfer?.files.length) return;
    e.preventDefault();
    libraryPage.importFiles(mediaFiles(e.dataTransfer.files));
  }
  function onDragOver(e: DragEvent) {
    if (view.kind !== "media" || readOnly || !e.dataTransfer?.types.includes("Files")) return;
    e.preventDefault();
    dropping = true;
  }
  let placeholder = $derived(
    view.kind === "songs" ? "Search titles and lyrics" : view.kind === "media" ? "Search media" : view.kind === "themes" ? "Search themes" : "Search props",
  );

  let listEl = $state<HTMLDivElement | null>(null);

  function select(id: string) {
    libraryPage.select(id);
  }

  function add(r: Row) {
    if (r.collectionName || view.kind !== "songs") return;
    void libraryPage.addToPlaylist({ id: r.id, name: r.title });
  }

  function onListKey(e: KeyboardEvent) {
    const i = sorted.findIndex((r) => r.id === libraryPage.selected);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = sorted[Math.max(0, Math.min(sorted.length - 1, i + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) {
        select(next.id);
        listEl?.querySelector(`[data-id="${next.id}"]`)?.scrollIntoView({ block: "nearest" });
      }
    } else if (e.key === "Enter" && i >= 0) {
      e.preventDefault();
      add(sorted[i]!);
    }
  }

  function jump(letter: string) {
    if (!rail) return;
    const at = railTarget(rail, letter, libraryPage.sort?.dir === "desc");
    const r = at === null ? null : sorted[at];
    if (!r) return;
    select(r.id);
    const el = listEl?.querySelector<HTMLElement>(`[data-id="${r.id}"]`);
    el?.scrollIntoView({ block: "start" });
    el?.focus();
  }

  const scopes: { value: SearchScope; label: string }[] = [
    { value: "collection", label: "This collection" },
    { value: "all", label: "All collections" },
  ];

  let codex = $derived(new CodexClient(conn.rest));
  function thumb(m: (typeof media)[number]): string {
    const b = assetBlob(m.asset);
    return b ? codex.blobThumbnailUrl(b.sha256, b.ext) : conn.rest.mediaThumbnailUrl(m.asset.id, 320);
  }

  function focusSearch(node: HTMLInputElement) {
    if (libraryState.searchFocus) node.focus();
  }

  const KIND: Record<string, string> = { presentation: "Song", playlistNode: "Playlist", prop: "Prop", timer: "Timer", message: "Message", stageLayout: "Stage layout", library: "Shelf" };
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<section class="what" class:editing class:dropping aria-label={title} ondragover={onDragOver} ondragleave={() => (dropping = false)} ondrop={onDrop}>
  <div class="head">
    <h2>{editing ? "Slides" : title}</h2>
    <span class="count">{editing ? (session.open ? plural(session.open.slides.length, "slide") : "") : plural(count, "item")}</span>
    <span class="flex"></span>
    {#if editing}
      <Button size="sm" variant="ghost" onclick={() => libraryPage.endEdit()}>Done</Button>
    {:else if view.kind === "songs"}
      <Button size="sm" variant="outline" disabled={readOnly} onclick={() => session.newPresentation()}>+ New song</Button>
      <Button size="sm" variant="ghost" disabled={readOnly} title="A song with chords from a ChordPro file or pasted text" onclick={() => shell.appAction("importChordPro")} data-testid="import-chordpro">Import ChordPro…</Button>
    {:else if view.kind === "themes"}
      <Button size="sm" variant="outline" disabled={readOnly || libraryPage.busy} onclick={() => libraryPage.createTheme()}>+ New theme</Button>
    {:else if view.kind === "media"}
      {#if addTargets.length}
        <Select label="Bin or media playlist that Add uses" options={addTargets} bind:value={addTarget} style="width: 150px" />
        <Button size="sm" variant="ghost" disabled={readOnly || binsPage.busy || !addTarget || !libraryPage.selected} title="Adds the selected file to the chosen bin or media playlist" onclick={addSelected}>Add</Button>
      {/if}
      <Button size="sm" variant="outline" disabled={readOnly} aria-expanded={importOpen} onclick={() => (importOpen = !importOpen)}>+ Import</Button>
    {:else if view.kind === "groups"}
      <Button size="sm" variant="outline" disabled={readOnly || binsPage.busy} onclick={() => binsPage.createGroup()}>+ New group</Button>
    {:else if view.kind === "collections"}
      <Button size="sm" variant="outline" icon={Plus} disabled={readOnly || libraryPage.busy} onclick={() => libraryPage.createCollection()}>New collection</Button>
    {:else if view.kind === "trash"}
      <Button size="sm" variant="outline" disabled={!libraryPage.trash.length || readOnly} onclick={() => libraryPage.emptyTrash()}>Empty trash</Button>
    {/if}
  </div>

  {#if view.kind !== "trash" && view.kind !== "collections" && !binView && !editing}
    <div class="search">
      <label class="field">
        <Search size={15} strokeWidth={1.8} aria-hidden="true" />
        {#key libraryState.searchFocus}
          <input type="search" placeholder={placeholder} bind:value={libraryPage.query} spellcheck="false" aria-label={placeholder} {@attach focusSearch} />
        {/key}
      </label>
      {#if q}
        <div class="scope">
          <Segmented size="sm" label="Search in" options={scopes} bind:value={libraryPage.scope} />
          <span class="found">{libraryPage.hits === null ? "Searching…" : libraryPage.searchError ? libraryPage.searchError : plural(libraryPage.hits.length, "match", "matches")}</span>
        </div>
      {/if}
    </div>
  {/if}

  {#if editing && libraryPage.editState}
    <div class="slides"><EditSlides {conn} st={libraryPage.editState} /></div>
  {:else if view.kind === "collections"}
    <CollectionsList />
  {:else if view.kind === "groups"}
    <GroupsList {readOnly} />
  {:else if view.kind === "bin" || view.kind === "mediaPlaylist"}
    <BinItems {conn} {readOnly} />
  {:else if view.kind === "media"}
    <MediaTools bind:importOpen {readOnly} {missing} />
    <div class="grid-wrap">
      <div class="grid">
        {#each media as m (m.asset.id)}
          <button
            type="button"
            class="tile"
            class:on={libraryPage.selected === m.asset.id}
            aria-pressed={libraryPage.selected === m.asset.id}
            onpointerdown={(e) => !readOnly && binDrag.press(e, [m])}
            onclick={() => !binDrag.swallowClick() && select(m.asset.id)}
          >
            <span class="thumb">
              <!-- A missing file has no thumbnail (404): no broken-image icon. -->
              {#if m.availability !== "missing"}<img src={thumb(m)} alt="" loading="lazy" draggable="false" />{/if}
              {#if m.availability !== "online"}<span class="badge warn">{m.availability === "missing" ? "MISSING" : "COPYING"}</span>
              {:else if prepBadge(mediaPrep.jobFor(m.asset.id), m.asset)}
                {@const b = prepBadge(mediaPrep.jobFor(m.asset.id), m.asset)!}
                <span class="badge" class:warn={b.warn} data-testid="prep-badge">{b.text}</span>
              {:else if m.asset.probe?.duration}<span class="badge">{formatDuration(m.asset.probe.duration)}</span>{/if}
            </span>
            <span class="tname">{(m.asset.name ?? "").replace(/\.[^.]+$/, "")}</span>
            <span class="tsub">{m.asset.probe?.width ? `${m.asset.probe.width}×${m.asset.probe.height}` : ""}{m.asset.name?.match(/\.([^.]+)$/) ? ` · ${m.asset.name.match(/\.([^.]+)$/)![1]!.toUpperCase()}` : ""}</span>
          </button>
        {:else}
          <p class="empty">
            {q
              ? `Nothing here matches “${q}”.`
              : libraryPage.mediaFilter === "missing"
                ? "Nothing is missing here."
                : view.media === "image"
                  ? "No images in this collection yet. Use + Import or drop files here."
                  : "No videos in this collection yet. Use + Import or drop files here."}
          </p>
        {/each}
      </div>
    </div>
    {#if storageLine}
      <div class="storage" class:low={storageLine.low} title={storageLine.detail}>
        <span class="sl">Codex storage</span>
        <span class="sv">{storageLine.main}</span>
        <span class="sd">{storageLine.detail}</span>
      </div>
    {/if}
  {:else if view.kind === "trash"}
    <div class="list" role="list">
      {#each libraryPage.trash as t (t.id)}
        <div class="trow" class:on={libraryPage.selected === t.id} role="listitem">
          <button type="button" class="tmain" onclick={() => select(t.id)}>
            <span class="tt">{t.name}</span>
            <span class="tm">{KIND[t.kind] ?? t.kind}{t.subtree ? ` and ${plural(t.subtree, "item")} in it` : ""} · deleted {shortTime(t.deletedAt)}{t.by.client ? ` by ${t.by.client}` : ""} · {plural(daysLeft(t.expiresAt), "day")} left</span>
          </button>
          <Button size="sm" variant="secondary" icon={RotateCcw} disabled={readOnly} onclick={() => libraryPage.restore(t)}>Restore</Button>
        </div>
      {:else}
        <p class="empty">Nothing was deleted in the last 30 days.</p>
      {/each}
      {#if libraryPage.trash.length}<p class="foot">Deleted songs, playlists, props, timers and stage layouts stay here for 30 days.</p>{/if}
    </div>
  {:else}
    <div class="table" class:four={heads.length > 3}>
      <div class="thead" role="row">
        {#each heads as h (h.key)}
          {#if h.label}
            <button type="button" class="th" role="columnheader" aria-sort={libraryPage.sort?.key === h.key ? (libraryPage.sort.dir === "asc" ? "ascending" : "descending") : "none"} onclick={() => (libraryPage.sort = nextSort(libraryPage.sort, h.key))}>
              {h.label}
              {#if libraryPage.sort?.key === h.key}
                {#if libraryPage.sort.dir === "asc"}<ArrowUp size={12} strokeWidth={2} aria-hidden="true" />{:else}<ArrowDown size={12} strokeWidth={2} aria-hidden="true" />{/if}
              {/if}
            </button>
          {:else}<span></span>{/if}
        {/each}
      </div>
      <div class="body">
        <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
        <div class="rows" bind:this={listEl} role="listbox" aria-label={title} tabindex="-1" onkeydown={onListKey}>
          {#each sorted as r (r.id + (r.collectionName ?? ""))}
            <button
              type="button"
              class="row"
              class:on={libraryPage.selected === r.id}
              class:foreign={!!r.collectionName}
              role="option"
              aria-selected={libraryPage.selected === r.id}
              data-id={r.id}
              title={view.kind === "songs" && !r.collectionName ? "Double-click or Enter adds it to the playlist" : undefined}
              onclick={() => select(r.id)}
              ondblclick={() => add(r)}
            >
              <span class="c1">
                <span class="name">{#if r.dot}<span class="kdot" style:background={r.dot} aria-hidden="true"></span>{/if}{r.title}</span>
                {#if !r.collectionName}
                  {@const lk = presence.lockedBy(view.kind === "themes" ? "theme" : view.kind === "props" ? "prop" : "presentation", r.id)}
                  {#if lk}<span class="editing" data-editing-note>{personLabel(lk)} is editing</span>{/if}
                {/if}
                {#if r.note}<span class="note">{#each r.note as part, j (j)}{#if part.mark}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}</span>{/if}
              </span>
              <span class="c2" title={r.c2 || undefined}>{r.c2}</span>
              <span class="c3">{r.c3}</span>
              {#if heads.length > 3}<span class="c2">{r.c4 ?? ""}</span>{/if}
            </button>
          {:else}
            <p class="empty">
              {q ? `Nothing here matches “${q}”.` : libraryPage.loading ? "Loading…" : view.kind === "songs" ? "No songs here yet." : view.kind === "themes" ? "No themes in this collection." : "No props in this collection."}
            </p>
          {/each}
        </div>
        {#if rail}
          <div class="rail" role="toolbar" aria-label="Jump to a letter">
            {#each RAIL as l (l)}
              <button type="button" class:has={rail.has(l)} tabindex="-1" aria-label={`Jump to ${l}`} onclick={() => jump(l)}>{l}</button>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  {/if}
  <UploadsTray />
</section>

<style>
  .what {
    position: relative;
    flex: 0 0 430px;
    min-width: 0;
    display: flex;
    flex-direction: column;
    border-right: 1px solid var(--line);
    background: var(--surface-sunk);
  }
  .what.editing {
    flex-basis: clamp(240px, 20vw, 430px);
  }
  .what.dropping::after {
    content: "Drop to import into the Codex";
    position: absolute;
    inset: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: 2px dashed var(--accent);
    border-radius: var(--r-card);
    background: rgba(18, 19, 23, 0.85);
    color: var(--accent-link);
    font-weight: 600;
    pointer-events: none;
  }
  .slides {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--surface-raised);
  }
  .slides > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .storage {
    flex: none;
    display: grid;
    grid-template-columns: auto 1fr;
    column-gap: 8px;
    padding: 7px 12px;
    border-top: 1px solid var(--surface-raised);
    font-size: var(--fs-xs);
  }
  .sl {
    color: var(--text-label);
    font-weight: 700;
    letter-spacing: var(--label-tracking);
    text-transform: uppercase;
    font-size: 10px;
  }
  .sv {
    text-align: right;
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .sd {
    grid-column: 1 / -1;
    color: var(--text-meta);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .storage.low .sv {
    color: var(--live);
  }
  .head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 12px 8px;
  }
  h2 {
    margin: 0;
    font-size: var(--fs-title);
    font-weight: 700;
  }
  .count {
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .flex {
    flex: 1;
  }
  .search {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 0 12px 8px;
  }
  .field {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 8px;
    border-radius: var(--r);
    background: var(--surface-raised);
    border: 1px solid var(--line);
    color: var(--text-label);
  }
  .field:focus-within {
    border-color: var(--accent);
  }
  .field input {
    flex: 1;
    min-width: 0;
    min-height: 28px;
    border: 0;
    padding: 0;
    background: transparent;
    outline: none;
    box-shadow: none;
    font-size: 12.5px;
  }
  .scope {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .found {
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
  .table {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .thead,
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 120px 50px;
    gap: 10px;
    align-items: center;
  }
  .four .thead,
  .four .row {
    grid-template-columns: minmax(0, 1fr) minmax(0, 0.55fr) 44px minmax(0, 0.4fr);
  }
  .kdot {
    display: inline-block;
    width: 8px;
    height: 8px;
    margin-right: 7px;
    border-radius: 2px;
    vertical-align: 1px;
  }
  .thead {
    flex: none;
    padding: 0 12px;
    border-top: 1px solid var(--surface-raised);
    border-bottom: 1px solid var(--surface-raised);
    background: var(--surface-panel);
  }
  .th {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 7px 0;
    border: 0;
    background: none;
    color: var(--text-label);
    text-align: left;
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    white-space: nowrap;
  }
  .th[aria-sort="ascending"],
  .th[aria-sort="descending"] {
    color: var(--text);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .rows {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    outline: none;
  }
  .row {
    width: 100%;
    padding: 7px 12px;
    border: 0;
    border-bottom: 1px solid #1f2126;
    background: transparent;
    color: var(--text);
    text-align: left;
    font-size: var(--fs);
  }
  .row:hover {
    background: var(--surface-raised);
  }
  .row.on {
    background: var(--accent-sel);
  }
  .row.foreign .name {
    color: var(--text-2);
  }
  .row:focus-visible {
    outline: 2px solid var(--accent-ring);
    outline-offset: -2px;
  }
  .c1 {
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .name {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .editing {
    font-size: var(--fs-xs);
    color: var(--info);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .note {
    font-size: var(--fs-xs);
    color: var(--text-meta);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  mark {
    background: transparent;
    color: var(--accent-link);
    font-weight: 600;
  }
  .c2,
  .c3 {
    color: var(--text-label);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .c3 {
    font-variant-numeric: tabular-nums;
  }
  .rail {
    flex: none;
    width: 22px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: space-between;
    padding: 4px 0;
    border-left: 1px solid var(--surface-raised);
    overflow: hidden;
  }
  .rail button {
    flex: 1 1 0;
    min-height: 0;
    max-height: 22px;
    width: 100%;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text-disabled);
    font-size: 10px;
    font-weight: 700;
    line-height: 1;
  }
  .rail button.has {
    color: var(--text-label);
  }
  .rail button.has:hover {
    color: var(--accent-link);
  }
  .empty {
    margin: 0;
    padding: 16px;
    color: var(--text-label);
  }
  .grid-wrap {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 4px 12px 12px;
    border-top: 1px solid var(--surface-raised);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
    padding-top: 8px;
  }
  .grid .empty {
    grid-column: 1 / -1;
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 4px;
    border-radius: 7px;
    border: 2px solid transparent;
    background: var(--surface-panel);
    color: var(--text);
    text-align: left;
    min-width: 0;
  }
  .tile:hover {
    border-color: var(--line-strong);
  }
  .tile.on {
    border-color: var(--accent);
  }
  .thumb {
    position: relative;
    display: block;
    aspect-ratio: 16 / 9;
    border-radius: 4px;
    overflow: hidden;
    background: var(--surface-well);
  }
  .thumb img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .badge {
    position: absolute;
    right: 4px;
    bottom: 4px;
    padding: 0 5px;
    border-radius: 3px;
    background: rgba(0, 0, 0, 0.65);
    font-size: 10px;
    font-weight: 800;
  }
  .badge.warn {
    background: var(--live);
    color: var(--live-ink);
  }
  .tname,
  .tsub {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tname {
    font-size: var(--fs-sm);
    font-weight: 600;
  }
  .tsub {
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    border-top: 1px solid var(--surface-raised);
  }
  .trow {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-bottom: 1px solid #1f2126;
  }
  .trow.on {
    background: var(--accent-sel);
  }
  .tmain {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    padding: 2px 0;
    border: 0;
    background: none;
    color: var(--text);
    text-align: left;
  }
  .tt {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tm {
    font-size: var(--fs-xs);
    color: var(--text-meta);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .foot {
    margin: 0;
    padding: 12px;
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
</style>
