<script lang="ts">
  // Edit view, left: the presentation's name with + (new slide), then its slides grouped by
  // section (collapsible headers in the group colour). Right-click: Duplicate, Delete, Move to
  // Group. Drag a slide to reorder it (moveSlide). A theme's template slides (theme mode) get
  // the same, with "+" offering the starter layouts, each slide's roles under it, and a delete
  // that names the songs following that template slide (they keep their look, unlinked).
  import Plus from "lucide-svelte/icons/plus";
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import type { PresentationOp, Section } from "$protocol";
  import type { Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import { shell } from "../../state/shell.svelte";
  import { aspectRatio, cssColor, thumbWidthFor } from "../../model";
  import { slideHash } from "../../edit/reflow";
  import { uuid } from "../../edit/ids";
  import { addSlideOps } from "../../edit/ops";
  import { pressRow } from "../../edit/listDrag";
  import type { EditState } from "../../edit/editState.svelte";
  import type { MenuItem } from "../../shell/menu";
  import { TEMPLATE_KINDS, roleLabel, slideRoles, templateSlide, type TemplateKind } from "../../edit/themeTemplates";
  import { useLive } from "../../live";

  let { conn, st }: { conn: Connection; st: EditState } = $props();
  const live = useLive();
  const liveSlideId = live.liveSlideId;

  let pres = $derived(st.presentation);
  /** A prop is open (prop-document mode): one slide, no slide or group operations. */
  let isProp = $derived(!!session.open?.prop);
  /** A theme is open: its template slides can be added, duplicated, moved and deleted. */
  let isTheme = $derived(!!session.open?.theme);
  /** One fixed slide (a prop or stage layout): no slide operations. */
  let fixed = $derived(isProp && !isTheme);
  let sections = $derived(pres?.sections ?? []);
  let collapsed = $state<Record<string, boolean>>({});
  let width = $state(200);
  let listEl = $state<HTMLElement | null>(null);
  let line = $state<{ y: number; left: number; width: number } | null>(null);
  let aspect = $derived(aspectRatio(pres));
  let thumbW = $derived(thumbWidthFor(Math.max(80, width - 40), typeof devicePixelRatio === "number" ? devicePixelRatio : 1));

  /** Slides in display order with their numbers. */
  let rows = $derived.by(() => {
    const out: { id: string; section: Section; n: number }[] = [];
    let n = 0;
    for (const s of sections) for (const sl of s.slides ?? []) out.push({ id: sl.id, section: s, n: ++n });
    return out;
  });

  function src(sectionIdx: number, slideIdx: number): string {
    const sl = sections[sectionIdx]?.slides?.[slideIdx];
    if (!pres || !sl) return "";
    // The canvas size too: a canvas change reshapes every picture (A05-5).
    const c = pres.canvasSize;
    return `${conn.rest.thumbnailUrl(pres.id, sl.id, thumbW)}&v=${slideHash(sl)}-${c?.width ?? 0}x${c?.height ?? 0}`;
  }

  async function pick(id: string) {
    await st.finishText?.();
    st.selectSlide(id);
  }

  async function addSlide() {
    if (!pres) return;
    await st.finishText?.();
    const before = new Set(rows.map((r) => r.id));
    const ok = await st.run("New Slide", addSlideOps(pres, st.slideId, uuid));
    if (ok) {
      const added = rows.find((r) => !before.has(r.id));
      if (added) st.selectSlide(added.id);
    }
  }

  /** Theme mode: a new template slide of a starter layout, after the selected one. */
  async function addTemplate(kind: TemplateKind) {
    if (!pres) return;
    await st.finishText?.();
    const sec = sections[0];
    if (!sec) return;
    const taken = new Set(rows.map((r) => (sec.slides ?? []).find((x) => x.id === r.id)?.label?.text ?? ""));
    const base = TEMPLATE_KINDS.find((k) => k.kind === kind)!.label;
    let name = base;
    for (let i = 2; taken.has(name); i++) name = `${base} ${i}`;
    const slide = templateSlide(kind, pres.canvasSize, uuid, name);
    const after = st.slideId && (sec.slides ?? []).some((x) => x.id === st.slideId) ? st.slideId : sec.slides?.[sec.slides.length - 1]?.id;
    if (await st.run("New Template Slide", [{ type: "addSlide", section: sec.id, ...(after ? { after } : {}), slide }])) st.selectSlide(slide.id);
  }

  function addMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    shell.contextMenu = {
      x: r.left,
      y: r.bottom + 2,
      items: TEMPLATE_KINDS.map((k) => ({ type: "item" as const, id: `tpl-${k.kind}`, label: `${k.label} template slide`, run: () => void addTemplate(k.kind) })),
    };
  }

  function menu(e: MouseEvent, id: string) {
    e.preventDefault();
    if (!pres || fixed) return;
    if (isTheme) {
      shell.contextMenu = {
        x: e.clientX,
        y: e.clientY,
        items: [
          { type: "item", id: "dup", label: "Duplicate", run: () => void duplicate(id) },
          { type: "item", id: "del", label: "Delete…", disabled: rows.length <= 1, title: rows.length <= 1 ? "A theme keeps at least one template slide" : undefined, run: () => void delTemplate(id) },
        ],
      };
      return;
    }
    const here = sections.find((s) => (s.slides ?? []).some((x) => x.id === id));
    const items: MenuItem[] = [
      { type: "item", id: "dup", label: "Duplicate", run: () => void duplicate(id) },
      { type: "item", id: "del", label: "Delete", disabled: rows.length <= 1, title: rows.length <= 1 ? "A presentation keeps at least one slide" : undefined, run: () => void del(id) },
      { type: "separator" },
      {
        type: "item",
        id: "move",
        label: "Move to Group",
        submenu: sections.map((s, i) => ({
          type: "item" as const,
          id: `g${i}`,
          label: s.name || "Group",
          disabled: s.id === here?.id,
          run: () => void moveToSection(id, s),
        })),
      },
    ];
    shell.contextMenu = { x: e.clientX, y: e.clientY, items };
  }

  async function duplicate(id: string) {
    const before = new Set(rows.map((r) => r.id));
    if (await st.run("Duplicate Slide", [{ type: "duplicateSlide", slide: id }])) {
      const added = rows.find((r) => !before.has(r.id));
      if (added) st.selectSlide(added.id);
    }
  }

  async function del(id: string) {
    const i = rows.findIndex((r) => r.id === id);
    const next = rows[i + 1]?.id ?? rows[i - 1]?.id ?? null;
    // The neighbour first: the new document arrives (and is reconciled) before `run` returns,
    // and would find the edited slide gone ("deleted", the first slide).
    const was = st.slideId === id;
    if (was) st.selectSlide(next);
    const ok = await st.run("Delete Slide", [{ type: "deleteSlide", slide: id }]);
    if (!ok && was && st.slideId === next) st.selectSlide(id);
  }

  /** Theme mode: deletes a template slide after saying which songs follow it (they keep their
   * look, as plain copies). */
  async function delTemplate(id: string) {
    const sl = sections.flatMap((s) => s.slides ?? []).find((x) => x.id === id);
    const template = sl?.templateOrigin?.slide;
    const name = sl?.label?.text || "this template slide";
    let users: string[] = [];
    if (pres && template) {
      try {
        users = (await conn.rest.themeUsedBy(pres.id, template)).filter((u) => u.kind === "presentation").map((u) => u.name);
      } catch {
        users = [];
      }
    }
    const msg = users.length
      ? `${users.length === 1 ? "1 song follows" : `${users.length} songs follow`} it (${users.slice(0, 3).join(", ")}${users.length > 3 ? ", …" : ""}). They keep their look but stop following the theme.`
      : "No song follows it.";
    if (!(await shell.confirm(`Delete “${name}”`, msg, "Delete", true))) return;
    await del(id);
  }

  async function moveToSection(id: string, s: Section) {
    const slides = s.slides ?? [];
    const last = slides[slides.length - 1];
    await st.run("Move to Group", [{ type: "moveSlide", slide: id, toSection: s.id, ...(last ? { after: last.id } : {}) }]);
  }

  function press(e: PointerEvent, id: string) {
    if (!listEl || fixed) return;
    pressRow(e, {
      list: listEl,
      rows: "[data-slide-row]",
      line: (l) => (line = l),
      drop: (index) => void drop(id, index),
    });
  }

  /** Dropped before display row `index`: into that row's section, after its predecessor there. */
  async function drop(id: string, index: number) {
    const visible = [...(listEl?.querySelectorAll<HTMLElement>("[data-slide-row]") ?? [])].map((r) => r.dataset.slideRow!);
    const target = visible[index];
    let op: PresentationOp | null = null;
    if (target === id) return;
    if (target) {
      const sec = sections.find((s) => (s.slides ?? []).some((x) => x.id === target))!;
      const list = (sec.slides ?? []).map((x) => x.id).filter((x) => x !== id);
      const at = list.indexOf(target);
      const after = at > 0 ? list[at - 1] : undefined;
      op = { type: "moveSlide", slide: id, toSection: sec.id, ...(after ? { after } : {}) };
    } else {
      const sec = sections[sections.length - 1];
      if (!sec) return;
      const list = (sec.slides ?? []).map((x) => x.id).filter((x) => x !== id);
      const after = list[list.length - 1];
      op = { type: "moveSlide", slide: id, toSection: sec.id, ...(after ? { after } : {}) };
    }
    await st.run("Move Slide", [op]);
  }
