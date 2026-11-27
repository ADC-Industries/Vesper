<script lang="ts">
  // The mask editor (docs/LOOKS-MASKS-PLAN.md §6), a dialog of the Looks tab: the box's masks
  // (new, duplicate, delete: a mask that looks use asks first, then they forget it); the
  // selected mask's preview, name, mode (overlay: drawn over the screen; keyhole: the screen
  // shows only through the shapes, the rest is the colour), colour and feather; its shapes
  // (rectangle, ellipse, polygon) with position, size and colour. Edits are a draft, saved with
  // Save (`PUT /masks/{id}`, If-Match).
  import type { Mask, MaskEntry, MaskMode } from "$protocol";
  import Copy from "lucide-svelte/icons/copy";
  import Plus from "lucide-svelte/icons/plus";
  import Trash from "lucide-svelte/icons/trash-2";
  import { Button, ColorWell, Dialog, Segmented } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { ApiRequestError } from "../../server/rest";
  import { colorToHex, hexToColor } from "../../state/screens";
  import Num from "../editor/Num.svelte";
  import { useLive } from "../../live";
  import { SHAPES, canvasOf, fillOf, maskUsers, maskUsersText, newShape, polygonPoints, setBound, shapeOf, withElement, withElements, type MaskShape, type MaskUsers } from "./masks";

  let {
    initial = null,
    onclose,
    onchanged,
  }: {
    /** The mask to open first. */
    initial?: string | null;
    onclose: () => void;
    /** After a save, create or delete (the Looks tab reloads its masks and looks). */
    onchanged: (masks: MaskEntry[]) => void;
  } = $props();

  let masks = $state.raw<MaskEntry[]>([]);
  let selId = $state<string | null>(null);
  let draft = $state<Mask | null>(null);
  let element = $state<string | null>(null);
  let busy = $state(false);
  let error = $state("");
  let askDelete = $state<MaskUsers | null>(null);
  const liveScreens = useLive().screens;

  let entry = $derived(masks.find((m) => m.id === selId) ?? null);
  let dirty = $derived(!!draft && !!entry && JSON.stringify(strip(entry)) !== JSON.stringify(draft));
  let el = $derived(draft?.slide.elements?.find((e) => e.id === element) ?? null);
  let canvas = $derived(draft ? canvasOf(draft) : { width: 1920, height: 1080 });

  function strip(e: MaskEntry): Mask {
    const { revision: _r, ...m } = e;
    return m as Mask;
  }
  function errorText(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  async function load(pick: string | null) {
    const rest = session.conn?.rest;
    if (!rest) return;
    try {
      masks = await rest.masks();
      onchanged(masks);
      const id = pick && masks.some((m) => m.id === pick) ? pick : (masks[0]?.id ?? null);
      select(id);
    } catch (e) {
      error = `Could not load the masks: ${errorText(e)}`;
    }
  }

  /** Reloads the list (newer revisions) without touching the draft. */
  async function refreshKeepingDraft() {
    const rest = session.conn?.rest;
    if (!rest) return;
    try {
      masks = await rest.masks();
      onchanged(masks);
      if (!masks.some((m) => m.id === selId)) {
        const name = draft?.name?.trim();
        select(masks[0]?.id ?? null);
        // After select(), which clears the error: the operator sees why the editor moved on (E08-3).
        error = `${name ? `“${name}”` : "This mask"} was deleted by another operator, with your unsaved changes to it.${selId ? " Showing another mask." : ""}`;
      }
    } catch {
      // Keep what's shown; the next action reports the connection.
    }
  }

  function select(id: string | null) {
    selId = id;
    const e = masks.find((m) => m.id === id);
    draft = e ? $state.snapshot(strip(e)) : null;
    element = draft?.slide.elements?.[0]?.id ?? null;
    error = "";
  }

  $effect(() => {
    void load(initial);
  });

  async function act(what: string, run: () => Promise<void>) {
    if (busy) return;
    busy = true;
    error = "";
    try {
      await run();
    } catch (e) {
      if (e instanceof ApiRequestError && e.status === 409) {
        // Changed or deleted elsewhere: load the masks again but keep the draft, so Save
        // overwrites knowingly and Revert shows theirs (D08-4).
        error = what === "save the mask" ? `${errorText(e)} Save again to replace their version with yours, or Revert to see theirs.` : errorText(e);
        await refreshKeepingDraft();
      } else error = `Could not ${what}: ${errorText(e)}`;
    } finally {
      busy = false;
    }
  }

  function edit(f: (m: Mask) => Mask) {
    if (draft) draft = f(draft);
  }

  function save() {
    const rest = session.conn?.rest;
    const d = draft;
    const e0 = entry;
    if (!rest || !d || !e0) return;
    void act("save the mask", async () => {
      const e = await rest.updateMask(d, e0.revision);
      await load(e.id);
    });
  }

  function create() {
    const rest = session.conn?.rest;
    if (!rest) return;
    void act("add a mask", async () => {
      const e = await rest.createMask({});
      await load(e.id);
    });
  }

  function duplicate() {
    const rest = session.conn?.rest;
    if (!rest || !selId) return;
    const id = selId;
    void act("duplicate the mask", async () => {
      const e = await rest.duplicateMask(id);
      await load(e.id);
    });
  }

  function remove(detach: boolean) {
    const rest = session.conn?.rest;
    const e0 = entry;
    if (!rest || !e0) return;
    askDelete = null;
    void act("delete the mask", async () => {
      try {
        await rest.deleteMask(e0.id, e0.revision, detach);
        await load(null);
      } catch (e) {
        const users = e instanceof ApiRequestError ? maskUsers(e.current, (id) => $liveScreens.find((s) => s.id === id)?.name) : null;
        if (!detach && users) askDelete = users;
        else throw e;
      }
    });
  }

  function addShape(shape: MaskShape) {
    if (!draft) return;
    const n = (draft.slide.elements?.length ?? 0) + 1;
    const e = newShape(draft, shape, n);
    edit((m) => withElements(m, (els) => [...els, e]));
    element = e.id;
  }

  function removeShape(id: string) {
    edit((m) => withElements(m, (els) => els.filter((e) => e.id !== id)));
    if (element === id) element = draft?.slide.elements?.[0]?.id ?? null;
  }

  const MODES: { value: MaskMode; label: string }[] = [
    { value: "overlay", label: "Overlay" },
    { value: "keyhole", label: "Keyhole" },
  ];
  let mode = $derived<MaskMode>(draft?.mode === "keyhole" ? "keyhole" : "overlay");
</script>

<Dialog title="Masks" width={980} bare {onclose}>
  <div class="me" data-testid="mask-editor">
    <aside class="list" aria-label="Masks">
      <div class="lhead">
        <span class="b">Masks</span>
        <Button size="sm" variant="ghost" icon={Plus} onclick={create} disabled={busy} data-testid="mask-new">New</Button>
      </div>
      {#each masks as m (m.id)}
        <button class="item" class:on={m.id === selId} onclick={() => select(m.id)} data-testid="mask-item">
          <span class="name">{m.name || "Unnamed mask"}</span>
          <span class="meta">{m.mode === "keyhole" ? "Keyhole" : "Overlay"}</span>
        </button>
      {:else}
        <p class="note">No masks yet. A mask is drawn over a screen after all its layers.</p>
      {/each}
    </aside>

    {#if draft}
      {@const d = draft}
      <section class="stage" aria-label="Preview">
        <svg viewBox={`0 0 ${canvas.width} ${canvas.height}`} class="preview" role="img" aria-label="Mask preview" data-testid="mask-preview">
          <defs>
            <pattern id="mask-checker" width="60" height="60" patternUnits="userSpaceOnUse">
              <rect width="60" height="60" fill="#2A2D35" /><rect width="30" height="30" fill="#3A3D46" /><rect x="30" y="30" width="30" height="30" fill="#3A3D46" />
            </pattern>
            <mask id="mask-hole">
              <rect width={canvas.width} height={canvas.height} fill="white" />
              {#each d.slide.elements ?? [] as e (e.id)}
                {@const b = e.bounds ?? { x: 0, y: 0, width: 0, height: 0 }}
                {#if shapeOf(e) === "ellipse"}
                  <ellipse cx={(b.x ?? 0) + (b.width ?? 0) / 2} cy={(b.y ?? 0) + (b.height ?? 0) / 2} rx={(b.width ?? 0) / 2} ry={(b.height ?? 0) / 2} fill="black" />
                {:else if shapeOf(e) === "polygon"}
                  <polygon points={polygonPoints(b, e.kind?.type === "shape" && e.kind.shape?.type === "polygon" ? e.kind.shape.sides : 6)} fill="black" />
                {:else}
                  <rect x={b.x ?? 0} y={b.y ?? 0} width={b.width ?? 0} height={b.height ?? 0} fill="black" />
                {/if}
              {/each}
            </mask>
            <filter id="mask-feather"><feGaussianBlur stdDeviation={(d.feather ?? 0) / 2} /></filter>
          </defs>
          <!-- The screen behind: a checkerboard stands for whatever is live. -->
          <rect width={canvas.width} height={canvas.height} fill="url(#mask-checker)" />
          <g filter={(d.feather ?? 0) > 0 ? "url(#mask-feather)" : undefined}>
            {#if mode === "keyhole"}
              <rect width={canvas.width} height={canvas.height} fill={colorToHex(d.color)} fill-opacity={d.color?.a ?? 1} mask="url(#mask-hole)" />
            {:else}
              {#each d.slide.elements ?? [] as e (e.id)}
                {@const b = e.bounds ?? { x: 0, y: 0, width: 0, height: 0 }}
                {@const c = fillOf(e)}
                {#if shapeOf(e) === "ellipse"}
                  <ellipse cx={(b.x ?? 0) + (b.width ?? 0) / 2} cy={(b.y ?? 0) + (b.height ?? 0) / 2} rx={(b.width ?? 0) / 2} ry={(b.height ?? 0) / 2} fill={colorToHex(c)} fill-opacity={c.a ?? 1} />
                {:else if shapeOf(e) === "polygon"}
                  <polygon points={polygonPoints(b, e.kind?.type === "shape" && e.kind.shape?.type === "polygon" ? e.kind.shape.sides : 6)} fill={colorToHex(c)} fill-opacity={c.a ?? 1} />
                {:else}
                  <rect x={b.x ?? 0} y={b.y ?? 0} width={b.width ?? 0} height={b.height ?? 0} fill={colorToHex(c)} fill-opacity={c.a ?? 1} />
                {/if}
              {/each}
            {/if}
          </g>
          {#if el?.bounds}
            {@const b = el.bounds}
            <rect class="sel" x={b.x ?? 0} y={b.y ?? 0} width={b.width ?? 0} height={b.height ?? 0} fill="none" />
          {/if}
        </svg>
        <p class="note">
          {mode === "keyhole"
            ? "Keyhole: the screen shows only through the shapes; everywhere else is the mask's colour."
            : "Overlay: the shapes are drawn over the screen, on top of every layer."}
          The checkerboard stands for the live picture.
        </p>
      </section>

      <section class="props" aria-label="Mask settings">
        <label class="field"><span>Name</span><input value={d.name ?? ""} maxlength={64} oninput={(e) => { const name = e.currentTarget.value; edit((m) => ({ ...m, name })); }} data-testid="mask-name" /></label>
        <div class="field"><span>Mode</span><Segmented size="sm" label="Mask mode" options={MODES} value={mode} onchange={(v) => edit((m) => ({ ...m, mode: v }))} /></div>
        {#if mode === "keyhole"}
          <div class="field"><span>Colour outside</span><ColorWell label="Colour outside the shapes" value={colorToHex(d.color)} onchange={(v) => edit((m) => ({ ...m, color: hexToColor(v) }))} /></div>
        {/if}
        <div class="field"><span>Feather</span><Num label="Feather" value={d.feather ?? 0} min={0} max={200} step={1} unit="px" digits={0} onchange={(v) => edit((m) => ({ ...m, feather: v }))} /></div>

        <div class="shapes">
          <span class="lab">Shapes</span>
          <div class="add">
            {#each SHAPES as s (s.id)}<Button size="sm" onclick={() => addShape(s.id)} data-testid={`mask-add-${s.id}`}>+ {s.label}</Button>{/each}
          </div>
          {#each d.slide.elements ?? [] as e (e.id)}
            <div class="shape" class:on={e.id === element}>
              <button class="sname" onclick={() => (element = e.id)}>{e.name || shapeOf(e) || "Element"}</button>
              <Button size="sm" variant="ghost" iconOnly icon={Trash} aria-label={`Remove ${e.name ?? "shape"}`} onclick={() => removeShape(e.id)} />
            </div>
          {:else}
            <p class="note">No shapes: add one.</p>
          {/each}
          {#if el}
            {@const id = el.id}
            <div class="geo">
              {#each [["x", "X"], ["y", "Y"], ["width", "W"], ["height", "H"]] as [k, l] (k)}
                <label class="g"><span>{l}</span><Num label={`Shape ${l}`} value={el.bounds?.[k as "x"] ?? 0} step={1} digits={0} width={68} onchange={(v) => edit((m) => withElement(m, id, (x) => setBound(x, k as "x", v)))} /></label>
              {/each}
              {#if mode === "overlay"}
                <label class="g"><span>Colour</span><ColorWell label="Shape colour" value={colorToHex(fillOf(el))} onchange={(v) => edit((m) => withElement(m, id, (x) => ({ ...x, fill: { enabled: true, kind: { type: "color", color: hexToColor(v) } } })))} /></label>
              {/if}
            </div>
          {/if}
        </div>
        {#if error}<p class="err" role="alert">{error}</p>{/if}
      </section>
    {:else}
      <section class="stage empty"><p class="note">Add a mask to start.</p></section>
    {/if}
  </div>
  {#snippet footer()}
    {#if draft}
      <Button variant="ghost" icon={Copy} onclick={duplicate} disabled={busy}>Duplicate</Button>
      <Button variant="ghost" icon={Trash} onclick={() => remove(false)} disabled={busy} data-testid="mask-delete">Delete</Button>
    {/if}
    <span class="grow"></span>
    {#if dirty && entry}<Button variant="ghost" onclick={() => select(entry.id)} disabled={busy}>Revert</Button>{/if}
    <Button variant="primary" onclick={save} disabled={busy || !dirty || !draft?.name?.trim()} data-testid="mask-save">Save</Button>
    <Button onclick={onclose}>Close</Button>
  {/snippet}
</Dialog>

{#if askDelete}
  <Dialog title="Delete mask" width={420} onclose={() => (askDelete = null)}>
    <p data-testid="mask-delete-users">{maskUsersText(askDelete)}</p>
    {#snippet footer()}
      <Button onclick={() => (askDelete = null)}>Cancel</Button>
      <Button variant="danger" onclick={() => remove(true)} data-testid="mask-delete-ok">Delete</Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .me {
    display: grid;
    grid-template-columns: 200px minmax(0, 1fr) 280px;
    min-height: 420px;
    max-height: min(70vh, 640px);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px;
    border-right: 1px solid var(--line);
    overflow-y: auto;
  }
  .lhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 2px 2px 6px;
  }
  .b {
    font-weight: 600;
  }
  .item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
    padding: 7px 8px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text);
    text-align: left;
  }
  .item:hover {
    background: var(--surface-raised);
  }
  .item.on {
    background: var(--accent-sel);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--text-meta);
    font-size: 11px;
  }
  .stage {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    min-width: 0;
  }
  .stage.empty {
    grid-column: span 2;
    align-items: center;
    justify-content: center;
  }
  .preview {
    width: 100%;
    height: auto;
    border: 1px solid var(--line-strong);
    border-radius: var(--r-xs);
  }
  .sel {
    stroke: var(--accent-ring);
    stroke-width: 6;
    stroke-dasharray: 18 10;
  }
  .props {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    border-left: 1px solid var(--line);
    overflow-y: auto;
  }
  .field {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .field > span,
  .lab,
  .g > span {
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .field input {
    flex: 1;
    min-width: 0;
    max-width: 170px;
    padding: 4px 6px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r-xs);
    background: var(--surface-input, transparent);
    color: var(--text);
  }
  .shapes {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 8px;
    border-top: 1px solid var(--line);
  }
  .add {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .shape {
    display: flex;
    align-items: center;
    border-radius: var(--r-xs);
  }
  .shape.on {
    background: var(--accent-sel);
  }
  .sname {
    flex: 1;
    padding: 5px 6px;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
  }
  .geo {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 6px;
  }
  .g {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .note {
    margin: 0;
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .err {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-sm);
  }
  .grow {
    flex: 1;
  }
</style>
