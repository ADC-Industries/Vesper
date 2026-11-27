<script lang="ts">
  // A song in Edit in the Library (plan 4.2 "Song, edit" and 4.3; question 5): today's slide
  // editor components arranged as the board draws them. The canvas (with its insert bar and
  // rulers) on the left; on the right the objects on this slide and the tabs Object / Slide /
  // Song. The slides themselves are in the middle column (ItemsPanel shows EditSlides). Edits
  // save as they happen (edit ops); Undo / Redo are in the item's header. The slide editor's
  // components, keys and inspector look are the slide editor's own (one editor, two places).
  import { untrack } from "svelte";
  import type { Connection } from "../../server";
  import type { EditState } from "../../edit/editState.svelte";
  import { allSlides } from "../../edit/ops";
  import { session } from "../../state/session.svelte";
  import { Tabs } from "../../ui";
  import { handleEditorKey, type CanvasHandle } from "../editor/keys";
  import InspPanel from "../editor/InspPanel.svelte";
  import InspArrange from "../editor/InspArrange.svelte";
  import StyleButtons from "../editor/StyleButtons.svelte";
  import EditCanvas from "../editor/EditCanvas.svelte";
  import EditObjects from "../editor/EditObjects.svelte";
  import InspShape from "../editor/InspShape.svelte";
  import InspText from "../editor/InspText.svelte";
  import InspSlide from "../editor/InspSlide.svelte";
  import InspTheme from "../editor/InspTheme.svelte";
  import InspPresentation from "../editor/InspPresentation.svelte";
  import InspVisibility from "../editor/InspVisibility.svelte";
  import ConflictDialog from "../../components/ConflictDialog.svelte";

  let { conn, st }: { conn: Connection; st: EditState } = $props();

  let canvasRef = $state<CanvasHandle | null>(null);
  let tab = $state<"object" | "slide" | "song">("slide");
  const tabs = [
    { value: "object" as const, label: "Object" },
    { value: "slide" as const, label: "Slide" },
    { value: "song" as const, label: "Song" },
  ];

  // Selecting an object shows its tab; letting go of every object goes back to the slide's.
  let hadSelection = false;
  $effect(() => {
    const n = st.selection.length;
    untrack(() => {
      if (n && !hadSelection) tab = "object";
      else if (!n && hadSelection && tab === "object") tab = "slide";
    });
    hadSelection = n > 0;
  });

  $effect(() => {
    conn.rest
      .fonts()
      .then((f) => (st.fonts = f))
      .catch(() => {});
  });

  let pres = $derived(session.open?.presentation ?? null);
  // The first slide shown, and the editor kept in step with the document.
  $effect(() => {
    const p = pres;
    untrack(() => {
      if (p && (!st.slideId || !allSlides(p).some((s) => s.id === st.slideId))) st.slideId = allSlides(p)[0]?.id ?? null;
      st.reconcile(p);
    });
  });

  let slide = $derived(st.slide);
  let textEl = $derived(st.selected.find((e) => e.kind?.type === "text") ?? null);

  // The slide editor's editing keys (Delete, arrows, Enter, Esc, Ctrl+A/C/X/V/D, zoom).
  function onKeydown(e: KeyboardEvent) {
    void handleEditorKey(e, st, canvasRef);
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="songedit" data-edit-view>
  <div class="canvas"><EditCanvas {conn} {st} bind:this={canvasRef} /></div>
  <aside class="panel" aria-label="Slide editor panel">
    <div class="objects">
      <div class="olist"><EditObjects {st} /></div>
    </div>
    <div class="tabs"><Tabs {tabs} value={tab} label="Editor panel" idPrefix="libedit" fill onchange={(v) => (tab = v)} /></div>
    <div class="body" role="tabpanel" id="libedit-panel-{tab}" aria-labelledby="libedit-tab-{tab}">
      <InspPanel>
        {#if !pres}
          <p class="empty">Opening the song…</p>
        {:else if tab === "object"}
          {#if st.selection.length > 1}
            <InspArrange {st} />
            <InspShape {st} {conn} />
          {:else if st.selection.length}
            {#if slide}<InspTheme {st} {slide} el={st.selected[0] ?? null} />{/if}
            <InspShape {st} {conn} />
            {#if textEl}<StyleButtons {st} />{#key textEl.id}<InspText {st} el={textEl} />{/key}{/if}
            {#if slide && st.selected[0]}{#key st.selected[0].id}<InspVisibility {st} {slide} el={st.selected[0]} />{/key}{/if}
          {:else}
            <p class="empty">Click an object on the slide (or in the list above) to style it.</p>
          {/if}
        {:else if tab === "slide"}
          {#if slide}
            <InspTheme {st} {slide} />
            <InspSlide {st} {slide} />
          {:else}<p class="empty">No slide selected.</p>{/if}
        {:else}
          <InspPresentation {st} {pres} {conn} />
        {/if}
      </InspPanel>
    </div>
  </aside>
  {#if st.conflict}
    {@const c = st.conflict}
    <ConflictDialog
      title="This text was changed by someone else"
      message="The text box changed on the server while you were editing it. Choose which text to keep."
      theirs={c.theirs}
      mine={c.mine}
      onTheirs={() => {
        st.conflict = null;
        void session.reopen();
      }}
      onMine={() => void c.keepMine()}
    />
  {/if}
</div>

<style>
  .songedit {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  .canvas {
    flex: 1;
    min-width: 0;
    min-height: 0;
    position: relative;
    overflow: hidden;
  }
  .panel {
    flex: 0 0 clamp(300px, 19vw, 372px);
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-left: 1px solid var(--surface-raised);
    background: var(--surface-sunk);
    font-size: 13px;
  }
  .objects {
    flex: none;
    display: flex;
    flex-direction: column;
    border-bottom: 1px solid var(--surface-raised);
  }
  .olist {
    height: 170px;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .tabs {
    flex: none;
    padding: 8px 14px 0;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
</style>
