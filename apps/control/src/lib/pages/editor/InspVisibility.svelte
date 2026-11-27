<script lang="ts">
  // Inspector, Shape tab (one object): its live-data link settings (a timer's Value / Elapsed /
  // Remaining, a presentation name's layer) and its visibility conditions (all must hold on the
  // screens; the canvas shows every object). Plan G16, docs/TRANSITIONS-PLAN.md section 5.
  import X from "lucide-svelte/icons/x";
  import type { CountdownSource, DataLink, Element, MediaCondition, PlaylistItemWhich, Slide, TimerCondition, TimerShow, VisibilityCondition } from "$protocol";
  import type { EditState } from "../../edit/editState.svelte";
  import { elementName } from "../../edit/elements";
  import { setElements } from "../../edit/ops";
  import { shown, type Sent } from "../../edit/optimistic";
  import {
    CONDITION_KINDS,
    CONDITION_LAYERS,
    MEDIA_STATES,
    PLAYLIST_ITEMS,
    TIMER_SHOWS,
    TIMER_STATES,
    activeOf,
    conditionLabel,
    inputLink,
    newCondition,
    removeCondition,
    replaceCondition,
    timerShowLink,
    withActive,
    withCondition,
    type ConditionKind,
  } from "../../edit/visibility";
  import { dataLinkLabel } from "../stage/stage";
  import { timersState } from "../../state/timers.svelte";
  import { showDocsState as showDocs } from "../../state/showDocs.svelte";
  import { sourcesState } from "../../state/sources.svelte";

  let { st, slide, el }: { st: EditState; slide: Slide; el: Element } = $props();

  // Each edit sends the whole list: the list just sent, until that save settled, is what the
  // next edit builds on, so two quick edits don't undo each other (F11-3, as D10-2/E10-5).
  let sentList = $state.raw<Sent<VisibilityCondition[]> | null>(null);
  let list = $derived(shown(sentList, `${st.slideId}/${el.id}`, el.visibility ?? []));
  /** The slide's other objects with text (what "Object has text" can name). */
  let others = $derived((slide.elements ?? []).filter((e) => e.id !== el.id && e.kind?.type === "text"));
  let link = $derived(el.dataLink ?? null);
  const nameOf = (id: string) => {
    const e = (slide.elements ?? []).find((x) => x.id === id);
    return e ? elementName(e) : "Missing object";
  };
  const messageName = (id: string) => showDocs.messageDocs.find((m) => m.id === id)?.name ?? "Message";
  /** The show's video inputs (what "Video input live" names), read on first use. */
  let inputs = $derived(sourcesState.inputs);
  $effect(() => sourcesState.ensure());
  const inputName = (id: string) => {
    const v = sourcesState.inputs.find((x) => x.id === id);
    return v ? v.name || "Video input" : undefined;
  };
  const names = { element: nameOf, message: messageName, input: inputName };

  function save(label: string, next: VisibilityCondition[]) {
    const slide = st.slideId;
    if (!slide) return;
    const mine = { key: `${slide}/${el.id}`, value: next };
    sentList = mine;
    void st.run(label, setElements(slide, [{ id: el.id, patch: { visibility: next } }])).finally(() => {
      if (sentList === mine) sentList = null;
    });
  }
  function setLink(label: string, next: DataLink) {
    if (!st.slideId) return;
    void st.run(label, setElements(st.slideId, [{ id: el.id, patch: { dataLink: next } }]));
  }
  function add(kind: string) {
    const c = newCondition(kind as ConditionKind, others, inputs);
    if (c) save("Add Condition", withCondition(list, c));
  }
  function change(i: number, c: VisibilityCondition) {
    save("Condition", replaceCondition(list, i, c));
  }
  function timerRef(id: string) {
    const t = timersState.timerDocs.find((x) => x.id === id);
    return t ? { id: t.id, ...(t.name ? { name: t.name } : {}) } : undefined;
  }
  function messageRef(id: string) {
    const m = showDocs.messageDocs.find((x) => x.id === id);
    return m ? { id: m.id, ...(m.name ? { name: m.name } : {}) } : undefined;
  }
