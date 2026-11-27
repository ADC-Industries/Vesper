<script lang="ts">
  // The Edit view's canvas: rulers, the server's render of the slide as the base layer, and an
  // interactive overlay (selection, handles, snapping guides, marquee, in-place text editing).
  // Drags and resizes preview on the client (outline plus a snapshot cropped from the last
  // render) and commit one `setElement` per element on pointer-up; the server's new render then
  // replaces the preview. The server render is the truth for layout.
  import Type from "lucide-svelte/icons/type";
  import Shapes from "lucide-svelte/icons/shapes";
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import PenTool from "lucide-svelte/icons/pen-tool";
  import ImageIcon from "lucide-svelte/icons/image";
  import Video from "lucide-svelte/icons/video";
  import Globe from "lucide-svelte/icons/globe";
  import MoveHorizontal from "lucide-svelte/icons/move-horizontal";
  import Grid3x3 from "lucide-svelte/icons/grid-3x3";
  import Magnet from "lucide-svelte/icons/magnet";
  import ZoomIn from "lucide-svelte/icons/zoom-in";
  import LinkIcon from "lucide-svelte/icons/link";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import type { Element, MediaBinItemInfo } from "$protocol";
  import { onDestroy, tick } from "svelte";
  import type { Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import { timersState } from "../../state/timers.svelte";
  import { useLive } from "../../live";
  import { slideHash } from "../../edit/reflow";
  import type { EditState } from "../../edit/editState.svelte";
  import {
    HANDLES,
    constrainAxis,
    containsPoint,
    fitView,
    fitWidthView,
    handleUnit,
    intersects,
    rectFromPoints,
    resizeRect,
    rotatedBounds,
    snapMove,
    snapTargets,
    stepZoom,
    toModel,
    union,
    zoomAt,
    type Guide,
    type Handle,
    type Rect,
    type SnapTargets,
  } from "../../edit/geometry";
  import { SHAPE_CHOICES, boundsOf, fillDrawn, type ShapeChoice } from "../../edit/elements";
  import { insertDataLink, insertImage, insertShape, insertText } from "./inserts";
  import { DATA_LINKS, timerLink, type DataLinkChoice } from "../stage/stage";
  import { boundsOps, saveTextOps } from "../../edit/ops";
  import { editedFromTextBox, plainText, rangeFromUtf16, rebuildTextBox, cpToUtf16, sameTextBox } from "../../edit/text";
  import { offsetOf, readEditor, renderTextBox, selectOffsets } from "../../edit/textDom";
  import { hiddenAt } from "./builds";
  import { cssColor } from "../../model";
  import Ruler from "./Ruler.svelte";
  import MediaPicker from "./MediaPicker.svelte";

  // `insertBar`: the floating insert bar (the Library's song Edit; the Edit page has its toolbar).
  let { conn, st, insertBar = true }: { conn: Connection; st: EditState; insertBar?: boolean } = $props();

  const live = useLive();
  const liveSlide = live.slide;

  let viewport = $state<HTMLElement | null>(null);
  let vw = $state(0);
  let vh = $state(0);

  let pres = $derived(st.presentation);
  let slide = $derived(st.slide);
  let canvas = $derived(st.canvasSize());
  let view = $derived(st.view);
  let isLive = $derived(!!slide && !!pres && $liveSlide?.slide === slide.id && $liveSlide?.presentation === pres.id);

  /** Elements with the drag preview applied. */
  let shown = $derived<{ el: Element; rect: Rect }[]>(
    (slide?.elements ?? []).map((el) => ({ el, rect: st.preview?.get(el.id) ?? boundsOf(el) })),
  );

  // ---- fitting ----------------------------------------------------------------------------------

  $effect(() => {
    if (!vw || !vh) return;
    const c = canvas;
    if (st.viewMode === "fit") st.view = fitView(c, { width: vw, height: vh }, 30);
    else if (st.viewMode === "width") st.view = fitWidthView(c, { width: vw, height: vh }, 20);
  });

  // ---- the server's render ----------------------------------------------------------------------

  const BUCKETS = [480, 960, 1440, 1920, 2880, 3840];
  let dpr = typeof devicePixelRatio === "number" ? devicePixelRatio : 1;
  let renderW = $derived.by(() => {
    const need = canvas.width * view.zoom * dpr;
    const cap = Math.min(3840, Math.max(480, canvas.width * 2));
    return Math.min(cap, BUCKETS.find((b) => b >= need) ?? 3840);
  });

  /** Elements left out of the render: dragged (their snapshot moves instead) and edited text. */
  let dragHide = $state<string[]>([]);
  /** A text save is in flight. */
  let saving = $state(false);
  let closingText = $state<string | null>(null);
  // A text box being saved stays hidden until the save is done; then the render with the new
  // text loads while the editor's copy stays on top (see below).
  let hide = $derived([
    ...dragHide,
    ...(st.editingText ? [st.editingText] : []),
    ...(closingText && saving && closingText !== st.editingText ? [closingText] : []),
    // The Build tab's step preview: what the builds hide at that step (plan G18).
    ...(st.buildPreview !== null && slide ? hiddenAt(slide, st.buildPreview) : []),
  ]);
  let wanted = $derived(
    pres && slide ? conn.rest.slideRenderUrl(pres.id, slide.id, renderW, hide, `${slideHash(slide)}-${canvas.width}x${canvas.height}`, st.showChords) : "",
  );
  /** The render on screen (the previous one stays until the next has loaded). */
  let base = $state<{ src: string; hide: string[]; slide: string } | null>(null);
  /** The last render that showed every element (snapshots for drag previews). */
  let full = $state<string | null>(null);
  let renderFailed = $state(false);
  let clearPreview = false;

  $effect(() => {
    const src = wanted;
    const h = hide;
    const sid = slide?.id ?? "";
    if (!src) {
      base = null;
      return;
    }
    const img = new Image();
    img.onload = () => {
      if (wanted !== src) return;
      base = { src, hide: h, slide: sid };
      renderFailed = false;
      if (!h.length) {
        full = src;
        if (clearPreview) {
          clearPreview = false;
          st.preview = null;
          snapshot = null;
        }
      }
    };
    img.onerror = () => {
      if (wanted === src) renderFailed = true;
    };
    img.src = src;
  });

  // The edited text stays on screen until the render with the saved text has arrived.
  $effect(() => {
    if (closingText && !saving && base && !base.hide.includes(closingText) && base.src === wanted) closingText = null;
  });

  // ---- coordinates ------------------------------------------------------------------------------------

  function local(e: { clientX: number; clientY: number }) {
    const r = viewport!.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  function modelAt(e: { clientX: number; clientY: number }) {
    const p = local(e);
    return toModel(view, p.x, p.y);
  }

  function hitTest(x: number, y: number): Element | null {
    for (let i = shown.length - 1; i >= 0; i--) {
      const { el, rect } = shown[i]!;
      if (el.hidden || el.locked) continue;
      if (containsPoint(rect, el.rotation ?? 0, x, y)) return el;
    }
    return null;
  }

  function targetsExcept(ids: Set<string>): SnapTargets {
    const others = shown.filter((s) => !ids.has(s.el.id) && !s.el.hidden).map((s) => rotatedBounds(s.rect, s.el.rotation ?? 0));
    return snapTargets(canvas, others);
  }

  const SNAP_PX = 6;
  function snapOpt(e: { altKey?: boolean; ctrlKey?: boolean }) {
    return { threshold: SNAP_PX / view.zoom, grid: st.grid ? st.gridSize : 0, off: !st.snap || !!e.ctrlKey };
  }

  // ---- gestures -------------------------------------------------------------------------------------

  type Gesture =
    | { kind: "move"; sx: number; sy: number; rects: Map<string, Rect>; moved: boolean; targets: SnapTargets }
    | { kind: "resize"; id: string; handle: Handle; sx: number; sy: number; start: Rect; rotation: number; locked: boolean; targets: SnapTargets; moved: boolean }
    | { kind: "marquee"; sx: number; sy: number; x: number; y: number; additive: boolean; base: string[] }
    | { kind: "pan"; cx: number; cy: number; panX: number; panY: number };

  let gesture = $state.raw<Gesture | null>(null);
  let hover = $state<string | null>(null);
  let spaceDown = false;

  async function onPointerDown(e: PointerEvent) {
    if (!viewport || !slide) return;
    const target = e.target as HTMLElement;
    if (target.closest(".insert, .zoom-menu")) return;
    if (editorEl && editorEl.contains(target)) return;
    if (e.button === 1 || (e.button === 0 && spaceDown)) {
      e.preventDefault();
      gesture = { kind: "pan", cx: e.clientX, cy: e.clientY, panX: view.panX, panY: view.panY };
      viewport.setPointerCapture(e.pointerId);
      return;
    }
    if (e.button !== 0) return;
    // A click outside the edited text box saves it.
    if (st.editingText) await finishText();
    const m = modelAt(e);
    const handle = target.closest<SVGElement>("[data-handle]")?.dataset.handle as Handle | undefined;
    const one = st.single;
    if (handle && one && !one.locked) {
      gesture = {
        kind: "resize",
        id: one.id,
        handle,
        sx: m.x,
        sy: m.y,
        start: boundsOf(one),
        rotation: one.rotation ?? 0,
        locked: !!one.aspectLocked,
        targets: targetsExcept(new Set([one.id])),
        moved: false,
      };
      prepareSolo([one.id]);
      viewport.setPointerCapture(e.pointerId);
      return;
    }
    const hit = hitTest(m.x, m.y);
    if (hit) {
      if (e.shiftKey) {
        st.select(st.selection.includes(hit.id) ? st.selection.filter((x) => x !== hit.id) : [...st.selection, hit.id]);
        if (!st.selection.includes(hit.id)) return;
      } else if (!st.selection.includes(hit.id)) st.select([hit.id]);
      const ids = st.selected.filter((x) => !x.locked).map((x) => x.id);
      if (!ids.length) return;
      gesture = {
        kind: "move",
        sx: m.x,
        sy: m.y,
        rects: new Map(ids.map((id) => [id, boundsOf(st.elements.find((x) => x.id === id)!)])),
        moved: false,
        targets: targetsExcept(new Set(ids)),
      };
      prepareSolo(ids);
      viewport.setPointerCapture(e.pointerId);
      return;
    }
    // Empty canvas: marquee (Shift adds to the selection).
    gesture = { kind: "marquee", sx: m.x, sy: m.y, x: m.x, y: m.y, additive: e.shiftKey, base: e.shiftKey ? [...st.selection] : [] };
    if (!e.shiftKey) st.select([]);
    viewport.setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    const m = modelAt(e);
    st.pointer = { x: Math.round(m.x), y: Math.round(m.y) };
    const g = gesture;
    if (!g) {
      hover = hitTest(m.x, m.y)?.id ?? null;
      return;
    }
    switch (g.kind) {
      case "pan":
        st.viewMode = "manual";
        st.view = { ...view, panX: g.panX + e.clientX - g.cx, panY: g.panY + e.clientY - g.cy };
        return;
      case "marquee": {
        g.x = m.x;
        g.y = m.y;
        marquee = rectFromPoints(g.sx, g.sy, g.x, g.y);
        const hits = shown.filter((s) => !s.el.locked && !s.el.hidden && intersects(rotatedBounds(s.rect, s.el.rotation ?? 0), marquee!)).map((s) => s.el.id);
        st.select([...new Set([...g.base, ...hits])]);
        return;
      }
      case "move": {
        let dx = m.x - g.sx,
          dy = m.y - g.sy;
        if (!g.moved && Math.hypot(dx, dy) * view.zoom < 3) return;
        if (!g.moved) {
          g.moved = true;
          dragHide = [...g.rects.keys()];
          startSnapshot();
        }
        if (e.shiftKey) ({ dx, dy } = constrainAxis(dx, dy));
        const group = union([...g.rects.values()].map((r, i) => rotatedBounds(r, rotationOf([...g.rects.keys()][i]!))))!;
        let guides: Guide[] = [];
        const o = snapOpt(e);
        if (!o.off) {
          const s = snapMove({ ...group, x: group.x + dx, y: group.y + dy }, g.targets, o);
          if (e.shiftKey) {
            // Keep the constrained axis exactly.
            if (dx !== 0) dx = s.rect.x - group.x;
            if (dy !== 0) dy = s.rect.y - group.y;
            guides = s.guides.filter((gd) => (gd.axis === "x" ? dx !== 0 : dy !== 0));
          } else {
            dx = s.rect.x - group.x;
            dy = s.rect.y - group.y;
            guides = s.guides;
          }
        }
        const next = new Map<string, Rect>();
        for (const [id, r] of g.rects) next.set(id, { ...r, x: r.x + dx, y: r.y + dy });
        st.preview = next;
        st.guides = guides;
        return;
      }
      case "resize": {
        const dx = m.x - g.sx,
          dy = m.y - g.sy;
        if (!g.moved && Math.hypot(dx, dy) * view.zoom < 2) return;
        if (!g.moved) {
          g.moved = true;
          dragHide = [g.id];
          startSnapshot();
        }
        const o = snapOpt(e);
        const r = resizeRect(g.start, g.handle, dx, dy, {
          keepAspect: e.shiftKey || g.locked,
          fromCenter: e.altKey,
          rotation: g.rotation,
          snap: o.off ? null : { targets: g.targets, opt: o },
        });
        st.preview = new Map([[g.id, r.rect]]);
        st.guides = r.guides;
        return;
      }
    }
  }

  function rotationOf(id: string): number {
    return st.elements.find((e) => e.id === id)?.rotation ?? 0;
  }

  async function onPointerUp(e: PointerEvent) {
    const g = gesture;
    gesture = null;
    marquee = null;
    if (viewport?.hasPointerCapture(e.pointerId)) viewport.releasePointerCapture(e.pointerId);
    if (!g || !slide) return;
    if ((g.kind === "move" || g.kind === "resize") && g.moved && st.preview) {
      const moved = [...st.preview].map(([id, rect]) => ({ id, rect, was: boundsOf(st.elements.find((x) => x.id === id) ?? { id }) }));
      st.guides = [];
      const label = g.kind === "move" ? (moved.length > 1 ? "Move Objects" : "Move") : "Resize";
      await st.run(label, boundsOps(slide.id, moved), { keepPreview: true });
      // The preview stays until the render showing the result has loaded.
      clearPreview = true;
      dragHide = [];
    } else if (g.kind === "move" || g.kind === "resize") {
      st.preview = null;
      st.guides = [];
    }
  }

  function onDblClick(e: MouseEvent) {
    const m = modelAt(e);
    const hit = hitTest(m.x, m.y);
    if (hit?.kind?.type === "text") void startText(hit.id, false, e);
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    const p = local(e);
    st.viewMode = "manual";
    if (e.ctrlKey || e.metaKey) {
      const f = Math.exp(-e.deltaY * (e.deltaMode === 1 ? 0.05 : 0.0015));
      st.view = zoomAt(view, view.zoom * f, p.x, p.y);
    } else {
      const dx = e.shiftKey ? e.deltaY : e.deltaX;
      const dy = e.shiftKey ? 0 : e.deltaY;
      st.view = { ...view, panX: view.panX - dx, panY: view.panY - dy };
    }
  }

  function onKey(e: KeyboardEvent, down: boolean) {
    if (e.code === "Space" && !(e.target as HTMLElement)?.isContentEditable) spaceDown = down;
  }

  let marquee = $state.raw<Rect | null>(null);
  /** The picture dragged around: a render of only the dragged elements (else the full render). */
  let snapshot = $state<string | null>(null);
  let solo: { ids: string; src: string; ready: boolean } | null = null;

  /** Starts loading a render of just `ids` (everything else hidden) for the drag preview. */
  function prepareSolo(ids: string[]) {
    if (!pres || !slide) return;
    const key = [...ids].sort().join(",");
    if (solo?.ids === key && solo.src.includes(slideHash(slide))) return;
    const others = (slide.elements ?? []).map((e) => e.id).filter((id) => !ids.includes(id));
    const src = conn.rest.slideRenderUrl(pres.id, slide.id, renderW, others, `${slideHash(slide)}-${canvas.width}x${canvas.height}`, st.showChords);
    const entry = { ids: key, src, ready: false };
    solo = entry;
    const img = new Image();
    img.onload = () => {
      entry.ready = true;
      if (snapshot && gesture && solo === entry) snapshot = src;
    };
    img.src = src;
  }

  function startSnapshot() {
    snapshot = solo?.ready ? solo.src : full;
  }

  // ---- inserting ----------------------------------------------------------------------------------

  let shapeMenu = $state(false);
  let linkMenu = $state(false);
  let timerSub = $state(false);
  let imagePicker = $state(false);

  function addText() {
    void insertText(st, (id) => editText(id, true));
  }

  /** "Data link" insert: a text element showing live data (stage layouts, props, slides). */
  function addDataLink(c: DataLinkChoice) {
    linkMenu = false;
    timerSub = false;
    void insertDataLink(st, c);
  }

  function addShape(c: ShapeChoice) {
    shapeMenu = false;
    void insertShape(st, c);
  }

  function addImage(it: MediaBinItemInfo) {
    imagePicker = false;
    void insertImage(st, conn, it);
  }

  // ---- in-place text editing --------------------------------------------------------------------------

  let editorEl = $state<HTMLElement | null>(null);
  let editing = $derived(st.editingText ? (shown.find((s) => s.el.id === st.editingText) ?? null) : null);
  let editorFor = $derived(st.editingText ?? closingText);
  let editorShown = $derived(editorFor ? (shown.find((s) => s.el.id === editorFor) ?? null) : null);
  /** The text box as editing started, and where it is (the save goes there even when another
   * slide or presentation was opened meanwhile). */
  let orig: { tb: import("$protocol").TextBox; text: string; slide: string; presentation: string } | null = null;

  async function startText(id: string, selectAll: boolean, at?: MouseEvent) {
    const el = st.elements.find((e) => e.id === id);
    if (!el || el.kind?.type !== "text" || el.locked) return;
    if (st.editingText && st.editingText !== id) await finishText();
    st.select([id]);
    st.tab = "text";
    st.editingText = id;
    closingText = null;
    await tick();
    const root = editorEl;
    if (!root) return;
    orig = null;
    loadEditor(el);
    root.focus();
    const sel = window.getSelection();
    if (selectAll) {
      const r = document.createRange();
      r.selectNodeContents(root);
      sel?.removeAllRanges();
      sel?.addRange(r);
    } else if (at && "caretPositionFromPoint" in document) {
      const pos = (document as Document & { caretPositionFromPoint(x: number, y: number): { offsetNode: Node; offset: number } | null }).caretPositionFromPoint(at.clientX, at.clientY);
      if (pos && root.contains(pos.offsetNode)) {
        sel?.collapse(pos.offsetNode, pos.offset);
      } else placeAtEnd(root);
    } else placeAtEnd(root);
    fitText();
  }

  function placeAtEnd(root: HTMLElement) {
    const r = document.createRange();
    r.selectNodeContents(root);
    r.collapse(false);
    const sel = window.getSelection();
    sel?.removeAllRanges();
    sel?.addRange(r);
  }

  function loadEditor(el: Element) {
    const root = editorEl;
    if (!root || el.kind?.type !== "text") return;
    const tb = el.kind.text ?? {};
    // A reload (after a save) keeps where the text is; a new edit takes the slide shown.
    const slideId = orig?.slide ?? st.slideId;
    const presentation = orig?.presentation ?? st.presentation?.id;
    if (!slideId || !presentation) return;
    orig = { tb: structuredClone(tb), text: plainText(tb), slide: slideId, presentation };
    renderTextBox(root, tb, 1);
    st.textHooks = {
      draft: () => draft(),
      before: () => orig?.text ?? "",
      reload: () => {
        const now = st.elements.find((e) => e.id === st.editingText);
        const range = st.textRange;
        if (!now) return;
        loadEditor(now);
        if (range && editorEl) {
          const t = orig?.text ?? "";
          selectOffsets(editorEl, cpToUtf16(t, range.start), cpToUtf16(t, range.end));
        }
        fitText();
      },
    };
  }

  function draft(): import("$protocol").TextBox | null {
    if (!editorEl || !orig) return null;
    const now = rebuildTextBox(orig.tb, readEditor(editorEl).paragraphs);
    const before = rebuildTextBox(orig.tb, editedFromTextBox(orig.tb));
    return sameTextBox(now, before) ? null : now;
  }

  /** Saves and closes the in-place editor. */
  async function finishText() {
    const id = st.editingText;
    const o = orig;
    if (!id) return;
    const d = o ? draft() : null;
    st.editingText = null;
    st.textRange = null;
    st.textHooks = null;
    orig = null;
    if (!d || !o) {
      closingText = null;
      return;
    }
    closingText = id;
    if (editorEl) editorEl.contentEditable = "false";
    saving = true;
    try {
      await st.run("Typing", saveTextOps(o.slide, id, o.text, d), { presentation: o.presentation });
    } finally {
      saving = false;
    }
  }

  $effect(() => {
    st.finishText = finishText;
  });

  function onEditorKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      void finishText();
      return;
    }
    const mod = e.ctrlKey || e.metaKey;
    // No browser formatting (bold / italic tags): styling goes through the inspector.
    if (mod && ["KeyB", "KeyI", "KeyU"].includes(e.code)) e.preventDefault();
    e.stopPropagation();
  }

  function onEditorPaste(e: ClipboardEvent) {
    e.preventDefault();
    const t = e.clipboardData?.getData("text/plain") ?? "";
    document.execCommand("insertText", false, t);
  }

  function onSelectionChange() {
    const root = editorEl;
    if (!root || !st.editingText) return;
    const sel = window.getSelection();
    if (!sel || !sel.rangeCount || !sel.anchorNode || !root.contains(sel.anchorNode) || !sel.focusNode || !root.contains(sel.focusNode)) return;
    const { text } = readEditor(root);
    const a = offsetOf(root, sel.anchorNode, sel.anchorOffset);
    const b = offsetOf(root, sel.focusNode, sel.focusOffset);
    st.textRange = rangeFromUtf16(text, a, b);
  }

  /** Shrink / scale to fit, approximated with a font-size factor. */
  function fitText() {
    const root = editorEl;
    const s = editing ?? editorShown;
    if (!root || !s || s.el.kind?.type !== "text") return;
    const fit = s.el.kind.text?.fit;
    root.style.setProperty("--fs", "1");
    if (fit !== "shrinkToFit" && fit !== "scaleToFit") return;
    const box = root.parentElement!;
    const availH = box.clientHeight - parseFloat(getComputedStyle(box).paddingTop) - parseFloat(getComputedStyle(box).paddingBottom);
    let fs = 1;
    for (let i = 0; i < 4; i++) {
      const h = root.scrollHeight;
      if (!h) break;
      const want = availH / h;
      if (fit === "shrinkToFit" && want >= 1 && fs === 1) break;
      fs = Math.max(0.05, fs * want);
      if (fit === "shrinkToFit") fs = Math.min(1, fs);
      root.style.setProperty("--fs", String(fs));
    }
  }

  $effect(() => {
    document.addEventListener("selectionchange", onSelectionChange);
    return () => document.removeEventListener("selectionchange", onSelectionChange);
  });

  onDestroy(() => {
    void finishText();
    st.finishText = null;
  });

  // ---- view ----------------------------------------------------------------------------------------------

  let zoomMenu = $state(false);
  function setZoom(z: number | "fit" | "width") {
    zoomMenu = false;
    if (z === "fit" || z === "width") st.viewMode = z;
    else {
      st.viewMode = "manual";
      st.view = zoomAt(view, z, vw / 2, vh / 2);
    }
  }

  /** Esc during a drag: drop the gesture, nothing changes. True when there was one. */
  export function cancelGesture(): boolean {
    const g = gesture;
    if (!g) return false;
    gesture = null;
    marquee = null;
    st.preview = null;
    st.guides = [];
    dragHide = [];
    snapshot = null;
    return true;
  }

  /** Starts editing a text element in place (Enter; a new text box with its text selected). */
  export function editText(id: string, selectAll = false) {
    void tick().then(() => startText(id, selectAll));
  }

  export function zoomStep(dir: 1 | -1) {
    setZoom(stepZoom(view.zoom, dir));
  }

  let selBounds = $derived(union(st.selection.map((id) => shown.find((s) => s.el.id === id)).filter((s) => !!s).map((s) => rotatedBounds(s.rect, s.el.rotation ?? 0))));
  let slideScreen = $derived({ x: view.panX, y: view.panY, w: canvas.width * view.zoom, h: canvas.height * view.zoom });
  const fmt = (v: number | undefined) => (v === undefined ? "----" : String(Math.round(v)));

  function polyFor(r: Rect, rot: number): string {
    const z = view.zoom;
    const cx = (r.x + r.width / 2) * z + view.panX,
      cy = (r.y + r.height / 2) * z + view.panY;
    const t = (rot * Math.PI) / 180;
    const pts = [
      [-1, -1],
      [1, -1],
      [1, 1],
      [-1, 1],
    ].map(([a, b]) => {
      const ox = (a! * r.width * z) / 2,
        oy = (b! * r.height * z) / 2;
      return `${(cx + ox * Math.cos(t) - oy * Math.sin(t)).toFixed(1)},${(cy + ox * Math.sin(t) + oy * Math.cos(t)).toFixed(1)}`;
    });
    return pts.join(" ");
  }

  function handlePos(r: Rect, rot: number, h: Handle) {
    const z = view.zoom;
    const { u, v } = handleUnit(h);
    const cx = (r.x + r.width / 2) * z + view.panX,
      cy = (r.y + r.height / 2) * z + view.panY;
    const t = (rot * Math.PI) / 180;
    const ox = (u - 0.5) * r.width * z,
      oy = (v - 0.5) * r.height * z;
    return { x: cx + ox * Math.cos(t) - oy * Math.sin(t), y: cy + ox * Math.sin(t) + oy * Math.cos(t) };
  }

  const CURSORS: Record<Handle, string> = { nw: "nwse-resize", se: "nwse-resize", ne: "nesw-resize", sw: "nesw-resize", n: "ns-resize", s: "ns-resize", e: "ew-resize", w: "ew-resize" };

  /** CSS for the in-place editor box: model-sized, scaled and rotated onto the canvas. */
  function editorBoxStyle(s: { el: Element; rect: Rect }): string {
    const r = s.rect;
    const tb = s.el.kind?.type === "text" ? s.el.kind.text : undefined;
    const m = tb?.margins ?? {};
    const va = tb?.verticalAlign === "top" ? "flex-start" : tb?.verticalAlign === "bottom" ? "flex-end" : "center";
    const fill = fillDrawn(s.el.fill) && s.el.fill?.kind?.type === "color" ? cssColor(s.el.fill.kind.color) : null;
    const z = view.zoom;
    return [
      `width: ${r.width}px`,
      `height: ${r.height}px`,
      `transform: translate(${r.x * z + view.panX}px, ${r.y * z + view.panY}px) scale(${z}) translate(${r.width / 2}px, ${r.height / 2}px) rotate(${s.el.rotation ?? 0}deg) translate(${-r.width / 2}px, ${-r.height / 2}px)`,
      `padding: ${m.top ?? 0}px ${m.right ?? 0}px ${m.bottom ?? 0}px ${m.left ?? 0}px`,
      `justify-content: ${va}`,
      fill ? `background: ${fill}` : "",
      `opacity: ${s.el.opacity ?? 1}`,
    ]
      .filter(Boolean)
      .join("; ");
  }

  function ghostStyle(id: string): { box: string; img: string } | null {
    const s = shown.find((x) => x.el.id === id);
    const el = st.elements.find((x) => x.id === id);
    if (!s || !el) return null;
    const rot = el.rotation ?? 0;
    const from = rotatedBounds(boundsOf(el), rot);
    const to = rotatedBounds(s.rect, rot);
    const z = view.zoom;
    const sx = from.width ? to.width / from.width : 1;
    const sy = from.height ? to.height / from.height : 1;
    return {
      box: `left:${to.x * z + view.panX}px; top:${to.y * z + view.panY}px; width:${to.width * z}px; height:${to.height * z}px`,
      img: `left:${-from.x * z * sx}px; top:${-from.y * z * sy}px; width:${canvas.width * z * sx}px; height:${canvas.height * z * sy}px`,
    };
  }

  let gridPattern = $derived(st.grid ? Math.max(2, st.gridSize * view.zoom) : 0);
