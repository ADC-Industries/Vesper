<script lang="ts">
  import Button from "../../ui/Button.svelte";
  // Inspector, Slide tab: the slide's actions (run in order when the slide is triggered).
  // Add, remove, reorder, enable / disable, and small editors per kind. Every change is one
  // edit op (one undo step). Media and imported kinds this app does not edit are read-only.
  import Plus from "lucide-svelte/icons/plus";
  import Trash from "lucide-svelte/icons/trash-2";
  import ChevronUp from "lucide-svelte/icons/chevron-up";
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import type { ActionKind, ClearLayer, Slide, SlideAction, TimerOperation } from "$protocol";
  import type { EditState } from "../../edit/editState.svelte";
  import { session } from "../../state/session.svelte";
  import { showDocsState } from "../../state/showDocs.svelte";
  import { timersState } from "../../state/timers.svelte";
  import { shell } from "../../state/shell.svelte";
  import { uuid } from "../../edit/ids";
  import type { MenuItem } from "../../shell/menu";
  import { formatDuration, parseDuration } from "../timers/timers";
  import {
    ADDABLE,
    CLEAR_TARGETS,
    KIND_LABELS,
    actionSummary,
    addActionOp,
    isEditable,
    moveActionOp,
    newAction,
    removeActionOp,
    setActionOp,
    type ActionNames,
  } from "../../edit/actions";

  let { st, slide }: { st: EditState; slide: Slide } = $props();

  let actions = $derived(slide.actions ?? []);
  let open = $state<string | null>(null);

  let names = $derived<ActionNames>({
    timers: new Map(timersState.timerDocs.map((t) => [t.id, t.name || "Timer"])),
    messages: new Map(showDocsState.messageDocs.map((m) => [m.id, m.name || "Message"])),
    props: new Map(showDocsState.propDocs.map((p) => [p.id, p.name || "Prop"])),
    looks: new Map(session.looks.map((l) => [l.id, l.name || "Look"])),
    clearGroups: new Map(session.clearGroups.map((g) => [g.id, g.name || "Clear Group"])),
  });

  const first = (l: { id: string; name?: string }[]) => (l[0] ? { id: l[0].id, ...(l[0].name ? { name: l[0].name } : {}) } : undefined);

  function addMenu(e: MouseEvent) {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const firsts = {
      timer: first(timersState.timerDocs),
      message: first(showDocsState.messageDocs),
      prop: first(showDocsState.propDocs),
      look: first(session.looks),
      clearGroup: first(session.clearGroups),
    };
    const items: MenuItem[] = ADDABLE.map((k) => {
      const a = newAction(k, uuid(), firsts);
      return {
        type: "item" as const,
        id: k,
        label: KIND_LABELS[k],
        disabled: !a,
        title: a ? undefined : k === "macro" || k === "stageLayout" ? "Not editable in this version" : `There is no ${KIND_LABELS[k].toLowerCase()} yet`,
        run: () => {
          if (!a) return;
          void st.run(`Add ${KIND_LABELS[k]} Action`, [addActionOp(slide.id, actions, a)]).then((ok) => ok && (open = a.id));
        },
      };
    });
    shell.contextMenu = { x: r.left, y: r.bottom + 2, items };
  }

  function set(a: SlideAction, label = "Change Action") {
    void st.run(label, [setActionOp(slide.id, a)]);
  }
  function setKind(a: SlideAction, kind: ActionKind) {
    set({ ...a, kind });
  }
  function move(i: number, d: -1 | 1) {
    const op = moveActionOp(slide.id, actions, i, d);
    if (op) void st.run("Move Action", [op]);
  }
  function remove(a: SlideAction) {
    void st.run("Remove Action", [removeActionOp(slide.id, a.id)]);
  }
  function link<T extends { id: string; name?: string }>(list: T[], id: string) {
    const d = list.find((x) => x.id === id);
    return { id, ...(d?.name ? { name: d.name } : {}) };
  }

  function opKind(op: TimerOperation | undefined): string {
    return op?.type ?? "start";
  }
  function setOp(a: SlideAction & { kind: { type: "timer" } }, type: string, seconds?: number) {
    const s = seconds ?? (a.kind.operation && "seconds" in a.kind.operation ? a.kind.operation.seconds : 60);
    const op: TimerOperation = type === "set" || type === "adjust" ? { type, seconds: s } : ({ type } as TimerOperation);
    setKind(a, { ...a.kind, operation: op });
  }
  function toggleTarget(a: SlideAction & { kind: { type: "clearLayer" } }, layer: ClearLayer, on: boolean) {
    const cur = a.kind.targets ?? [];
    const has = cur.some((t) => (t.layer ?? "all") === layer);
    if (on === has) return;
    const targets = on ? [...cur, layer === "all" ? {} : { layer }] : cur.filter((t) => (t.layer ?? "all") !== layer);
    setKind(a, { ...a.kind, targets });
  }
  function tokensOf(id: string) {
    return showDocsState.messageDocs.find((m) => m.id === id)?.tokens?.filter((t) => !t.kind || t.kind.type === "text") ?? [];
  }
  function setToken(a: SlideAction & { kind: { type: "message" } }, name: string, value: string) {
    const values = (a.kind.values ?? []).filter((v) => v.name !== name);
    if (value) values.push({ name, value });
    setKind(a, { ...a.kind, values });
  }