</script>

{#if link && (link.type === "timer" || link.type === "presentationName" || link.type === "songArtwork")}
  <section class="sec" data-live-link>
    <h4>Live Data: {dataLinkLabel(link)}</h4>
    {#if link.type === "timer"}
      {@const tl = link}
      <div class="line">
        <span class="lab">Show</span>
        <select value={tl.show ?? "value"} onchange={(e) => setLink("Timer Shows", timerShowLink(tl, e.currentTarget.value as TimerShow))} aria-label="Timer shows" data-field="timer-show">
          {#each TIMER_SHOWS as o (o.v)}<option value={o.v}>{o.label}</option>{/each}
        </select>
      </div>
    {:else if link.type === "presentationName"}
      <div class="line">
        <span class="lab">Layer</span>
        <select value={link.source ?? "presentation"} onchange={(e) => setLink("Presentation Name", { type: "presentationName", source: e.currentTarget.value as CountdownSource })} aria-label="Presentation of layer" data-field="presentation-source">
          <option value="presentation">Slide</option>
          <option value="announcements">Announcements</option>
        </select>
      </div>
    {:else}
      <p class="note">Shows the live song's artwork (set on the Presentation tab), fitted to the box; nothing when the song has none.</p>
    {/if}
  </section>
{/if}

<section class="sec" data-visibility>
  <div class="line between">
    <h4>Visibility</h4>
    <select
      value=""
      onchange={(e) => {
        const v = e.currentTarget.value;
        e.currentTarget.value = "";
        if (v) add(v);
      }}
      aria-label="Add condition"
      data-field="add-condition"
    >
      <option value="">Add condition…</option>
      {#each CONDITION_KINDS as k (k.kind)}
        <option value={k.kind} disabled={(k.kind === "elementHasText" && !others.length) || (k.kind === "videoInput" && !inputs.length)}>{k.label}</option>
      {/each}
    </select>
  </div>
  {#if !list.length}
    <p class="note">Always shown. With conditions, the screens show this object only while all of them hold (the canvas shows it anyway).</p>
  {:else}
    <ul class="conds">
      {#each list as c, i (i)}
        <li class="cond" data-condition={c.type}>
          <div class="row">
            <span class="what">{conditionLabel(c, names)}</span>
            <button class="rm" onclick={() => save("Remove Condition", removeCondition(list, i))} title="Remove this condition" aria-label="Remove condition: {conditionLabel(c, names)}"><X size={14} /></button>
          </div>
          {#if c.type === "elementHasText"}
            <div class="line">
              <select value={c.element} onchange={(e) => change(i, { ...c, element: e.currentTarget.value })} aria-label="Object">
                {#if !others.some((o) => o.id === c.element)}<option value={c.element}>Missing object</option>{/if}
                {#each others as o (o.id)}<option value={o.id}>{elementName(o)}</option>{/each}
              </select>
              <select value={c.hasText === false ? "no" : "yes"} onchange={(e) => { const { hasText: _, ...rest } = c; change(i, e.currentTarget.value === "no" ? { ...rest, hasText: false } : rest); }} aria-label="Has text">
                <option value="yes">has text</option>
                <option value="no">is empty</option>
              </select>
            </div>
          {:else if c.type === "timer"}
            <div class="line">
              <select value={c.timer?.id ?? ""} onchange={(e) => { const { timer: _, ...rest } = c; const t = timerRef(e.currentTarget.value); change(i, t ? { ...rest, timer: t } : rest); }} aria-label="Timer">
                <option value="">Most recently started</option>
                {#if c.timer && !timersState.timerDocs.some((t) => t.id === c.timer?.id)}<option value={c.timer.id}>{c.timer.name ?? "Missing timer"}</option>{/if}
                {#each timersState.timerDocs as t (t.id)}<option value={t.id}>{t.name || "Timer"}</option>{/each}
              </select>
              <select value={c.state ?? "hasTimeRemaining"} onchange={(e) => change(i, { ...c, state: e.currentTarget.value as TimerCondition })} aria-label="Timer state">
                {#each TIMER_STATES as s (s.v)}<option value={s.v}>{s.label}</option>{/each}
              </select>
            </div>
          {:else if c.type === "videoInput"}
            <div class="line">
              <select
                value={c.input.id}
                onchange={(e) => {
                  const v = inputs.find((x) => x.id === e.currentTarget.value);
                  if (v) change(i, { ...c, input: inputLink(v) });
                }}
                aria-label="Video input"
                data-field="condition-input"
              >
                {#if !inputs.some((v) => v.id === c.input.id)}<option value={c.input.id}>{c.input.name ?? "Missing input"}</option>{/if}
                {#each inputs as v (v.id)}<option value={v.id}>{v.name || "Video input"}</option>{/each}
              </select>
              <select value={activeOf(c)} onchange={(e) => change(i, withActive(c, e.currentTarget.value))} aria-label="Video input live">
                <option value="yes">live</option>
                <option value="no">not live</option>
              </select>
            </div>
          {:else if c.type === "message"}
            <div class="line">
              <select value={c.message?.id ?? ""} onchange={(e) => { const { message: _, ...rest } = c; const m = messageRef(e.currentTarget.value); change(i, m ? { ...rest, message: m } : rest); }} aria-label="Message">
                <option value="">Any message</option>
                {#if c.message && !showDocs.messageDocs.some((m) => m.id === c.message?.id)}<option value={c.message.id}>{c.message.name ?? "Missing message"}</option>{/if}
                {#each showDocs.messageDocs as m (m.id)}<option value={m.id}>{m.name || "Message"}</option>{/each}
              </select>
              <select value={activeOf(c)} onchange={(e) => change(i, withActive(c, e.currentTarget.value))} aria-label="Message showing">
                <option value="yes">showing</option>
                <option value="no">not showing</option>
              </select>
            </div>
          {:else if c.type === "stageMessage"}
            <div class="line">
              <select value={activeOf(c)} onchange={(e) => change(i, withActive(c, e.currentTarget.value))} aria-label="Stage message showing">
                <option value="yes">showing</option>
                <option value="no">not showing</option>
              </select>
            </div>
          {:else if c.type === "layer"}
            <div class="line">
              <select value={c.layer} onchange={(e) => change(i, { ...c, layer: e.currentTarget.value as typeof c.layer })} aria-label="Layer">
                {#each CONDITION_LAYERS as l (l.v)}<option value={l.v}>{l.label}</option>{/each}
              </select>
              <select value={activeOf(c)} onchange={(e) => change(i, withActive(c, e.currentTarget.value))} aria-label="Layer live">
                <option value="yes">live</option>
                <option value="no">clear</option>
              </select>
            </div>
          {:else if c.type === "playlistItem"}
            <div class="line">
              <select value={c.which ?? "current"} onchange={(e) => change(i, { ...c, which: e.currentTarget.value as PlaylistItemWhich })} aria-label="Playlist item">
                {#each PLAYLIST_ITEMS as p (p.v)}<option value={p.v}>{p.label}</option>{/each}
              </select>
              <select value={activeOf(c)} onchange={(e) => change(i, withActive(c, e.currentTarget.value))} aria-label="Playlist item exists">
                <option value="yes">exists</option>
                <option value="no">missing</option>
              </select>
            </div>
          {:else if c.type === "videoCountdown" || c.type === "audioCountdown"}
            <div class="line">
              <select value={c.state ?? "hasTimeRemaining"} onchange={(e) => change(i, { ...c, state: e.currentTarget.value as MediaCondition })} aria-label="{c.type === 'videoCountdown' ? 'Video' : 'Audio'} state">
                {#each MEDIA_STATES as s (s.v)}<option value={s.v}>{s.label}</option>{/each}
              </select>
            </div>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .conds {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .cond {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border: 1px solid var(--line);
    border-radius: 6px;
    background: var(--surface-sunk);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .what {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .rm {
    flex: none;
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--text-meta);
    cursor: pointer;
  }
  .rm:hover {
    background: var(--surface-raised);
    color: var(--text);
  }
  .cond select {
    min-width: 0;
    flex: 1 1 0;
  }
</style>
