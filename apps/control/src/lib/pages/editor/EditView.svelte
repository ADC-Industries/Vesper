<script lang="ts">
  // The Edit page's body (the canvas's slide editor): slides on the left; the formatting bar,
  // the canvas and the text field in the centre; the inspector (with the objects docked) on the
  // right. Owns the editor state (the page's toolbar gets it through `st`) and the editing keys.
  import { onDestroy, untrack } from "svelte";
  import type { Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import { useLive } from "../../live";
  import type { EditState } from "../../edit/editState.svelte";
  import { allSlides } from "../../edit/ops";
  import Splitter from "../../ui/Splitter.svelte";
  import EditSlides from "./EditSlides.svelte";
  import EditCanvas from "./EditCanvas.svelte";
  import EditInspector from "./EditInspector.svelte";
  import FormatBar from "./FormatBar.svelte";
  import TextField from "./TextField.svelte";
  import { handleEditorKey } from "./keys";
  import ConflictDialog from "../../components/ConflictDialog.svelte";
  import Toasts from "../../shell/Toasts.svelte";

  let { conn, st, locked = false }: { conn: Connection; st: EditState; locked?: boolean } = $props();
  const live = useLive();
  const liveSlide = live.slide;

  let canvasRef = $state<{ zoomStep: (d: 1 | -1) => void; editText: (id: string, selectAll?: boolean) => void; cancelGesture: () => boolean } | null>(null);

  // Column sizes (px), remembered per browser.
  function load(key: string, d: number): number {
    try {
      const v = Number(localStorage.getItem(`edit.${key}`));
      return Number.isFinite(v) && v > 0 ? v : d;
    } catch {
      return d;
    }
  }
  function save(key: string, v: number) {
    try {
      localStorage.setItem(`edit.${key}`, String(Math.round(v)));
    } catch {
      /* private mode */
    }
  }
  const W = typeof window === "undefined" ? 1920 : window.innerWidth;
  let left = $state(load("left", Math.round(Math.max(170, Math.min(230, W * 0.11)))));
  let right = $state(load("right", Math.round(Math.max(300, Math.min(360, W * 0.172)))));

  // Fonts for the font picker.
  $effect(() => {
    conn.rest
      .fonts()
      .then((f) => (st.fonts = f))
      .catch(() => {});
  });

  // The first slide shown: the live one when it belongs to this presentation. Another
  // presentation opened: typing in place is saved to the one it was typed in, and the editor
  // starts over (its slide wasn't deleted, so no "deleted" note). Before the DOM updates, so
  // the in-place editor still holds the typing.
  let pres = $derived(session.open?.presentation ?? null);
  let lastPres: string | null = null;
  $effect.pre(() => {
    const p = pres;
    untrack(() => {
      const id = p?.id ?? null;
      if (lastPres !== null && id !== lastPres) {
        void st.finishText?.();
        st.startOver();
      }
      lastPres = id;
      if (!st.slideId && p) {
        const l = $liveSlide;
        const slides = allSlides(p);
        st.slideId = (l?.presentation === p.id && slides.some((s) => s.id === l.slide) ? l.slide : slides[0]?.id) ?? null;
      }
      st.reconcile(p);
    });
  });

  onDestroy(() => void st.finishText?.());

  // ---- shortcuts: the editing keys (shared with the Library's song Edit) ----------------------------

  function onKeydown(e: KeyboardEvent) {
    // Someone else is editing (G8): look only.
    if (locked) return;
    void handleEditorKey(e, st, canvasRef);
  }

  /** Opens a text object for typing in place (a new text box: its text selected). */
  export function editText(id: string, selectAll = false) {
    canvasRef?.editText(id, selectAll);
  }
</script>

<svelte:window onkeydown={onKeydown} />

<div class="edit" style:grid-template-columns={`${left}px 1px minmax(320px, 1fr) 1px ${right}px`} data-edit-view>
  <div class="left"><EditSlides {conn} {st} /></div>
  <div class="vs"><Splitter dir="col" label="Resize the slide list" value={left} onchange={(v) => ((left = Math.max(140, Math.min(420, v))), save("left", left))} /></div>
  <main class="centre" inert={locked} class:locked>
    <FormatBar {st} />
    <div class="canvas"><EditCanvas {conn} {st} insertBar={false} bind:this={canvasRef} /><Toasts {conn} /></div>
    <TextField {st} />
  </main>
  <div class="vs"><Splitter dir="col" label="Resize the inspector" value={right} scale={-1} onchange={(v) => ((right = Math.max(280, Math.min(560, v))), save("right", right))} /></div>
  <div class="right" inert={locked} class:locked><EditInspector {conn} {st} /></div>
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
  .edit {
    position: relative;
    height: 100%;
    display: grid;
    min-height: 0;
    background: var(--line);
  }
  .left {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }
  .centre {
    display: flex;
    flex-direction: column;
    background: var(--surface-panel);
  }
  .canvas {
    position: relative;
    flex: 1 1 0;
    min-height: 200px;
  }
  .locked {
    opacity: 0.7;
  }
  .centre,
  .right {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .vs {
    display: flex;
    min-width: 0;
    min-height: 0;
  }
  .vs :global(.splitter) {
    height: 100%;
  }
</style>