</script>

<section class="sec acts" data-testid="slide-actions">
  <div class="line between">
    <h4>Actions</h4>
    <Button size="sm" onclick={addMenu} data-testid="add-action"><Plus size={13} /> Add</Button>
  </div>
  {#if !actions.length}<p class="note">No actions. Actions run in order when the slide is triggered.</p>{/if}
  <ul>
    {#each actions as a, i (a.id)}
      {@const editable = isEditable(a)}
      <li class:off={a.enabled === false} class:open={open === a.id} data-testid="action-row">
        <div class="row">
          <input type="checkbox" checked={a.enabled !== false} onchange={(e) => set({ ...a, enabled: e.currentTarget.checked }, e.currentTarget.checked ? "Enable Action" : "Disable Action")} aria-label="Enabled" title="Enabled" />
          <button class="sum" onclick={() => (open = open === a.id ? null : a.id)} disabled={!editable} title={editable ? "Edit" : "Read-only"}>
            <span class="kind">{KIND_LABELS[a.kind.type]}</span>
            <span class="what">{actionSummary(a, names)}</span>
          </button>
          <button class="tg" onclick={() => move(i, -1)} disabled={i === 0} aria-label="Move up"><ChevronUp size={13} /></button>
          <button class="tg" onclick={() => move(i, 1)} disabled={i === actions.length - 1} aria-label="Move down"><ChevronDown size={13} /></button>
          <button class="tg" onclick={() => remove(a)} aria-label="Remove action" title="Remove"><Trash size={13} /></button>
        </div>
        {#if open === a.id && editable}
          <div class="ed">
            {#if a.kind.type === "timer"}
              {@const k = a.kind}
              <select value={k.timer.id} onchange={(e) => setKind(a, { ...k, timer: link(timersState.timerDocs, e.currentTarget.value) })} aria-label="Timer">
                {#if !names.timers?.has(k.timer.id)}<option value={k.timer.id}>{k.timer.name ?? "(missing)"}</option>{/if}
                {#each timersState.timerDocs as t (t.id)}<option value={t.id}>{t.name || "Timer"}</option>{/each}
              </select>
              <select value={opKind(k.operation)} onchange={(e) => setOp({ ...a, kind: k }, e.currentTarget.value)} aria-label="Operation">
                <option value="start">Start</option>
                <option value="stop">Stop</option>
                <option value="reset">Reset</option>
                <option value="restart">Reset and start</option>
                <option value="set">Set to…</option>
                <option value="adjust">Add / subtract…</option>
              </select>
              {#if k.operation?.type === "set" || k.operation?.type === "adjust"}
                {@const sec = k.operation.seconds}
                <input type="text" value={(sec < 0 ? "-" : "") + formatDuration(Math.abs(sec))} onchange={(e) => {
                  const v = e.currentTarget.value.trim();
                  const neg = v.startsWith("-");
                  const p = parseDuration(neg ? v.slice(1) : v);
                  if (p !== null) setOp({ ...a, kind: k }, k.operation!.type, neg ? -p : p);
                }} onkeydown={(e) => e.stopPropagation()} aria-label="Seconds" title="m:ss (a bare number is minutes)" />
              {/if}
            {:else if a.kind.type === "message"}
              {@const k = a.kind}
              <select value={k.message.id} onchange={(e) => setKind(a, { ...k, message: link(showDocsState.messageDocs, e.currentTarget.value), values: [] })} aria-label="Message">
                {#if !names.messages?.has(k.message.id)}<option value={k.message.id}>{k.message.name ?? "(missing)"}</option>{/if}
                {#each showDocsState.messageDocs as m (m.id)}<option value={m.id}>{m.name || "Message"}</option>{/each}
              </select>
              <select value={k.show === false ? "hide" : "show"} onchange={(e) => setKind(a, { ...k, show: e.currentTarget.value === "show" })} aria-label="Show or hide">
                <option value="show">Show</option>
                <option value="hide">Hide</option>
              </select>
              {#if k.show !== false}
                {#each tokensOf(k.message.id) as t (t.name)}
                  <label class="tok"><span>{t.name}</span><input type="text" value={k.values?.find((v) => v.name === t.name)?.value ?? ""} placeholder={t.kind?.type === "text" ? (t.kind.defaultValue ?? "") : ""} onchange={(e) => setToken({ ...a, kind: k }, t.name, e.currentTarget.value)} onkeydown={(e) => e.stopPropagation()} /></label>
                {/each}
              {/if}
            {:else if a.kind.type === "prop"}
              {@const k = a.kind}
              <select value={k.prop.id} onchange={(e) => setKind(a, { ...k, prop: link(showDocsState.propDocs, e.currentTarget.value) })} aria-label="Prop">
                {#if !names.props?.has(k.prop.id)}<option value={k.prop.id}>{k.prop.name ?? "(missing)"}</option>{/if}
                {#each showDocsState.propDocs as p (p.id)}<option value={p.id}>{p.name || "Prop"}</option>{/each}
              </select>
              <select value={k.show === false ? "hide" : "show"} onchange={(e) => setKind(a, { ...k, show: e.currentTarget.value === "show" })} aria-label="Show or hide">
                <option value="show">Show</option>
                <option value="hide">Hide</option>
              </select>
            {:else if a.kind.type === "look"}
              {@const k = a.kind}
              <select value={k.look.id} onchange={(e) => setKind(a, { ...k, look: link(session.looks, e.currentTarget.value) })} aria-label="Look">
                {#if !names.looks?.has(k.look.id)}<option value={k.look.id}>{k.look.name ?? "(missing)"}</option>{/if}
                {#each session.looks as l (l.id)}<option value={l.id}>{l.name || "Look"}</option>{/each}
              </select>
            {:else if a.kind.type === "clearGroup"}
              {@const k = a.kind}
              <select value={k.group.id} onchange={(e) => setKind(a, { ...k, group: link(session.clearGroups, e.currentTarget.value) })} aria-label="Clear group">
                {#if !names.clearGroups?.has(k.group.id)}<option value={k.group.id}>{k.group.name ?? "(missing)"}</option>{/if}
                {#each session.clearGroups as g (g.id)}<option value={g.id}>{g.name || "Clear Group"}</option>{/each}
              </select>
            {:else if a.kind.type === "clearLayer"}
              {@const k = a.kind}
              <div class="targets">
                {#each CLEAR_TARGETS as t (t.layer)}
                  <label class="check"><input type="checkbox" checked={(k.targets ?? []).some((x) => (x.layer ?? "all") === t.layer)} onchange={(e) => toggleTarget({ ...a, kind: k }, t.layer, e.currentTarget.checked)} />{t.label}</label>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
      </li>
    {/each}
  </ul>
</section>

<style>
  ul {
    list-style: none;
    margin: 6px 0 0;
    padding: 0;
  }
  li {
    border: 1px solid var(--line);
    border-radius: 2px;
    background: var(--surface-panel);
    margin-bottom: 4px;
  }
  li.open {
    border-color: var(--accent);
  }
  li.off .sum {
    opacity: 0.5;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 4px 3px 6px;
  }
  .sum {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: 1px 4px;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
  }
  .sum:disabled {
    cursor: default;
    color: var(--text-2);
  }
  .kind {
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--accent-ring);
  }
  .what {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tg {
    width: 22px !important;
    height: 22px !important;
  }
  .tg:disabled {
    opacity: 0.4;
  }
  .ed {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 6px 8px 8px;
    border-top: 1px solid var(--line);
  }
  .ed select {
    max-width: 100%;
  }
  .ed input[type="text"] {
    width: 80px;
  }
  .tok {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
  }
  .tok span {
    min-width: 60px;
    color: var(--text-2);
  }
  .tok input {
    flex: 1;
  }
  .targets {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px 10px;
  }
</style>
