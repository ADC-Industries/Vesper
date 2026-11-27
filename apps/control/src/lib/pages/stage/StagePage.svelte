<script lang="ts">
  // The Stage display page (plan 4.6): layout cards (+ "New layout"), the stage screens with a
  // layout picker each, the large live picture of the selected stage screen, the stage message
  // (Show / Hide / Flash, presets), and "On this layout": checkboxes that add or remove the
  // layout's data-link elements (compose.ts) with their settings, and "Design freely…" for the
  // slide editor. Picking a layout card shows it on the selected stage screen. The Show page's
  // compact Stage tab is show/CompactStage.
  import { onDestroy } from "svelte";
  import Plus from "lucide-svelte/icons/plus";
  import X from "lucide-svelte/icons/x";
  import type { Accidentals, ChordView, DataLink, StageLayoutEntry } from "$protocol";
  import { chordViewText } from "../../edit/chords";
  import type { Connection, PreviewImage } from "../../server";
  import PageFrame from "../PageFrame.svelte";
  import { Button, Chip, NewButton, SectionLabel, Slider } from "../../ui";
  import { stageState } from "../../state/stage.svelte";
  import { timersState } from "../../state/timers.svelte";
  import { shell } from "../../state/shell.svelte";
  import { useLive } from "../../live";
  import type { MenuItem } from "../../shell/menu";
  import { BUILT_IN_LAYOUT, MAX_MESSAGE, addPreset, removePreset } from "./stage";
  import {
    STAGE_ELEMENTS,
    composition,
    compositionText,
    currentTextSize,
    layoutClock24,
    layoutTimer,
    relinkOps,
    textSizeOps,
    toggleOps,
    type StageElementKind,
  } from "./compose";

  let { conn }: { conn: Connection } = $props();
  const live = useLive();
  const stageScreens = live.stageScreens;
  const stageMessage = live.stageMessage;
  const screensEnabled = live.screensEnabled;
  const screens = live.screens;
  // svelte-ignore state_referenced_locally
  const status = conn.status;
  // svelte-ignore state_referenced_locally
  const timerValues = conn.timerValues;

  let layouts = $derived(stageState.stageLayoutDocs);
  let screen = $derived($stageScreens.find((s) => s.screen === stageState.pickedScreen) ?? $stageScreens[0] ?? null);
  let layout = $derived<StageLayoutEntry | null>(
    layouts.find((l) => l.id === stageState.pickedLayout) ?? layouts.find((l) => l.id === screen?.layout) ?? layouts[0] ?? null,
  );
  let onScreens = $derived(layout ? $stageScreens.filter((s) => s.layout === layout!.id) : []);
  let comp = $derived(composition(layout?.slide));
  let timerId = $derived(layoutTimer(layout?.slide));
  let clock24 = $derived(layoutClock24(layout?.slide));
  let textSize = $derived(currentTextSize(layout?.slide));
  let online = $derived($status.phase === "online");
  // The layout's chord view (batch 16): capo, transposition, spelling, numbers.
  let view = $derived<ChordView>(layout?.chords ?? {});
  let hasChordLinks = $derived((layout?.slide?.elements ?? []).some((e) => e.dataLink?.type === "chordChart" || e.dataLink?.type === "songKey"));
  function setView(patch: Partial<ChordView>) {
    const l = layout;
    if (!l) return;
    const next: ChordView = { ...(l.chords ?? {}), ...patch };
    for (const k of Object.keys(next) as (keyof ChordView)[]) if (next[k] === undefined) delete next[k];
    const { revision, ...doc } = l;
    void guard("Chords", () => conn.rest.updateStageLayout({ ...doc, chords: next }, revision));
  }

  function routeOf(id: string): string {
    const s = $screens.find((x) => x.id === id);
    if (!s) return "not running";
    if (!s.enabled || !$screensEnabled.stage) return "off";
    const kinds = [...new Set(s.outputs.map((o) => (o.kind === "idn" ? "IDN" : o.kind.toUpperCase())))];
    return kinds.length ? kinds.join(" · ") : "no output";
  }

  // ---- layouts ------------------------------------------------------------------------------------

  function pick(l: StageLayoutEntry) {
    stageState.pickedLayout = l.id;
    if (screen && screen.layout !== l.id) void stageState.setStageLayout(screen.screen, l.id);
  }

  function pickScreen(id: string) {
    stageState.pickedScreen = id;
    stageState.pickedLayout = null;
  }

  function newMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const items: MenuItem[] = [
      ...stageState.stageTemplates.map((t) => ({ type: "item" as const, id: `tpl-${t.id}`, label: t.name, run: () => void create(t.id) })),
      { type: "separator" },
      { type: "item", id: "blank", label: "Blank layout", run: () => void create(null) },
    ];
    shell.contextMenu = { x: r.left, y: r.bottom + 2, items };
  }

  async function create(template: string | null) {
    const e = await stageState.createStageLayout(template);
    if (e) stageState.pickedLayout = e.id;
  }

  async function guard(what: string, fn: () => Promise<unknown>) {
    try {
      await fn();
    } catch (e) {
      conn.pushNotice("error", `${what} failed: ${e instanceof Error ? e.message : String(e)}`);
    }
    await stageState.load();
  }

  function cardMenu(e: MouseEvent, l: StageLayoutEntry) {
    e.preventDefault();
    shell.contextMenu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { type: "item", id: "design", label: "Design freely…", run: () => void stageState.openStageLayout(l.id) },
        {
          type: "item",
          id: "rename",
          label: "Rename…",
          run: async () => {
            const n = await shell.askText("Rename Stage Layout", l.name ?? "", "Rename");
            if (n && n !== l.name) await guard("Rename", () => conn.rest.updateStageLayout({ ...l, name: n }, l.revision));
          },
        },
        {
          type: "item",
          id: "dup",
          label: "Duplicate",
          run: () => {
            const { id: _id, revision: _r, sortKey: _s, import: _i, ...rest } = l;
            void guard("Duplicate", () => conn.rest.createStageLayout({ ...rest, name: `${l.name ?? "Layout"} copy` }));
          },
        },
        { type: "separator" },
        {
          type: "item",
          id: "del",
          label: "Delete",
          run: async () => {
            if (await shell.confirm("Delete Stage Layout", `Delete “${l.name || "Layout"}”? Stage screens showing it go back to their default.`))
              await guard("Delete", () => conn.rest.deleteStageLayout(l.id, l.revision));
          },
        },
      ],
    };
  }

  // ---- composing ----------------------------------------------------------------------------------

  const size = () => ({ width: layout?.size?.width ?? 1920, height: layout?.size?.height ?? 1080 });

  function toggle(kind: StageElementKind, on: boolean) {
    if (!layout) return;
    const first = timersState.timerDocs[0];
    const ops = toggleOps(layout.slide, kind, on, { ...size(), timer: first ? { id: first.id, ...(first.name ? { name: first.name } : {}) } : null, use24Hour: timersState.clock24 });
    const label = STAGE_ELEMENTS.find((x) => x.kind === kind)!.label;
    void stageState.editStageLayout(layout.id, on ? `Add ${label}` : `Remove ${label}`, ops);
  }

  function setTimer(id: string) {
    if (!layout) return;
    const t = timersState.timerDocs.find((x) => x.id === id);
    const ops = relinkOps(layout.slide, "timer", (old): DataLink => {
      const { timer: _old, ...base } = old.type === "timer" ? old : { type: "timer" as const };
      return t ? { ...base, timer: { id: t.id, ...(t.name ? { name: t.name } : {}) } } : base;
    });
    void stageState.editStageLayout(layout.id, "Stage Timer", ops);
  }

  function setClock(use24: boolean) {
    if (!layout) return;
    const ops = relinkOps(layout.slide, "clock", (old) => ({ ...(old.type === "clock" ? old : {}), type: "clock", format: { ...(old.type === "clock" ? old.format : {}), use24Hour: use24 } }));
    void stageState.editStageLayout(layout.id, "Clock Format", ops);
  }

  function setSize(px: number) {
    if (layout) void stageState.editStageLayout(layout.id, "Text Size", textSizeOps(layout.slide, Math.round(px)));
  }

  // ---- the live picture ---------------------------------------------------------------------------

  let img = $state.raw<PreviewImage | null>(null);
  let subscribed: string | null = null;
  let liveId = $derived(screen && layout && screen.layout === layout.id && $screens.some((s) => s.id === screen!.screen) ? screen.screen : null);
  $effect(() => {
    const id = liveId;
    if (id === subscribed) return;
    if (subscribed) conn.socket.unsubscribePreview(subscribed);
    subscribed = id;
    img = id ? (conn.previews.get(id) ?? null) : null;
    if (id) conn.socket.subscribePreview(id, 10, 960);
  });
  // svelte-ignore state_referenced_locally
  const off = conn.onPreview((p) => {
    if (p.screen === subscribed) img = p;
  });
  onDestroy(() => {
    off();
    if (subscribed) conn.socket.unsubscribePreview(subscribed);
  });

  // ---- stage message ------------------------------------------------------------------------------

  let text = $state("");
  let flash = $state(false);

  async function show(t = text) {
    if (await stageState.stageMessage(t, flash)) text = t.trim();
  }
