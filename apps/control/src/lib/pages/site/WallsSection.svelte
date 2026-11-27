<script lang="ts">
  // Site ▸ Displays ▸ Video walls (Vesper Mosaic round M2, docs/MOSAIC-PLAN.md): each wall as a
  // card (its picture cut by its tiles with the bezels left out, its state and sync, what it
  // shows, Identify, Edit, Delete) and the wall designer: name, rows × columns, bezels and the
  // screens' visible size, what it shows, and the screens dragged into its cells (from the
  // Displays grid above or the list beside the wall) or picked per cell. Saving keeps the feed
  // the wall shows now unless the designer's own Shows was changed (H08-2); Rows and Columns
  // apply when the field is left or Enter pressed, never per keystroke (H08-3); Delete asks first
  // (H08-6: the wall's screens show the whole picture again at once). A cell's screen that no
  // box lists now stays in its cell, "(not listed now)", and Save keeps it (R1-5).
  import type { MosaicChannel, MosaicWall } from "$protocol";
  import { Button, Select, StatusDot } from "../../ui";
  import { mosaicPage } from "../../state/mosaic.svelte";
  import {
    cellChoices,
    cellKey,
    cellName,
    draftOf,
    emptyWall,
    feedChoices,
    feedLabel,
    feedSource,
    feedValue,
    freeScreens,
    parseFeed,
    parseKey,
    placeScreen,
    resizeWall,
    wallBody,
    SCREEN_DRAG,
    screenLabel,
    unlistedScreenLabel,
    wallLayout,
    wallProblem,
    wallStatus,
    type DisplayTile,
    type WallDraft,
  } from "./mosaic";

  let { tiles, channels, sources, walls }: { tiles: DisplayTile[]; channels: MosaicChannel[]; sources: string[]; walls: MosaicWall[] } = $props();

  let draft = $state<WallDraft | null>(null);
  /** The wall being edited (null: a new one). */
  let editing = $state<string | null>(null);
  let over = $state<string | null>(null);
  let saving = $state(false);
  /** The designer's own Shows was changed: otherwise Save keeps the wall's live feed (H08-2). */
  let feedChanged = $state(false);
  /** The wall whose Delete asks for confirmation (H08-6). */
  let deleteAsk = $state<string | null>(null);
  /** The wall whose tiles show their row and column big (while identifying). */
  let identifying = $state<string | null>(null);

  let problem = $derived(draft ? wallProblem(draft, walls, editing) : null);
  let free = $derived(freeScreens(tiles, editing));
  let byKey = $derived(new Map(tiles.map((t) => [t.key, t])));
  let live = $derived(editing ? walls.find((w) => w.id === editing) : undefined);
  /** What the designer shows as the wall's feed: the live one until its Shows is changed. */
  let designFeed = $derived(draft ? (live && !feedChanged ? live.feed : draft.feed) : null);

  function startNew() {
    let n = walls.length + 1;
    while (walls.some((w) => w.name.toLowerCase() === `wall ${n}`)) n++;
    draft = emptyWall(`Wall ${n}`);
    editing = null;
    feedChanged = false;
  }
  function edit(w: MosaicWall) {
    draft = draftOf(w);
    editing = w.id;
    feedChanged = false;
  }
  async function save() {
    if (!draft || problem) return;
    saving = true;
    const body = wallBody(draft, live, feedChanged);
    try {
      const r = editing ? await mosaicPage.putWall(editing, body) : await mosaicPage.createWall(body);
      if (r !== undefined) draft = null;
      else if (!editing) {
        // A wall made though a screen didn't take it (503): the designer edits it from now on,
        // so Save again changes it instead of making it twice (409).
        const before = new Set(walls.map((w) => w.id));
        await mosaicPage.load();
        const made = mosaicPage.state?.walls?.find((w) => !before.has(w.id) && w.name.toLowerCase() === body.name.toLowerCase());
        if (made && draft) {
          editing = made.id;
          feedChanged = true;
        }
      }
    } finally {
      saving = false;
    }
  }
  async function remove(id: string) {
    deleteAsk = null;
    await mosaicPage.deleteWall(id);
  }
  /** Rows or Columns left (or Enter): the new size; an emptied field shows the size again. */
  function resized(e: Event & { currentTarget: HTMLInputElement }, which: "rows" | "columns") {
    if (!draft) return;
    const v = e.currentTarget.valueAsNumber;
    resize(which === "rows" ? v : draft.rows, which === "columns" ? v : draft.columns);
    e.currentTarget.value = String(draft[which]);
  }
  async function identify(w: MosaicWall) {
    identifying = w.id;
    setTimeout(() => identifying === w.id && (identifying = null), 5000);
    await mosaicPage.identifyWall(w.id);
  }
  function place(row: number, column: number, key: string) {
    if (draft) draft = { ...draft, cells: placeScreen(draft.cells, row, column, parseKey(key)) };
  }
  function resize(rows: number, columns: number) {
    if (draft) draft = resizeWall(draft, rows, columns);
  }
  function setNum(i: 0 | 1, k: "bezelMm" | "screenMm", v: number) {
    if (!draft) return;
    const cur: [number, number] = draft[k] ? [...(draft[k] as [number, number])] : [0, 0];
    cur[i] = Number.isFinite(v) ? v : 0;
    draft = { ...draft, [k]: cur };
  }

  function drop(e: DragEvent, row: number, column: number) {
    e.preventDefault();
    over = null;
    const key = e.dataTransfer?.getData(SCREEN_DRAG) || e.dataTransfer?.getData("text/plain") || "";
    if (free.some((t) => t.key === key)) place(row, column, key);
  }
  function dragOver(e: DragEvent, id: string) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
    over = id;
  }
  function dragStart(e: DragEvent, key: string) {
    e.dataTransfer?.setData(SCREEN_DRAG, key);
    e.dataTransfer?.setData("text/plain", key);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = "move";
  }
  function tileLabel(machine: string, screen: number): string {
    const t = byKey.get(`${machine}/${screen}`);
    return t ? screenLabel(t) : unlistedScreenLabel(screen);
  }
  function placedIn(key: string): string | null {
    const c = draft?.cells.find((x) => `${x.machine}/${x.screen}` === key);
    return c ? `R${c.row + 1} C${c.column + 1}` : null;
  }