</script>

<div class="slides" bind:clientWidth={width}>
  <header>
    <span class="pname" title={pres?.name}>{isProp ? `${session.open?.stageLayout ? "Stage Layout" : session.open?.theme ? "Theme" : "Prop"}: ${pres?.name ?? ""}` : (pres?.name ?? "")}</span>
    {#if isTheme}
      <button class="add" onclick={addMenu} disabled={!pres} title="New template slide" aria-label="New template slide" data-testid="theme-add-slide"><Plus size={20} /></button>
    {/if}
    {#if isProp}
      <button class="done" onclick={() => session.closeProp()} title={session.open?.stageLayout ? "Stop editing the stage layout" : session.open?.theme ? "Stop editing the theme" : "Stop editing the prop"} data-testid="prop-done">Done</button>
    {:else}
      <button class="add" onclick={addSlide} disabled={!pres} title="New slide" aria-label="New slide"><Plus size={20} /></button>
    {/if}
  </header>
  <div class="list" bind:this={listEl} data-edit-slides>
    {#each sections as s, si (s.id)}
      {@const color = cssColor(s.color, 1) ?? "#9a9a9a"}
      <button class="ghead" onclick={() => (collapsed[s.id] = !collapsed[s.id])} aria-expanded={!collapsed[s.id]} style:--gc={color} data-section={s.id}>
        {#if collapsed[s.id]}<ChevronRight size={15} />{:else}<ChevronDown size={15} />{/if}
        <span>{s.name || "Group"}</span>
      </button>
      {#if !collapsed[s.id]}
        {#each s.slides ?? [] as sl, i (sl.id)}
          {@const n = rows.find((r) => r.id === sl.id)?.n ?? 0}
          <button
            class="row"
            class:sel={st.slideId === sl.id}
            class:disabled={sl.enabled === false}
            onclick={() => void pick(sl.id)}
            oncontextmenu={(e) => menu(e, sl.id)}
            onpointerdown={(e) => press(e, sl.id)}
            data-slide-row={sl.id}
            aria-current={st.slideId === sl.id ? "true" : undefined}
            title={`Slide ${n}${sl.label?.text ? ` · ${sl.label.text}` : ""}`}
          >
            <div class="frame" style:aspect-ratio={aspect}>
              {#key src(si, i)}<img src={src(si, i)} alt="" draggable="false" decoding="async" />{/key}
            </div>
            <div class="cap">
              <span class="n" class:live={$liveSlideId === sl.id}>{n}.</span>
              {#if sl.label?.text}<span class="lbl" style:color={cssColor(sl.label.color, 1) ?? undefined}>{sl.label.text}</span>{/if}
            </div>
            {#if isTheme}
              {@const roles = slideRoles(sl)}
              <div class="roles" data-testid="template-roles">{roles.length ? roles.map(roleLabel).join(" · ") : "No slots"}</div>
            {/if}
          </button>
        {/each}
      {/if}
    {/each}
  </div>
  {#if line}<div class="line" style:top={`${line.y}px`} style:left={`${line.left}px`} style:width={`${line.width}px`}></div>{/if}
</div>

<style>
  .done {
    height: 22px;
    padding: 0 10px;
    border: 1px solid var(--accent);
    border-radius: 2px;
    background: var(--accent-dim);
    color: var(--text-strong);
    font-size: 12px;
  }
  .slides {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface-panel);
  }
  header {
    flex: none;
    display: flex;
    align-items: center;
    height: 34px;
    padding: 0 4px 0 8px;
    background: var(--surface-panel);
    border-bottom: 1px solid var(--line);
  }
  .pname {
    flex: 1;
    font-size: 14px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .add {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 3px;
    background: transparent;
    color: var(--text);
  }
  .add:hover {
    background: var(--surface-raised);
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 8px;
  }
  .ghead {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 30px;
    padding: 0 8px;
    border: 0;
    background: transparent;
    color: var(--gc);
    font-weight: 700;
    font-size: 13.5px;
    text-align: left;
  }
  .ghead span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    filter: brightness(1.25);
  }
  .row {
    display: block;
    width: 100%;
    padding: 8px 14px 4px 18px;
    border: 0;
    background: transparent;
    text-align: left;
    color: var(--text);
  }
  .row.sel {
    background: var(--accent-sel);
  }
  .row.disabled .frame {
    opacity: 0.4;
  }
  .frame {
    position: relative;
    width: 100%;
    background-color: var(--surface-app);
    background-image: conic-gradient(var(--surface-sunk) 25%, var(--surface-app) 0 50%, var(--surface-sunk) 0 75%, var(--surface-app) 0);
    background-size: 12px 12px;
    border: 2px solid var(--line);
    overflow: hidden;
  }
  .row.sel .frame {
    border-color: var(--live);
  }
  .frame img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }
  .cap {
    display: flex;
    gap: 6px;
    align-items: center;
    height: 20px;
    font-size: 12.5px;
    font-weight: 600;
  }
  .roles {
    margin-top: -2px;
    font-size: 11.5px;
    color: var(--text-meta);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .n.live {
    color: var(--live);
  }
  .lbl {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: 400;
  }
  .line {
    position: fixed;
    height: 2px;
    background: var(--accent-bright);
    pointer-events: none;
    z-index: 30;
  }
</style>