</script>

<svelte:window onkeydown={(e) => onKey(e, true)} onkeyup={(e) => onKey(e, false)} />

<div class="wrap" class:norulers={!st.rulers}>
  {#if st.rulers}
    <div class="corner"></div>
    <Ruler zoom={view.zoom} pan={view.panX} mark={selBounds ? [selBounds.x, selBounds.x + selBounds.width] : null} />
    <Ruler vertical zoom={view.zoom} pan={view.panY} mark={selBounds ? [selBounds.y, selBounds.y + selBounds.height] : null} />
  {/if}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="viewport"
    bind:this={viewport}
    bind:clientWidth={vw}
    bind:clientHeight={vh}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    onpointerleave={() => {
      if (!gesture) st.pointer = null;
      hover = null;
    }}
    ondblclick={onDblClick}
    onwheel={onWheel}
    data-edit-canvas
  >
    {#if slide}
      <div
        class="slide"
        class:checker={st.checker}
        style:left={`${slideScreen.x}px`}
        style:top={`${slideScreen.y}px`}
        style:width={`${slideScreen.w}px`}
        style:height={`${slideScreen.h}px`}
      >
        {#if base && base.slide === slide.id}<img class="render" src={base.src} alt="" draggable="false" data-render={base.src} />{/if}
        {#if gridPattern}
          <svg class="grid" width="100%" height="100%" aria-hidden="true">
            <defs>
              <pattern id="edit-grid" width={gridPattern} height={gridPattern} patternUnits="userSpaceOnUse">
                <path d={`M ${gridPattern} 0 L 0 0 0 ${gridPattern}`} fill="none" stroke="rgba(255,255,255,0.18)" stroke-width="1" />
              </pattern>
            </defs>
            <rect width="100%" height="100%" fill="url(#edit-grid)" />
          </svg>
        {/if}
      </div>

      <!-- Drag previews: a snapshot of each moving element, cropped from the last full render. -->
      {#if st.preview && snapshot}
        {#each [...st.preview.keys()] as id (id)}
          {@const g = ghostStyle(id)}
          {#if g}
            <div class="ghost" style={g.box}><img src={snapshot} alt="" draggable="false" style={g.img} /></div>
          {/if}
        {/each}
      {/if}

      {#if editorShown}
        <div class="textbox" style={editorBoxStyle(editorShown)} data-text-editor-box>
          <div
            class="texted"
            bind:this={editorEl}
            contenteditable={st.editingText ? "true" : "false"}
            spellcheck="false"
            role="textbox"
            tabindex="-1"
            aria-multiline="true"
            aria-label="Text"
            onkeydown={onEditorKey}
            oninput={fitText}
            onpaste={onEditorPaste}
            data-text-editor
          ></div>
        </div>
      {/if}

      <svg class="overlay" width={vw} height={vh} aria-hidden="true">
        {#each shown as s (s.el.id)}
          {#if s.el.hidden && st.selection.includes(s.el.id)}
            <polygon points={polyFor(s.rect, s.el.rotation ?? 0)} class="hidden-outline" />
          {/if}
        {/each}
        {#if hover && !gesture && !st.selection.includes(hover)}
          {@const s = shown.find((x) => x.el.id === hover)}
          {#if s}<polygon points={polyFor(s.rect, s.el.rotation ?? 0)} class="hover" />{/if}
        {/if}
        {#each st.selection as id (id)}
          {@const s = shown.find((x) => x.el.id === id)}
          {#if s}<polygon points={polyFor(s.rect, s.el.rotation ?? 0)} class="sel" class:locked={s.el.locked} data-selected={id} />{/if}
        {/each}
        {#if st.selection.length > 1 && selBounds}
          <rect class="group" x={selBounds.x * view.zoom + view.panX} y={selBounds.y * view.zoom + view.panY} width={selBounds.width * view.zoom} height={selBounds.height * view.zoom} />
        {/if}
        {#if st.single && !st.single.locked && !st.editingText}
          {@const s = shown.find((x) => x.el.id === st.single?.id)}
          {#if s}
            {#each HANDLES as h (h)}
              {@const p = handlePos(s.rect, s.el.rotation ?? 0, h)}
              <rect class="handle" x={p.x - 4.5} y={p.y - 4.5} width="9" height="9" data-handle={h} style:cursor={CURSORS[h]} />
            {/each}
          {/if}
        {/if}
        {#each st.guides as g, i (i)}
          {#if g.axis === "x"}
            <line class="guide" x1={g.pos * view.zoom + view.panX} x2={g.pos * view.zoom + view.panX} y1="0" y2={vh} data-guide="x" />
          {:else}
            <line class="guide" y1={g.pos * view.zoom + view.panY} y2={g.pos * view.zoom + view.panY} x1="0" x2={vw} data-guide="y" />
          {/if}
        {/each}
        {#if marquee}
          <rect class="marquee" x={marquee.x * view.zoom + view.panX} y={marquee.y * view.zoom + view.panY} width={marquee.width * view.zoom} height={marquee.height * view.zoom} />
        {/if}
      </svg>

      {#if isLive}<span class="livebadge" style:left={`${slideScreen.x + 8}px`} style:top={`${slideScreen.y + 8}px`} title="This slide is live: edits show on the output at once">LIVE</span>{/if}
      {#if renderFailed}<span class="renderfail">The server could not render this slide.</span>{/if}
    {:else}
      <div class="empty">{session.openLoading ? "" : "This presentation has no slides. Use + to add one."}</div>
    {/if}

    {#if insertBar}
    <div class="insert" role="toolbar" aria-label="Insert">
      <button class="ib" onclick={addText} title="Text box" aria-label="Insert text box" disabled={!slide}><Type size={24} strokeWidth={2.2} /></button>
      <span class="isep"></span>
      <div class="pop-anchor">
        <button class="ib shape" onclick={() => ((shapeMenu = !shapeMenu), (linkMenu = false))} title="Shape" aria-label="Insert shape" aria-expanded={shapeMenu} disabled={!slide}>
          <Shapes size={22} strokeWidth={2} /><ChevronDown size={12} />
        </button>
        {#if shapeMenu}
          <div class="menu" role="menu">
            {#each SHAPE_CHOICES as c (c.id)}
              <button role="menuitem" onclick={() => addShape(c.id)} data-shape={c.id}>{c.label}</button>
            {/each}
          </div>
        {/if}
      </div>
      <span class="isep"></span>
      <div class="pop-anchor">
        <button class="ib shape" onclick={() => ((linkMenu = !linkMenu), (timerSub = false), (shapeMenu = false))} title="Data link: live text (current / next slide, notes, clock, timers, …)" aria-label="Insert data link" aria-expanded={linkMenu} disabled={!slide} data-testid="insert-data-link">
          <LinkIcon size={21} strokeWidth={2} /><ChevronDown size={12} />
        </button>
        {#if linkMenu}
          <div class="menu links" role="menu">
            {#each DATA_LINKS.slice(0, 4) as c (c.id)}
              <button role="menuitem" onclick={() => addDataLink(c)} data-link={c.id}>{c.label}</button>
            {/each}
            <div class="subanchor">
              <button role="menuitem" class="has-sub" aria-haspopup="menu" aria-expanded={timerSub} onclick={() => (timerSub = !timerSub)} onpointerenter={() => (timerSub = true)} data-link="timer">
                <span>Timer</span><ChevronRight size={13} />
              </button>
              {#if timerSub}
                <div class="menu sub" role="menu">
                  <button role="menuitem" onclick={() => addDataLink(timerLink(null))}>Most Recently Started</button>
                  {#each timersState.timerDocs as t (t.id)}
                    <button role="menuitem" onclick={() => addDataLink(timerLink(t))}>{t.name || "Timer"}</button>
                  {/each}
                </div>
              {/if}
            </div>
            {#each DATA_LINKS.slice(4) as c (c.id)}
              <button role="menuitem" onclick={() => addDataLink(c)} onpointerenter={() => (timerSub = false)} data-link={c.id}>{c.label}</button>
            {/each}
          </div>
        {/if}
      </div>
      <span class="isep"></span>
      <button class="ib" disabled title="Pen (vector paths): coming later" aria-label="Pen (coming later)"><PenTool size={21} strokeWidth={2} /></button>
      <span class="isep"></span>
      <div class="pop-anchor">
        <button class="ib" onclick={() => (imagePicker = !imagePicker)} title="Image from the media bins" aria-label="Insert image" disabled={!slide}><ImageIcon size={22} strokeWidth={2} /></button>
        {#if imagePicker}<MediaPicker {conn} onpick={addImage} onclose={() => (imagePicker = false)} />{/if}
      </div>
      <span class="isep"></span>
      <button class="ib" disabled title="Video elements in slides arrive in a later version" aria-label="Video (coming later)"><Video size={22} strokeWidth={2} /></button>
      <span class="isep"></span>
      <button class="ib" disabled title="Web elements: not available yet" aria-label="Web (not available)"><Globe size={21} strokeWidth={2} /></button>
    </div>
    {/if}
  </div>

  <footer class="status">
    <span class="xy">X: {st.pointer ? st.pointer.x : "----"}&nbsp; Y: {st.pointer ? st.pointer.y : "----"}</span>
    <span class="wh">W: {fmt(selBounds?.width)}&nbsp; H: {fmt(selBounds?.height)}</span>
    {#if st.info}<span class="info" role="status">{st.info}</span>{/if}
    <span class="grow"></span>
    <button class="sb" class:on={st.checker} onclick={() => (st.checker = !st.checker)} title="Show transparency (checkerboard)" aria-pressed={st.checker} aria-label="Transparency checkerboard">
      <svg width="16" height="16" viewBox="0 0 16 16" aria-hidden="true"><rect x="1" y="1" width="14" height="14" fill="none" stroke="currentColor" /><path d="M1 1h4.67v4.67H1zM10.33 1H15v4.67h-4.67zM5.67 5.67h4.67v4.67H5.67zM1 10.33h4.67V15H1zM10.33 10.33H15V15h-4.67z" fill="currentColor" /></svg>
    </button>
    <button class="sb" class:on={st.viewMode === "width"} onclick={() => setZoom("width")} title="Fit to width" aria-label="Fit to width"><MoveHorizontal size={17} /></button>
    <button class="sb" class:on={st.grid} onclick={() => (st.grid = !st.grid)} title="Grid" aria-pressed={st.grid} aria-label="Grid"><Grid3x3 size={16} /></button>
    {#if st.grid}
      <select class="gsize" bind:value={st.gridSize} title="Grid spacing (canvas px)" aria-label="Grid spacing">
        {#each [10, 20, 40, 60, 80, 120, 160] as g (g)}<option value={g}>{g}</option>{/each}
      </select>
    {/if}
    <button class="sb" class:on={st.snap} onclick={() => (st.snap = !st.snap)} title="Snapping (hold Ctrl to skip)" aria-pressed={st.snap} aria-label="Snapping"><Magnet size={16} /></button>
    <div class="pop-anchor up">
      <button class="sb zoom" onclick={() => (zoomMenu = !zoomMenu)} title="Zoom" aria-label="Zoom" data-zoom={view.zoom}><ZoomIn size={16} /><span>{Math.round(view.zoom * 100)}%</span></button>
      {#if zoomMenu}
        <div class="menu zoom-menu" role="menu">
          <button role="menuitem" onclick={() => setZoom("fit")}>Fit</button>
          <button role="menuitem" onclick={() => setZoom("width")}>Fit Width</button>
          {#each [0.25, 0.5, 0.75, 1, 1.5, 2, 3, 4] as z (z)}<button role="menuitem" onclick={() => setZoom(z)}>{z * 100}%</button>{/each}
        </div>
      {/if}
    </div>
  </footer>
</div>

<style>
  .wrap {
    height: 100%;
    display: grid;
    grid-template-columns: 20px 1fr;
    grid-template-rows: 20px 1fr 28px;
    min-height: 0;
    min-width: 0;
    background: var(--surface-panel);
  }
  .wrap.norulers {
    grid-template-columns: 1fr;
    grid-template-rows: 1fr 28px;
  }
  .corner {
    background: var(--surface-panel);
    border-right: 1px solid var(--line);
    border-bottom: 1px solid var(--line);
  }
  .wrap > :global(.ruler:not(.vertical)) {
    border-bottom: 1px solid var(--line);
  }
  .wrap > :global(.ruler.vertical) {
    border-right: 1px solid var(--line);
  }
  .viewport {
    position: relative;
    overflow: hidden;
    background: var(--surface-backdrop);
    touch-action: none;
    min-width: 0;
    min-height: 0;
  }
  .slide {
    position: absolute;
    background: var(--surface-sunk);
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.4);
    overflow: hidden;
  }
  .slide.checker {
    background-color: var(--line-hover);
    background-image: conic-gradient(var(--line-strong) 25%, var(--line-hover) 0 50%, var(--line-strong) 0 75%, var(--line-hover) 0);
    background-size: 24px 24px;
  }
  .render {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }
  .grid {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .ghost {
    position: absolute;
    overflow: hidden;
    opacity: 0.6;
    pointer-events: none;
  }
  .ghost img {
    position: absolute;
    max-width: none;
  }
  .textbox {
    position: absolute;
    left: 0;
    top: 0;
    transform-origin: 0 0;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    overflow: visible;
  }
  .texted {
    outline: none;
    white-space: pre-wrap;
    overflow-wrap: break-word;
    user-select: text;
    cursor: text;
    caret-color: var(--accent-ring);
  }
  .overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
    overflow: visible;
  }
  .overlay polygon,
  .overlay rect {
    fill: none;
  }
  .sel {
    stroke: var(--accent-ring);
    stroke-width: 1.5;
  }
  .sel.locked {
    stroke: var(--live);
    stroke-dasharray: 4 3;
  }
  .hover {
    stroke: var(--accent);
    stroke-width: 1;
    stroke-dasharray: 3 3;
  }
  .hidden-outline {
    stroke: rgba(255, 255, 255, 0.5);
    stroke-dasharray: 2 3;
  }
  .group {
    stroke: var(--accent);
    stroke-dasharray: 5 4;
  }
  .overlay .handle {
    fill: var(--text-strong);
    stroke: var(--accent-ring);
    stroke-width: 1.5;
    pointer-events: all;
  }
  .guide {
    stroke: #ff3db4;
    stroke-width: 1;
  }
  .overlay .marquee {
    fill: rgba(110, 92, 230, 0.14);
    stroke: var(--accent-ring);
    stroke-width: 1;
  }
  .livebadge {
    position: absolute;
    padding: 2px 7px;
    border-radius: 3px;
    background: var(--live);
    color: #111;
    font-size: 11px;
    font-weight: 800;
    letter-spacing: 0.06em;
    pointer-events: none;
  }
  .renderfail,
  .empty {
    position: absolute;
    left: 50%;
    top: 50%;
    transform: translate(-50%, -50%);
    color: var(--text);
    background: rgba(0, 0, 0, 0.5);
    padding: 6px 12px;
    border-radius: 4px;
  }
  .empty:empty {
    display: none;
  }
  .insert {
    position: absolute;
    top: 10px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    height: 44px;
    padding: 0 6px;
    border-radius: 3px;
    background: var(--surface-panel);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.45);
    z-index: 5;
  }
  .ib {
    width: 64px;
    height: 36px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 3px;
    border: 0;
    border-radius: 3px;
    background: transparent;
    color: var(--text);
  }
  .ib:hover:not(:disabled) {
    background: var(--surface-raised);
  }
  .ib:disabled {
    color: var(--text-meta);
    cursor: default;
  }
  .isep {
    width: 1px;
    height: 24px;
    background: var(--surface-overlay);
  }
  .pop-anchor {
    position: relative;
  }
  .menu {
    position: absolute;
    top: 40px;
    left: 0;
    min-width: 170px;
    padding: 4px 0;
    background: var(--menu-bg);
    border: 1px solid var(--line-strong);
    border-radius: 3px;
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.5);
    z-index: 10;
  }
  .menu button {
    display: block;
    width: 100%;
    padding: 5px 14px;
    border: 0;
    background: transparent;
    text-align: left;
    color: var(--text-strong);
  }
  .menu button:hover {
    background: var(--menu-hover);
  }
  .menu.links {
    min-width: 200px;
  }
  .subanchor {
    position: relative;
  }
  .menu .has-sub {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .menu.sub {
    top: -5px;
    left: 100%;
    min-width: 190px;
  }
  .up .menu {
    top: auto;
    bottom: 26px;
    left: auto;
    right: 0;
    min-width: 110px;
  }
  .status {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 14px;
    padding: 0 10px;
    background: var(--surface-panel);
    border-top: 1px solid var(--line);
    font-size: 12.5px;
    color: var(--text);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .xy {
    min-width: 120px;
  }
  .info {
    color: var(--accent-link);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .grow {
    flex: 1;
  }
  .sb {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 4px;
    border: 0;
    border-radius: 3px;
    background: transparent;
    color: var(--text-label);
  }
  .sb.on {
    color: var(--accent-ring);
  }
  .sb:hover {
    background: var(--surface-raised);
  }
  .sb.zoom {
    color: var(--accent-ring);
  }
  .gsize {
    height: 20px;
    background: var(--surface-panel);
    border: 1px solid var(--line-strong);
    border-radius: 2px;
    font-size: 11.5px;
  }
</style>