</script>

<PageFrame page="stage">
  <div class="layout" data-testid="stage-page">
    <aside class="side left" aria-label="Stage layouts">
      <SectionLabel>Layouts</SectionLabel>
      {#each layouts as l (l.id)}
        <button class="lcard" class:on={layout?.id === l.id} aria-pressed={layout?.id === l.id} onclick={() => pick(l)} oncontextmenu={(e) => cardMenu(e, l)} data-testid="stage-layout-card">
          <span class="lname">{l.name || "Layout"}</span>
          <span class="ldesc">{compositionText(l.slide)}</span>
        </button>
      {:else}
        <div class="lcard on static">
          <span class="lname">{BUILT_IN_LAYOUT}</span>
          <span class="ldesc">Built in · make a layout to change it</span>
        </div>
      {/each}
      <NewButton label="New layout" onclick={newMenu} data-testid="new-stage-layout" />

      <div class="gap"></div>
      <SectionLabel>Stage screens</SectionLabel>
      {#each $stageScreens as s (s.screen)}
        <div class="scard" class:on={screen?.screen === s.screen}>
          <button class="shead" onclick={() => pickScreen(s.screen)} aria-pressed={screen?.screen === s.screen} title="Show this screen in the preview">
            <span class="sname">{s.name || "Stage"}</span>
            <span class="sroute">{routeOf(s.screen)}</span>
          </button>
          <select value={s.layout ?? ""} onchange={(e) => void stageState.setStageLayout(s.screen, e.currentTarget.value || null)} aria-label={`Layout on ${s.name}`} data-testid="stage-layout-select">
            {#if !layouts.length}<option value="">{BUILT_IN_LAYOUT}</option>{/if}
            {#each layouts as l (l.id)}<option value={l.id}>{l.name || "Layout"}</option>{/each}
          </select>
        </div>
      {:else}
        <p class="none">No stage screens.</p>
      {/each}
      <button class="link" onclick={() => shell.appAction("pageScreens")}>Configure screens</button>
    </aside>

    <main class="main">
      <div class="head">
        <h1>{layout ? `${layout.name || "Layout"} layout` : BUILT_IN_LAYOUT}</h1>
        {#if liveId && img}<Chip kind="live">LIVE</Chip>{/if}
        <span class="where">{onScreens.length ? `Showing on ${onScreens.map((s) => s.name || "Stage").join(" and ")}` : layout ? "Not on a stage screen" : "Shown while there are no layouts"}</span>
        <span class="grow"></span>
        {#if $stageMessage}<span class="onstage" title={$stageMessage.text}>Message on stage{$stageMessage.flash ? " · flashed" : ""}</span>{/if}
      </div>
      <div class="frame" style:aspect-ratio={`${layout?.size?.width ?? 1920} / ${layout?.size?.height ?? 1080}`} data-testid="stage-preview">
        {#if liveId && img}
          <img src={img.url} alt={`Live picture of ${screen?.name ?? "the stage screen"}`} />
        {:else if layout}
          <img src={conn.rest.stageLayoutThumbnailUrl(layout.id, 960, layout.revision)} alt={`${layout.name} as designed`} />
          <span class="tag">{liveId ? (online ? "Waiting for picture…" : "Offline · as designed") : "As designed"}</span>
        {:else}
          <img src={conn.rest.stageTemplateThumbnailUrl("currentNext", 960)} alt={BUILT_IN_LAYOUT} />
        {/if}
      </div>

      <div class="msgrow">
        <label class="mfield">
          Stage message (only the stage sees this)
          <input
            type="text"
            bind:value={text}
            maxlength={MAX_MESSAGE}
            placeholder="Message to the performers"
            onkeydown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") void show();
            }}
            data-testid="stage-message-text"
          />
        </label>
        <Button variant="primary" onclick={() => void show()} disabled={!text.trim()} data-testid="stage-message-show">Show</Button>
        <Button onclick={() => void stageState.stageMessageClear()} disabled={!$stageMessage} data-testid="stage-message-clear">Hide</Button>
        <label class="check"><input type="checkbox" bind:checked={flash} />Flash</label>
      </div>
      <div class="presets" aria-label="Message presets">
        {#each stageState.stagePresets as p, i (`${i}:${p}`)}
          <span class="chip" class:live={$stageMessage?.text === p}>
            <button class="ctext" onclick={() => void show(p)} title="Show on the stage screens">{p}</button>
            <button class="cx" onclick={() => stageState.setStagePresets(removePreset(stageState.stagePresets, i))} aria-label={`Remove preset ${p}`}><X size={11} /></button>
          </span>
        {/each}
        <Button size="sm" variant="ghost" icon={Plus} onclick={() => stageState.setStagePresets(addPreset(stageState.stagePresets, text))} disabled={!text.trim()}>Save as preset</Button>
      </div>
    </main>

    <aside class="side right" aria-label="Layout elements">
      <SectionLabel>On this layout</SectionLabel>
      {#each STAGE_ELEMENTS as el (el.kind)}
        <label class="erow" class:dis={!layout}>
          <input type="checkbox" checked={comp[el.kind]} disabled={!layout} onchange={(e) => toggle(el.kind, e.currentTarget.checked)} data-testid={`stage-el-${el.kind}`} />
          <span class="ename">{el.label}</span>
          {#if el.kind === "timer" && timerId !== null}
            <span class="ehint">{timerId ? (timersState.timerDocs.find((t) => t.id === timerId)?.name ?? "missing timer") : "last started"}</span>
          {/if}
        </label>
      {/each}
      {#if !layout}<p class="none">The built-in layout can't change. Make a layout with New layout.</p>{/if}

      <div class="gap"></div>
      <SectionLabel>Element settings</SectionLabel>
      <label class="f">
        Timer to show
        <select value={timerId ?? ""} disabled={timerId === null} onchange={(e) => setTimer(e.currentTarget.value)}>
          <option value="">Most recently started</option>
          {#each timersState.timerDocs as t (t.id)}
            {@const v = $timerValues[t.id]}
            <option value={t.id}>{t.name || "Timer"}{v ? ` · ${v.display}` : ""}</option>
          {/each}
        </select>
      </label>
      <button class="link" onclick={() => shell.appAction("pageTimers")}>Create or edit timers in Timers</button>
      <label class="f">
        Clock format
        <select value={clock24 ? "24" : "12"} disabled={clock24 === null} onchange={(e) => setClock(e.currentTarget.value === "24")}>
          <option value="12">12-hour</option>
          <option value="24">24-hour</option>
        </select>
      </label>
      <div class="f">
        <span>Current slide text size</span>
        {#key `${layout?.revision}`}
          <Slider label="Current slide text size" value={textSize ?? 110} min={40} max={200} step={2} disabled={textSize === null} format={(v) => `${v} px`} onchange={setSize} />
        {/key}
      </div>

      <div class="gap"></div>
      <SectionLabel>Chords for musicians</SectionLabel>
      <div class="chordview" data-testid="stage-chords">
        <label class="f half">
          Capo
          <select value={view.capo ?? 0} disabled={!layout} onchange={(e) => setView({ capo: Number(e.currentTarget.value) || undefined })} data-testid="stage-capo">
            {#each Array.from({ length: 10 }, (_, i) => i) as c (c)}<option value={c}>{c ? `Fret ${c}` : "No capo"}</option>{/each}
          </select>
        </label>
        <label class="f half">
          Transpose
          <select value={view.transpose ?? 0} disabled={!layout} onchange={(e) => setView({ transpose: Number(e.currentTarget.value) || undefined })} data-testid="stage-transpose">
            {#each Array.from({ length: 13 }, (_, i) => i - 6) as t (t)}<option value={t}>{t ? `${t > 0 ? "+" : ""}${t} semitone${Math.abs(t) === 1 ? "" : "s"}` : "None"}</option>{/each}
          </select>
        </label>
        <label class="f half">
          Sharps / flats
          <select value={view.accidentals ?? "auto"} disabled={!layout} onchange={(e) => setView({ accidentals: e.currentTarget.value === "auto" ? undefined : (e.currentTarget.value as Accidentals) })} data-testid="stage-accidentals">
            <option value="auto">By key</option>
            <option value="sharps">Sharps</option>
            <option value="flats">Flats</option>
          </select>
        </label>
        <label class="f half">
          Chords as
          <select value={view.notation ?? "letters"} disabled={!layout} onchange={(e) => setView({ notation: e.currentTarget.value === "nashville" ? "nashville" : undefined })} data-testid="stage-notation">
            <option value="letters">Letters</option>
            <option value="nashville">Numbers</option>
          </select>
        </label>
      </div>
      <p class="hint">{chordViewText(view)}{hasChordLinks ? "" : ". This layout has no chord chart: start one from the Musician (Chords) template."}</p>

      <div class="gap"></div>
      <Button onclick={() => layout && void stageState.openStageLayout(layout.id)} disabled={!layout} title="Open the layout in the slide editor: place, size and style anything" data-testid="design-freely">
        Design freely…
      </Button>
      <p class="hint">Move, resize and style the elements in the slide editor; the checkboxes add and remove them.</p>
    </aside>
  </div>
</PageFrame>

<style>
  .chordview {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px 8px;
  }
  .chordview .half select {
    width: 100%;
  }
  .layout {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-wrap: wrap;
    overflow: auto;
  }
  .side {
    max-width: 100%;
    max-height: 100%;
    box-sizing: border-box;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    background: var(--surface-panel);
  }
  .left {
    flex: 0 0 240px;
    border-right: 1px solid var(--line);
  }
  .right {
    flex: 0 0 300px;
    border-left: 1px solid var(--line);
  }
  .gap {
    height: 8px;
  }
  .lcard {
    display: flex;
    flex-direction: column;
    gap: 2px;
    text-align: left;
    padding: 10px;
    border-radius: var(--r-card);
    border: 1px solid var(--line);
    background: var(--surface-raised);
    color: var(--text);
  }
  .lcard:hover:not(.static) {
    border-color: var(--line-hover);
  }
  .lcard.on {
    background: var(--accent-sel);
    border-color: var(--accent-ring);
  }
  .lname {
    font-weight: 600;
  }
  .ldesc {
    color: var(--text-label);
    font-size: var(--fs-xs);
  }
  .scard {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 8px;
    border-radius: var(--r-card);
    border: 1px solid var(--line);
    background: var(--surface-raised);
    font-size: 12.5px;
  }
  .scard.on {
    border-color: var(--accent-ring);
  }
  .shead {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    text-align: left;
  }
  .sname {
    font-weight: 600;
  }
  .sroute {
    color: var(--text-label);
    font-size: var(--fs-xs);
  }
  .scard select,
  .f select {
    width: 100%;
  }
  .link {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-link);
    font-size: var(--fs-sm);
  }
  .link:hover {
    color: var(--accent-link-hover);
    text-decoration: underline;
  }
  .none,
  .hint {
    margin: 0;
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .main {
    flex: 999 1 520px;
    min-width: 0;
    max-height: 100%;
    box-sizing: border-box;
    overflow-y: auto;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    background: var(--surface-backdrop);
  }
  .main > * {
    width: 100%;
    max-width: 900px;
    align-self: center;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .head h1 {
    margin: 0;
    font-size: var(--fs-title);
    font-weight: 600;
  }
  .where {
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .frame {
    position: relative;
    background: #000;
    border-radius: var(--r);
    box-shadow: 0 0 0 1px var(--line);
    overflow: hidden;
  }
  .frame img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .tag {
    position: absolute;
    left: 8px;
    top: 8px;
    padding: 2px 6px;
    border-radius: var(--r-xs);
    background: rgba(0, 0, 0, 0.6);
    color: var(--text-2);
    font-size: var(--fs-2xs);
    font-weight: 700;
    letter-spacing: 0.04em;
  }
  .grow {
    flex: 1;
  }
  .onstage {
    padding: 2px 8px;
    border-radius: var(--r-xs);
    background: var(--live-soft);
    color: var(--live);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .msgrow {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    align-items: flex-end;
  }
  .mfield {
    flex: 1 1 260px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .mfield input {
    color: var(--text);
    font-size: var(--fs);
    padding: 7px 8px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    padding-bottom: 6px;
  }
  .check input {
    width: 16px;
    height: 16px;
  }
  .presets {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--line-strong);
    border-radius: var(--r-pill);
    background: var(--surface-raised);
    overflow: hidden;
  }
  .chip.live {
    border-color: var(--live);
  }
  .ctext {
    padding: 4px 4px 4px 10px;
    border: 0;
    background: none;
    color: var(--text);
    font-size: var(--fs-sm);
  }
  .cx {
    display: flex;
    padding: 4px 8px 4px 4px;
    border: 0;
    background: none;
    color: var(--text-label);
  }
  .cx:hover {
    color: var(--text);
  }
  .erow {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: var(--r);
    border: 1px solid var(--line);
    background: var(--surface-raised);
  }
  .erow.dis {
    opacity: 0.55;
  }
  .erow input {
    width: 16px;
    height: 16px;
  }
  .ename {
    flex: 1;
  }
  .ehint {
    color: var(--text-meta);
    font-size: var(--fs-xs);
  }
  .f {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-label);
    font-size: var(--fs-xs);
  }
</style>