</script>

{#snippet picture(w: Pick<WallDraft, "rows" | "columns" | "bezelMm" | "screenMm" | "cells">, source: string, design: boolean, big: boolean)}
  {@const layout = wallLayout(w)}
  {@const thumb = mosaicPage.sourceThumbnail(source)}
  <div class="wallpic" class:design style="aspect-ratio: {layout.aspect}; width: min(100%, {design ? 640 : 520}px, {(design ? 220 : 200) * layout.aspect}px)" data-testid={design ? "mosaic-wall-cells" : "mosaic-wall-picture"}>
    {#each layout.cells as c (`${c.row}/${c.column}`)}
      {@const key = cellKey(w.cells, c.row, c.column)}
      {@const id = `${c.row}/${c.column}`}
      <div
        class="cell"
        class:empty={!key}
        class:over={over === id}
        style="left: {c.x * 100}%; top: {c.y * 100}%; width: {c.w * 100}%; height: {c.h * 100}%"
        role={design ? "group" : undefined}
        aria-label={design ? cellName(c.row, c.column) : undefined}
        ondragover={design ? (e) => dragOver(e, id) : undefined}
        ondragleave={design ? () => over === id && (over = null) : undefined}
        ondrop={design ? (e) => drop(e, c.row, c.column) : undefined}
        data-testid={design ? "mosaic-wall-cell" : undefined}
      >
        {#if thumb && key}
          <img src={thumb} alt="" style="width: {100 / c.w}%; height: {100 / c.h}%; left: {(-c.x / c.w) * 100}%; top: {(-c.y / c.h) * 100}%" />
        {/if}
        <span class="rc" class:big>R{c.row + 1} C{c.column + 1}</span>
        {#if design}
          <span class="pick">
            <Select label={`${cellName(c.row, c.column)}: screen`} options={cellChoices(w.cells, c.row, c.column, free)} value={key} onchange={(v) => place(c.row, c.column, v)} />
          </span>
        {:else if key}
          {@const p = parseKey(key)}
          {#if p}<span class="who">{tileLabel(p.machine, p.screen)}</span>{/if}
        {/if}
      </div>
    {/each}
  </div>
{/snippet}

<h2 class="st-section">Video walls</h2>
<p class="st-note">A video wall shows one picture across several screens side by side, of one display box or several, in step (within one refresh). Each screen shows its part, with the bezels between them left out.</p>

{#if walls.length}
  <div class="walls">
    {#each walls as w (w.id)}
      {@const s = wallStatus(w)}
      <div class="st-card wall" data-testid="mosaic-wall">
        <div class="st-row">
          <h3>{w.name}</h3>
          <span class="st-meta">{w.rows} × {w.columns} · {w.cells.length} screen{w.cells.length === 1 ? "" : "s"}{w.bezelMm[0] || w.bezelMm[1] ? ` · bezels ${w.bezelMm[0]} / ${w.bezelMm[1]} mm` : ""}</span>
          <span class="st-grow"></span>
          <span title={w.problem ?? undefined} data-testid="mosaic-wall-status"><StatusDot status={s.status} label={s.label} /></span>
        </div>
        {@render picture(w, feedSource(w.feed, channels), false, identifying === w.id)}
        {#if w.problem}<p class="st-problem">{w.problem}</p>{/if}
        <div class="st-row">
          <span class="lbl">Shows</span>
          <Select label={`What ${w.name} shows`} options={feedChoices(sources, channels, w.feed)} value={feedValue(w.feed)} onchange={(v) => mosaicPage.setWallFeed(w.id, parseFeed(v))} data-testid="mosaic-wall-feed" />
          <span class="st-grow"></span>
          <Button size="sm" onclick={() => identify(w)} disabled={!w.cells.length}>Identify</Button>
          <Button size="sm" onclick={() => edit(w)} disabled={editing === w.id && !!draft}>Edit</Button>
          <Button size="sm" onclick={() => (deleteAsk = w.id)} disabled={deleteAsk === w.id} data-testid="mosaic-wall-delete">Delete</Button>
        </div>
        {#if deleteAsk === w.id}
          <div class="st-confirm" role="alertdialog" aria-label={`Delete the wall ${w.name}`} data-testid="mosaic-wall-delete-ask">
            <span class="st-two st-grow">
              <span class="st-strong">Delete {w.name}?</span>
              <span class="st-meta">Its screens keep the feed, but each shows the whole picture at once. The wall's layout is gone.</span>
            </span>
            <Button size="sm" onclick={() => (deleteAsk = null)}>Cancel</Button>
            <Button size="sm" variant="danger" onclick={() => remove(w.id)}>Delete wall</Button>
          </div>
        {/if}
      </div>
    {/each}
  </div>
{/if}

{#if !draft}
  <div class="st-row">
    <Button size="sm" onclick={startNew} disabled={!tiles.length} data-testid="mosaic-new-wall">New video wall</Button>
    {#if !tiles.length}<span class="st-meta">Walls are made of the display boxes' screens: none is listed yet.</span>{/if}
  </div>
{:else}
  <div class="st-card designer" data-testid="mosaic-wall-designer">
    <div class="st-row">
      <h3>{editing ? `Edit ${walls.find((w) => w.id === editing)?.name ?? "the wall"}` : "New video wall"}</h3>
      <span class="st-grow"></span>
      <span class="st-meta">{feedLabel(designFeed ?? draft.feed, channels)}</span>
    </div>
    <div class="fields">
      <label class="name">Name<input type="text" maxlength="48" bind:value={draft.name} aria-label="Wall name" /></label>
      <label>Rows<input type="number" min="1" max="16" value={draft.rows} onchange={(e) => resized(e, "rows")} aria-label="Rows" /></label>
      <label>Columns<input type="number" min="1" max="16" value={draft.columns} onchange={(e) => resized(e, "columns")} aria-label="Columns" /></label>
      <label>Bezel ↔ mm<input type="number" min="0" max="10000" step="0.5" value={draft.bezelMm[0]} oninput={(e) => setNum(0, "bezelMm", e.currentTarget.valueAsNumber)} aria-label="Bezel between columns, mm" /></label>
      <label>Bezel ↕ mm<input type="number" min="0" max="10000" step="0.5" value={draft.bezelMm[1]} oninput={(e) => setNum(1, "bezelMm", e.currentTarget.valueAsNumber)} aria-label="Bezel between rows, mm" /></label>
      <label>Screen width mm<input type="number" min="0" step="1" value={draft.screenMm?.[0] ?? ""} oninput={(e) => setNum(0, "screenMm", e.currentTarget.valueAsNumber)} aria-label="Screen's visible width, mm" /></label>
      <label>Screen height mm<input type="number" min="0" step="1" value={draft.screenMm?.[1] ?? ""} oninput={(e) => setNum(1, "screenMm", e.currentTarget.valueAsNumber)} aria-label="Screen's visible height, mm" /></label>
      <label class="feedpick">Shows<Select label="What the wall shows" options={feedChoices(sources, channels, designFeed ?? draft.feed)} value={feedValue(designFeed ?? draft.feed)} onchange={(v) => draft && ((draft = { ...draft, feed: parseFeed(v) }), (feedChanged = true))} /></label>
    </div>
    <div class="design">
      {@render picture(draft, feedSource(designFeed ?? draft.feed, channels), true, false)}
      <div class="palette" role="list" data-testid="mosaic-wall-palette">
        <span class="st-meta">Drag a screen into a cell (or from the Displays above), or pick one per cell.</span>
        {#each free as t (t.key)}
          {@const at = placedIn(t.key)}
          <span class="chip" class:placed={!!at} draggable="true" ondragstart={(e) => dragStart(e, t.key)} role="listitem" data-testid="mosaic-wall-chip">
            {screenLabel(t)}{#if at}<span class="at">{at}</span>{/if}
          </span>
        {/each}
        {#if !free.length}<span class="st-meta">Every screen is in another wall.</span>{/if}
      </div>
    </div>
    {#if problem}<p class="st-problem" data-testid="mosaic-wall-problem">{problem}</p>{/if}
    <div class="st-row">
      <span class="st-meta">{draft.cells.length} of {draft.rows * draft.columns} cells filled · every screen gets the same geometry and sync group, and the wall's feed</span>
      <span class="st-grow"></span>
      <Button size="sm" onclick={() => (draft = null)}>Cancel</Button>
      <Button size="sm" variant="primary" onclick={save} disabled={!!problem || saving} data-testid="mosaic-wall-save">{editing ? "Save wall" : "Create wall"}</Button>
    </div>
  </div>
{/if}

<style>
  .walls {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(420px, 1fr));
    align-items: start;
    gap: 10px;
  }
  h3 {
    margin: 0;
    font-size: var(--fs-md, 14px);
  }
  .lbl {
    color: var(--text-label);
    min-width: 48px;
  }
  .wallpic {
    position: relative;
    background: var(--surface-well);
    border-radius: 4px;
    max-width: 100%;
  }
  .wallpic.design {
    background: color-mix(in srgb, var(--surface-well) 70%, var(--text) 8%);
  }
  .cell {
    position: absolute;
    overflow: hidden;
    background: #000;
    outline: 1px solid var(--line-strong);
  }
  .cell.empty {
    background: var(--surface-panel);
    outline-style: dashed;
  }
  .cell.over {
    outline: 2px solid var(--accent);
    z-index: 1;
  }
  .cell img {
    position: absolute;
    object-fit: contain;
    max-width: none;
    pointer-events: none;
  }
  .rc {
    position: absolute;
    top: 4px;
    left: 4px;
    padding: 0 5px;
    border-radius: 3px;
    background: rgb(0 0 0 / 0.6);
    color: #fff;
    font-size: 10px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .rc.big {
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    font-size: 18px;
    padding: 2px 10px;
    background: var(--accent);
  }
  .who {
    position: absolute;
    left: 4px;
    right: 4px;
    bottom: 3px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: #fff;
    font-size: 10px;
    text-shadow: 0 1px 2px #000;
  }
  .pick {
    position: absolute;
    left: 4px;
    right: 4px;
    bottom: 4px;
  }
  .pick :global(select) {
    width: 100%;
    min-height: 24px;
    font-size: var(--fs-sm);
  }
  .fields {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 10px 14px;
  }
  .fields label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .fields input[type="number"] {
    width: 96px;
  }
  .fields .name input {
    width: 180px;
  }
  .design {
    display: flex;
    flex-wrap: wrap;
    gap: 14px;
    align-items: flex-start;
  }
  .design .wallpic {
    flex: 0 1 auto;
  }
  .palette {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 220px;
    flex: 1 1 220px;
  }
  .chip {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-panel);
    cursor: grab;
    font-size: var(--fs-sm);
  }
  .chip.placed {
    color: var(--text-2);
  }
  .at {
    margin-left: auto;
    color: var(--accent);
    font-variant-numeric: tabular-nums;
  }
</style>
