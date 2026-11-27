<script lang="ts">
  // The Edit page's inspector. Its tabs follow the selection: nothing selected, Presentation /
  // Slide / Actions / Build; one object, Shape (with its live data and visibility conditions) /
  // Text (a text object) / Build; several, Arrange / Build. Build edits the slide's object
  // builds (plan G18). The slide's objects are docked underneath.
  import type { Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import type { EditState, InspectorTab } from "../../edit/editState.svelte";
  import Tabs from "../../ui/Tabs.svelte";
  import InspPanel from "./InspPanel.svelte";
  import InspPresentation from "./InspPresentation.svelte";
  import InspSlide from "./InspSlide.svelte";
  import InspLanguage from "./InspLanguage.svelte";
  import InspActions from "./InspActions.svelte";
  import InspShape from "./InspShape.svelte";
  import InspText from "./InspText.svelte";
  import InspArrange from "./InspArrange.svelte";
  import AlignBar from "./AlignBar.svelte";
  import StyleButtons from "./StyleButtons.svelte";
  import EditObjects from "./EditObjects.svelte";
  import InspTheme from "./InspTheme.svelte";
  import InspSlot from "./InspSlot.svelte";
  import InspBuild from "./InspBuild.svelte";
  import InspVisibility from "./InspVisibility.svelte";
  import InspChords from "./InspChords.svelte";

  let { conn, st }: { conn: Connection; st: EditState } = $props();

  let pres = $derived(st.presentation);
  let slide = $derived(st.slide);
  let n = $derived(st.selection.length);
  let textEl = $derived(st.selected.find((e) => e.kind?.type === "text") ?? null);
  let oneSlide = $derived(!!session.open?.prop || !!session.open?.stageLayout);
  let tabs = $derived<{ id: InspectorTab; label: string }[]>(
    n > 1
      ? [{ id: "arrange", label: "Arrange" }]
      : n === 1
        ? [{ id: "shape", label: "Shape" }, ...(textEl ? [{ id: "text" as const, label: "Text" }] : [])]
        : [
            ...(session.open?.prop ? [] : [{ id: "presentation" as const, label: "Presentation" }]),
            { id: "slide", label: "Slide" },
            ...(oneSlide ? [] : [{ id: "actions" as const, label: "Actions" }]),
          ],
  );
  let tab = $derived(st.tab === "build" || tabs.some((t) => t.id === st.tab) ? st.tab : (tabs[0]?.id ?? "slide"));
  // The build step preview ends with the Build tab.
  $effect(() => {
    if (tab !== "build" && st.buildPreview !== null) st.buildPreview = null;
  });
</script>

<aside class="insp" aria-label="Inspector">
  <div class="tabs">
    <Tabs
      tabs={[...tabs.map((t) => ({ value: t.id, label: t.label })), { value: "build" as InspectorTab, label: "Build" }]}
      value={tab}
      label="Inspector"
      idPrefix="insp"
      onchange={(v) => (st.tab = v)}
    />
  </div>
  <div class="body" role="tabpanel" id="insp-panel-{tab}" aria-labelledby="insp-tab-{tab}">
    <InspPanel>
      {#if !pres}
        <p class="empty">No presentation open.</p>
      {:else if tab === "presentation"}
        <InspPresentation {st} {pres} {conn} />
      {:else if tab === "slide"}
        {#if slide}<InspTheme {st} {slide} /><InspSlide {st} {slide} actions={false} />{#if !session.open?.prop && !session.open?.stageLayout && !session.open?.theme}<InspLanguage {conn} {st} {slide} />{/if}{:else}<p class="empty">No slide selected.</p>{/if}
      {:else if tab === "actions"}
        {#if slide}<InspActions {st} {slide} />{:else}<p class="empty">No slide selected.</p>{/if}
      {:else if tab === "build"}
        {#if slide && !oneSlide}<InspBuild {st} {slide} />{:else if slide}<p class="empty">Builds play on presentation slides.</p>{:else}<p class="empty">No slide selected.</p>{/if}
      {:else if tab === "arrange"}
        <InspArrange {st} />
        <InspShape {st} {conn} />
      {:else if tab === "shape"}
        {#if slide && n === 1}<InspTheme {st} {slide} el={st.selected[0] ?? null} />{/if}
        {#if slide && n === 1 && session.open?.theme && st.selected[0]}<InspSlot {st} {slide} el={st.selected[0]} />{/if}
        <section class="sec"><span class="label">Align on the slide</span><AlignBar {st} toSlide /></section>
        <InspShape {st} {conn} />
        {#if slide && n === 1 && st.selected[0]}{#key st.selected[0].id}<InspVisibility {st} {slide} el={st.selected[0]} />{/key}{/if}
      {:else if tab === "text" && textEl}
        {#if slide && session.open?.theme}<InspSlot {st} {slide} el={textEl} />{/if}
        <StyleButtons {st} />
        {#key textEl.id}<InspText {st} el={textEl} />{/key}
        {#if !session.open?.stageLayout}<InspChords {st} el={textEl} />{/if}
      {/if}
    </InspPanel>
  </div>
  <section class="objects" aria-label="Objects on this slide"><EditObjects {st} /></section>
</aside>

<style>
  .insp {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface-panel);
    font-size: 13px;
  }
  .tabs {
    flex: none;
    padding: 0 8px;
    border-bottom: 1px solid var(--line);
  }
  .body {
    flex: 1 1 0;
    min-height: 120px;
    overflow-y: auto;
  }
  .objects {
    flex: none;
    height: 34%;
    min-height: 140px;
    max-height: 300px;
    display: flex;
    flex-direction: column;
  }
  /* A short window (1366 × 768): the docked list gives the tab more room, so the Arrange tab's
     Order row (and every other tab's controls) is not half under it; the tab still scrolls. */
  @media (max-height: 820px) {
    .objects {
      height: 24%;
      min-height: 120px;
    }
  }
</style>
