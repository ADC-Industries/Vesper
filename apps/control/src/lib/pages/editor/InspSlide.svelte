<script lang="ts">
  import Button from "../../ui/Button.svelte";
  // Inspector, Slide tab: label and colour, group, enabled, background, transition override
  // (the transitions grid), hotkey, notes; slide actions too where the editor has no Actions tab.
  import type { Slide } from "$protocol";
  import type { EditState } from "../../edit/editState.svelte";
  import { hotkeyFrom, hotkeyLabel, notesFromText, notesText } from "../../edit/ops";
  import { DEFAULT_GROUPS, groupChoices, groupOps, type GroupChoice } from "../../edit/reflow";
  import ColorIn from "./ColorIn.svelte";
  import TransitionGrid from "./TransitionGrid.svelte";
  import InspActions from "./InspActions.svelte";
  import { HEADER_COLORS, session } from "../../state/session.svelte";

  let { st, slide, actions = true }: { st: EditState; slide: Slide; actions?: boolean } = $props();

  // Group select: the presentation's groups, then the global ones (or the usual song parts).
  let pres = $derived(st.presentation);
  let section = $derived((pres?.sections ?? []).find((s) => (s.slides ?? []).some((x) => x.id === slide.id)) ?? null);
  let globals = $derived<GroupChoice[]>(
    session.globalGroups.length
      ? session.globalGroups.map((g) => ({ name: g.name ?? "", color: g.color ?? null, globalGroup: { id: g.id, name: g.name ?? "" } }))
      : DEFAULT_GROUPS,
  );
  let groups = $derived(groupChoices(pres, globals));
  function setGroup(name: string) {
    const p = pres;
    const g = [...groups.sections, ...groups.global].find((x) => x.name === name);
    if (p && g && name !== (section?.name ?? "")) void st.run("Group", groupOps(p, slide.id, g));
  }
  let capturing = $state(false);

  function set(label: string, fields: Omit<Extract<import("$protocol").PresentationOp, { type: "setSlide" }>, "type" | "slide">) {
    void st.run(label, [{ type: "setSlide", slide: slide.id, ...fields }]);
  }

  function setLabel(text: string) {
    const t = text.trim();
    if (t === (slide.label?.text ?? "")) return;
    set("Slide Label", { label: t ? { ...(slide.label ?? {}), text: t } : null });
  }

  function onHotkey(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") {
      capturing = false;
      return;
    }
    if (e.key === "Backspace" || e.key === "Delete") {
      capturing = false;
      set("Hotkey", { hotkey: null });
      return;
    }
    const k = hotkeyFrom(e);
    if (!k) return;
    capturing = false;
    set("Hotkey", { hotkey: k });
  }
</script>

<section class="sec">
  <h4>Label</h4>
  <div class="line">
    <input class="grow" type="text" value={slide.label?.text ?? ""} placeholder={session.open?.theme ? "Template name" : "No label"} onchange={(e) => setLabel(e.currentTarget.value)} onkeydown={(e) => e.stopPropagation()} aria-label="Slide label" data-field="slide-label" />
    <ColorIn color={slide.label?.color} label="Label colour" onchange={(c) => set("Label Colour", { label: { ...(slide.label ?? {}), color: c } })} />
  </div>
  <div class="swatches">
    {#each HEADER_COLORS as c, i (i)}
      <button class="sw" style:background={`rgb(${c.r * 255},${c.g * 255},${c.b * 255})`} onclick={() => set("Label Colour", { label: { ...(slide.label ?? {}), color: c } })} aria-label={`Label colour ${i + 1}`}></button>
    {/each}
  </div>
  <label class="check en"><input type="checkbox" checked={slide.enabled !== false} onchange={(e) => set(e.currentTarget.checked ? "Enable Slide" : "Disable Slide", { enabled: e.currentTarget.checked })} data-field="slide-enabled" />Enabled</label>
</section>

{#if !session.open?.prop && !session.open?.stageLayout}
  <section class="sec">
    <h4>Group</h4>
    <select class="wide" value={section?.name ?? ""} onchange={(e) => setGroup(e.currentTarget.value)} aria-label="Slide group" data-field="slide-group">
      {#if !section?.name}<option value="" disabled>No group</option>{/if}
      {#if groups.sections.length}
        <optgroup label="In this presentation">
          {#each groups.sections as g (g.name)}<option value={g.name}>{g.name}</option>{/each}
        </optgroup>
      {/if}
      {#if groups.global.length}
        <optgroup label="Groups">
          {#each groups.global as g (g.name)}<option value={g.name}>{g.name}</option>{/each}
        </optgroup>
      {/if}
    </select>
    <p class="note">The slide starts this group; the slides after it in its group follow.</p>
  </section>
{/if}

<section class="sec">
  <label class="check">
    <input type="checkbox" checked={!!slide.background} onchange={(e) => set("Slide Background", { background: e.currentTarget.checked ? { r: 0, g: 0, b: 0 } : null })} data-field="slide-background" />
    Background
  </label>
  {#if slide.background}
    <div class="line"><ColorIn color={slide.background} label="Slide background colour" onchange={(c) => set("Slide Background", { background: c })} /></div>
  {/if}
</section>

<section class="sec">
  <h4>Transition</h4>
  <TransitionGrid transition={slide.transition} owner={slide.id} defaultLabel="Default" label="Slide transition" onchange={(t) => st.run("Slide Transition", [{ type: "setSlide", slide: slide.id, transition: t }])} />
</section>

<section class="sec">
  <h4>Hotkey</h4>
  <div class="line">
    <button class="hk" class:cap={capturing} onclick={() => (capturing = true)} onkeydown={(e) => capturing && onHotkey(e)} onblur={() => (capturing = false)} data-field="slide-hotkey">
      {capturing ? "Press a key…" : hotkeyLabel(slide.hotkey) || "None"}
    </button>
    {#if slide.hotkey}<Button size="sm" onclick={() => set("Hotkey", { hotkey: null })}>Clear</Button>{/if}
  </div>
</section>

<section class="sec">
  <h4>Notes</h4>
  <textarea class="notes" rows="4" value={notesText(slide.notes)} onchange={(e) => set("Slide Notes", { notes: notesFromText(e.currentTarget.value) })} onkeydown={(e) => e.stopPropagation()} aria-label="Slide notes" data-field="slide-notes-tab"></textarea>
</section>

{#if actions && !session.open?.prop}
  <InspActions {st} {slide} />
{/if}
