<script lang="ts">
  // The bottom bin's Props tab: every prop with its picture and an on / off switch (props
  // stack: several can be up). + makes a blank prop or one from the current slide; Edit opens the
  // prop in the Edit view; right-click for Rename / Duplicate / Delete.
  import Plus from "lucide-svelte/icons/plus";
  import Pencil from "lucide-svelte/icons/pencil";
  import ListFilter from "lucide-svelte/icons/list-filter";
  import type { PropEntry } from "$protocol";
  import type { Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import { showDocsState } from "../../state/showDocs.svelte";
  import { shell } from "../../state/shell.svelte";
  import { useLive } from "../../live";
  import type { MenuItem } from "../../shell/menu";

  let { conn }: { conn: Connection } = $props();
  const live = useLive();
  const layers = live.layers;
  const liveSlide = live.slide;

  let filter = $state("");
  let list = $derived(showDocsState.propDocs.filter((p) => !filter.trim() || (p.name ?? "").toLowerCase().includes(filter.trim().toLowerCase())));
  let up = $derived(new Set(($layers?.props ?? []).map((p) => p.prop)));

  /** The slide "From Current Slide" copies: the live slide, else the open presentation's first. */
  let current = $derived.by(() => {
    const l = $liveSlide;
    if (l) return { presentation: l.presentation, slide: l.slide };
    const o = session.open;
    if (o && !o.prop && o.slides[0]) return { presentation: o.presentation.id, slide: o.slides[0].id };
    return null;
  });

  function nextName(): string {
    const taken = new Set(showDocsState.propDocs.map((p) => (p.name ?? "").toLowerCase()));
    for (let i = 1; ; i++) if (!taken.has(`prop ${i}`)) return `Prop ${i}`;
  }

  function addMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const items: MenuItem[] = [
      { type: "item", id: "blank", label: "Blank Prop", run: () => void create(false) },
      {
        type: "item",
        id: "slide",
        label: "From Current Slide",
        disabled: !current,
        title: current ? undefined : "Put a slide live (or open a presentation) first",
        run: () => void create(true),
      },
    ];
    shell.contextMenu = { x: r.left, y: r.bottom + 2, items };
  }

  async function create(fromSlide: boolean) {
    const e = await showDocsState.createProp(nextName(), fromSlide ? current : null);
    if (e && !fromSlide) void showDocsState.openProp(e.id);
  }

  async function guard(what: string, fn: () => Promise<unknown>) {
    try {
      await fn();
    } catch (e) {
      conn.pushNotice("error", `${what} failed: ${e instanceof Error ? e.message : String(e)}`);
    }
    await showDocsState.load("prop");
  }

  function menu(e: MouseEvent, p: PropEntry) {
    e.preventDefault();
    const items: MenuItem[] = [
      { type: "item", id: "edit", label: "Edit", run: () => void showDocsState.openProp(p.id) },
      {
        type: "item",
        id: "rename",
        label: "Rename…",
        run: async () => {
          const n = await shell.askText("Rename Prop", p.name ?? "", "Rename");
          if (n && n !== p.name) await guard("Rename", () => conn.rest.updateProp({ ...p, name: n }, p.revision));
        },
      },
      {
        type: "item",
        id: "dup",
        label: "Duplicate",
        run: () => {
          const { id: _id, revision: _r, sortKey: _s, ...rest } = p;
          void guard("Duplicate", () => conn.rest.createProp({ ...rest, name: `${p.name ?? "Prop"} copy` }));
        },
      },
      { type: "separator" },
      {
        type: "item",
        id: "del",
        label: "Delete",
        run: async () => {
          if (await shell.confirm("Delete Prop", `Delete “${p.name || "Prop"}”?`)) await guard("Delete", () => conn.rest.deleteProp(p.id, p.revision));
        },
      },
    ];
    shell.contextMenu = { x: e.clientX, y: e.clientY, items };
  }
</script>

<div class="body" data-testid="props-panel">
  <div class="grid">
    {#each list as p (p.id)}
      {@const on = up.has(p.id)}
      {@const w = p.size?.width ?? 1920}
      {@const h = p.size?.height ?? 1080}
      <div class="card" class:on oncontextmenu={(e) => menu(e, p)} role="group" aria-label={p.name} data-testid="prop-card">
        <button class="thumb" style:aspect-ratio={`${w} / ${h}`} onclick={() => session.run({ type: "propToggle", prop: p.id })} title={on ? "Click to hide" : "Click to show"} aria-label={`${on ? "Hide" : "Show"} ${p.name}`}>
          <img src={conn.rest.propThumbnailUrl(p.id, 240, p.revision)} alt="" loading="lazy" draggable="false" />
        </button>
        <div class="foot">
          <span class="name" title={p.name}>{p.name || "Prop"}</span>
          <button class="ibtn" onclick={() => showDocsState.openProp(p.id)} title="Edit" aria-label={`Edit ${p.name}`}><Pencil size={13} /></button>
          <button class="sw" class:on role="switch" aria-checked={on} onclick={() => session.run({ type: on ? "propHide" : "propShow", prop: p.id })} aria-label={`${p.name} on screen`} data-testid="prop-switch"><span></span></button>
        </div>
      </div>
    {:else}
      <div class="tile-empty">
        <button class="tile small" onclick={addMenu} aria-label="New Prop"><Plus size={38} strokeWidth={1.3} /></button>
        <span>{showDocsState.propDocs.length ? "No props match the filter" : "New Prop"}</span>
      </div>
    {/each}
  </div>
  <div class="bottom">
    <button class="ibtn" onclick={addMenu} title="New prop" aria-label="New Prop" data-testid="new-prop"><Plus size={16} /></button>
    <span class="meta">{showDocsState.propDocs.length} {showDocsState.propDocs.length === 1 ? "prop" : "props"} · {up.size} on screen</span>
    <span class="grow"></span>
    <label class="filter">
      <ListFilter size={14} />
      <input type="search" placeholder="Filter" bind:value={filter} spellcheck="false" aria-label="Filter Props" />
    </label>
  </div>
</div>

<style>
  .body {
    flex: 1;
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .grid {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 10px 12px;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 10px;
    align-content: start;
  }
  .grid > :global(.tile-empty) {
    grid-column: 1 / -1;
  }
  .card {
    display: flex;
    flex-direction: column;
    border: 2px solid var(--line);
    border-radius: var(--r);
    overflow: hidden;
    background: var(--surface-raised);
  }
  .card.on {
    border-color: var(--layer-props);
  }
  .thumb {
    display: block;
    width: 100%;
    padding: 0;
    border: 0;
    background: repeating-conic-gradient(var(--surface-panel) 0% 25%, var(--surface-sunk) 0% 50%) 0 0 / 16px 16px;
  }
  .thumb img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .foot {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 4px 3px 6px;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
  }
  .sw {
    flex: none;
    width: 28px;
    height: 16px;
    padding: 0;
    border: 0;
    border-radius: 8px;
    background: var(--surface-overlay);
    position: relative;
  }
  .sw span {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #ddd;
    transition: left 0.12s;
  }
  .sw.on {
    background: var(--layer-props);
  }
  .sw.on span {
    left: 14px;
    background: #fff;
  }
  .tile.small {
    width: 74px;
    height: 74px;
  }
  .bottom {
    flex: none;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 8px;
    border-top: 1px solid var(--line);
  }
  .meta {
    margin-left: 6px;
    color: var(--text-meta);
    font-size: 12px;
  }
  .grow {
    flex: 1;
  }
  .bottom .filter {
    width: 200px;
    min-width: 60px;
  }
</style>
