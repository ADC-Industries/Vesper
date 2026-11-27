<script lang="ts">
  // The Looks tab of the Screens & Looks page (plan 4.8, docs/LOOKS-MASKS-PLAN.md): the box's
  // looks and "Every layer"; the selected look's layer matrix per audience screen (rows: mask,
  // the layers, theme override; a click toggles), its name and transition, edited as a draft
  // and saved with Save (`PUT /looks/{id}`, If-Match); Make Live (`setLook`; double-click a
  // look does the same); new, duplicate, move up / down and delete. The live look is a saved
  // look: saving it changes the screens at once, with the look's transition.
  import type { LanguageDisplay, Look, LookEntry, MaskEntry, ThemeSummary, Transition } from "$protocol";
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import ChevronUp from "lucide-svelte/icons/chevron-up";
  import Copy from "lucide-svelte/icons/copy";
  import Plus from "lucide-svelte/icons/plus";
  import Trash from "lucide-svelte/icons/trash-2";
  import { untrack } from "svelte";
  import { Button, Chip, Dialog, Menu, Popover, SectionLabel } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { useLive } from "../../live";
  import { ApiRequestError } from "../../server/rest";
  import TransitionGrid from "../editor/TransitionGrid.svelte";
  import ChunkError from "../../components/ChunkError.svelte";
  // The mask editor loads on first use (its own chunk).
  const maskEditorModule = () => import("./MaskEditor.svelte");
  import { LAYER_ROWS, entryOf, isDirty, languageOf, movedKey, reselect, shows, withLanguage, withLayer, withMask, withStored, withTheme } from "./looks";
  import { LANGUAGE_DISPLAYS } from "../../edit/language";

  const live = useLive();
  const activeLook = live.activeLook;
  const screens = live.screens;

  /** The selected look: an ID, or "" for "Every layer". */
  let picked = $state<string | null>(null);
  let selId = $derived(picked ?? $activeLook ?? "");
  let stored = $derived<Look | null>(session.looks.find((l) => l.id === selId) ?? null);
  let audience = $derived($screens.filter((s) => s.role === "audience"));
  let isLive = $derived((selId || null) === $activeLook);

  /** The selected look as stored, with its revision (loaded on selection and after changes). */
  let entry = $state<LookEntry | null>(null);
  /** The edits not saved yet. */
  let draft = $state<Look | null>(null);
  let dirty = $derived(isDirty(draft, entry));
  let busy = $state(false);
  let error = $state("");
  let saved = $state(false);
  let themeFor = $state<string | null>(null);
  let themes = $state.raw<ThemeSummary[]>([]);
  let confirmDelete = $state(false);
  let maskFor = $state<string | null>(null);
  let masks = $state.raw<MaskEntry[]>([]);
  /** The mask editor: open, with the mask to show first. */
  let maskEditor = $state<{ initial: string | null } | null>(null);
  /** The look whose draft is open. */
  let loadedFor: string | null = null;
  /** The stored look changed (another operator) under unsaved edits: Reload or Overwrite (D08-2). */
  let conflict = $state(false);
  /** A selection change waiting for "Discard changes" (D08-3). */
  let pendingSwitch = $state<(() => void) | null>(null);

  function errorText(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  function stripRevision(e: LookEntry): Look {
    const { revision: _r, ...look } = e;
    return look as Look;
  }

  async function loadEntry(id: string, keepDraft: boolean) {
    const rest = session.conn?.rest;
    if (!rest || !id) {
      entry = null;
      draft = null;
      return;
    }
    try {
      const e = await rest.look(id);
      if (selId !== id) return;
      entry = e;
      if (!keepDraft || !draft || draft.id !== id) draft = $state.snapshot(stripRevision(e));
    } catch (e) {
      if (selId === id) error = `Could not load the look: ${errorText(e)}`;
    }
  }

  // A new selection loads its entry; a change of the stored look (another operator, a save)
  // reloads it, keeping unsaved edits.
  $effect(() => {
    const id = selId;
    const s = stored;
    if (id !== loadedFor) {
      loadedFor = id;
      error = "";
      saved = false;
      themeFor = null;
      draft = null;
      entry = null;
      conflict = false;
      void loadEntry(id, false);
    } else if (s && entry && JSON.stringify(s) !== JSON.stringify(stripRevision(entry))) {
      // Unsaved edits keep the revision they started from, so Save can't silently overwrite
      // the other operator's change: the operator picks Reload or Overwrite (D08-2).
      if (untrack(() => dirty)) conflict = true;
      else void loadEntry(id, false);
    }
  });

  function edit(f: (l: Look) => Look) {
    if (!draft) return;
    draft = f(draft);
    saved = false;
    // An edited look stays selected when the live look changes (D08-3).
    if (picked === null) picked = selId;
  }

  /** Runs a selection change now, or after the operator agreed to discard unsaved edits (D08-3). */
  function guarded(action: () => void) {
    if (dirty) pendingSwitch = action;
    else action();
  }

  function select(id: string | null) {
    // The same look stays selected (e.g. the Live box while the edited look is live): no
    // prompt, and a dirty draft stays pinned to it (E08-1).
    const same = reselect(id, $activeLook ?? null, selId, dirty);
    if (same !== undefined) picked = same;
    else guarded(() => (picked = id));
  }

  /** "Discard changes": the draft goes back to the stored look, then the selection changes. */
  function discardAndRun() {
    const action = pendingSwitch;
    pendingSwitch = null;
    conflict = false;
    error = "";
    if (entry) draft = $state.snapshot(stripRevision(entry));
    action?.();
  }

  /** Reload: drop the unsaved edits and load the stored look. */
  function reloadAfterConflict() {
    conflict = false;
    error = "";
    draft = null;
    if (selId) void loadEntry(selId, false);
  }

  /** Overwrite: save the edits over the other operator's version (its current revision). */
  function overwrite() {
    const rest = session.conn?.rest;
    const id = selId;
    if (!rest || !id) return;
    void act("save the look", async () => {
      const latest = await rest.look(id);
      const d = draft;
      if (selId !== id || !d) return;
      entry = latest;
      conflict = false;
      await saveDraft(rest, d, latest);
    });
  }

  function makeLive(id: string) {
    void session.run({ type: "setLook", look: id || null });
  }

  async function refreshLooks() {
    const v = await session.conn?.rest.looks().catch(() => null);
    if (v) session.looks = v;
  }

  async function act(what: string, run: () => Promise<void>) {
    if (busy) return;
    busy = true;
    error = "";
    try {
      await run();
    } catch (e) {
      if (e instanceof ApiRequestError && e.status === 409) {
        // Keep the draft and its old revision: the banner offers Reload or Overwrite (D08-2).
        if (draft && dirty) conflict = true;
        else {
          error = errorText(e);
          if (selId) await loadEntry(selId, false);
        }
      } else error = `Could not ${what}: ${errorText(e)}`;
    } finally {
      busy = false;
    }
  }

  function save() {
    const rest = session.conn?.rest;
    const d = draft;
    const e0 = entry;
    if (!rest || !d || !e0) return;
    void act("save the look", () => saveDraft(rest, d, e0));
  }

  async function saveDraft(rest: NonNullable<typeof session.conn>["rest"], d: Look, e0: LookEntry) {
    const e = await rest.updateLook({ ...d, name: (d.name ?? "").trim() }, e0.revision);
    entry = e;
    draft = $state.snapshot(stripRevision(e));
    saved = true;
    conflict = false;
    await refreshLooks();
  }

  function revert() {
    if (conflict) return reloadAfterConflict();
    if (entry) draft = $state.snapshot(stripRevision(entry));
    error = "";
  }

  function create() {
    const rest = session.conn?.rest;
    if (!rest) return;
    if (dirty) return guarded(create_);
    create_();
  }

  function create_() {
    const rest = session.conn?.rest;
    if (!rest) return;
    void act("add a look", async () => {
      const e = await rest.createLook({});
      await refreshLooks();
      picked = e.id;
    });
  }

  function duplicate() {
    if (dirty) return guarded(duplicate_);
    duplicate_();
  }

  function duplicate_() {
    const rest = session.conn?.rest;
    const id = selId;
    if (!rest || !id) return;
    void act("duplicate the look", async () => {
      const e = await rest.duplicateLook(id);
      await refreshLooks();
      picked = e.id;
    });
  }

  function move(dir: -1 | 1) {
    const rest = session.conn?.rest;
    const key = selId ? movedKey(session.looks, selId, dir) : null;
    const e0 = entry;
    if (!rest || !e0 || key === null) return;
    void act("move the look", async () => {
      const e = await rest.updateLook({ ...stripRevision(e0), sortKey: key }, e0.revision);
      // The stored list gets this client's own write with the entry, so the selection effect
      // doesn't take the new sort key for another operator's change (E08-2).
      session.looks = withStored(session.looks, stripRevision(e));
      entry = e;
      if (draft) draft = { ...draft, sortKey: key };
      await refreshLooks();
    });
  }

  function remove() {
    const rest = session.conn?.rest;
    const e0 = entry;
    confirmDelete = false;
    if (!rest || !e0) return;
    void act("delete the look", async () => {
      await rest.deleteLook(e0.id, e0.revision);
      picked = null;
      await refreshLooks();
    });
  }

  async function openTheme(screen: string) {
    themeFor = themeFor === screen ? null : screen;
    if (themeFor && !themes.length) themes = (await session.conn?.rest.themes().catch(() => null)) ?? [];
  }

  function themeLabel(screen: string): string | null {
    const t = entryOf(view, screen)?.templateOverride;
    if (!t) return null;
    const th = themes.find((x) => x.id === t.theme.id);
    const slide = th?.slides.find((s) => s.id === t.slide)?.name;
    return slide ? `${th?.name ?? t.theme.name ?? "Theme"} · ${slide}` : (t.theme.name ?? "Theme");
  }

  function maskLabel(screen: string): string | null {
    const id = entryOf(view, screen)?.mask;
    if (!id) return null;
    return masks.find((m) => m.id === id)?.name || "Mask";
  }

  function pickMask(screen: string, id: string) {
    maskFor = null;
    if (id === "edit") maskEditor = { initial: entryOf(view, screen)?.mask ?? null };
    else if (id === "new") {
      const rest = session.conn?.rest;
      if (!rest) return;
      void act("add a mask", async () => {
        const m = await rest.createMask({});
        masks = await rest.masks();
        edit((l) => withMask(l, screen, m.id));
        maskEditor = { initial: m.id };
      });
    } else edit((l) => withMask(l, screen, id === "none" ? null : id));
  }

  // The box's masks (for names and the picker).
  $effect(() => {
    const rest = session.conn?.rest;
    if (!rest) return;
    void rest
      .masks()
      .then((m) => (masks = m))
      .catch(() => {});
  });

  function setTransition(t: Transition | null) {
    edit((l) => {
      const n = { ...l };
      if (t && t.kind?.type !== "cut") n.transition = t;
      else delete n.transition;
      return n;
    });
  }

  let index = $derived(session.looks.findIndex((l) => l.id === selId));
  let view = $derived<Look | null>(draft ?? stored);
  let editable = $derived(!!draft && !!selId);

  // Theme names for the cells (loaded once, in the background).
  $effect(() => {
    if (!themes.length && session.conn && session.looks.some((l) => l.screens?.some((s) => s.templateOverride))) {
      void session.conn.rest
        .themes()
        .then((t) => (themes = t))
        .catch(() => {});
    }
  });
</script>

<div class="looks" data-testid="looks-view">
  <aside class="list" aria-label="Looks">
    <button class="item livebox" class:on={selId === ($activeLook ?? "")} onclick={() => select(null)}>
      <span class="b">Live</span>
      <span class="meta">{$activeLook === null ? "Every layer" : (session.looks.find((l) => l.id === $activeLook)?.name ?? "Look")}</span>
    </button>
    <SectionLabel>
      Presets
      {#snippet actions()}<Button size="sm" variant="ghost" icon={Plus} onclick={create} disabled={busy} data-testid="look-new">New look</Button>{/snippet}
    </SectionLabel>
    <div class="items">
      {#each session.looks as l (l.id)}
        <button class="item" class:on={selId === l.id} onclick={() => select(l.id)} ondblclick={() => makeLive(l.id)} title="Double-click to make live" data-testid="look-item">
          <span class="name">{l.name || "Unnamed look"}</span>
          {#if $activeLook === l.id}<Chip kind="live">LIVE</Chip>{/if}
        </button>
      {/each}
      <button class="item" class:on={selId === ""} onclick={() => select("")} ondblclick={() => makeLive("")} title="Every layer on every screen. Double-click to make live">
        <span class="name">Every layer</span>
        {#if $activeLook === null}<Chip kind="live">LIVE</Chip>{/if}
      </button>
    </div>
  </aside>

  <main class="main">
    <div class="head">
      {#if editable && draft}
        <input
          class="title"
          aria-label="Look name"
          value={draft.name ?? ""}
          oninput={(e) => {
            const name = e.currentTarget.value;
            edit((l) => ({ ...l, name }));
          }}
          maxlength={64}
          data-testid="look-name"
        />
      {:else}
        <h2 class="title">{view ? view.name || "Unnamed look" : "Every layer"}</h2>
      {/if}
      {#if selId}
        <span class="tools">
          <Button size="sm" variant="ghost" iconOnly icon={ChevronUp} aria-label="Move up" title="Move up" disabled={busy || index <= 0} onclick={() => move(-1)} />
          <Button size="sm" variant="ghost" iconOnly icon={ChevronDown} aria-label="Move down" title="Move down" disabled={busy || index < 0 || index >= session.looks.length - 1} onclick={() => move(1)} />
          <Button size="sm" variant="ghost" iconOnly icon={Copy} aria-label="Duplicate" title="Duplicate" disabled={busy} onclick={duplicate} data-testid="look-duplicate" />
          <Button
            size="sm"
            variant="ghost"
            iconOnly
            icon={Trash}
            aria-label="Delete"
            title={isLive ? "The live look can't be deleted: make another look live first" : "Delete"}
            disabled={busy || isLive}
            onclick={() => (confirmDelete = true)}
            data-testid="look-delete"
          />
        </span>
      {/if}
      {#if isLive}
        <Chip kind="live">LIVE</Chip>
      {:else}
        <Button variant="live" onclick={() => makeLive(selId)} data-testid="make-live">Make Live</Button>
      {/if}
      {#if editable}
        {#if dirty}<Button variant="ghost" onclick={revert} disabled={busy} data-testid="look-revert">Revert</Button>{/if}
        <Button variant="primary" onclick={save} disabled={busy || !dirty || !draft?.name?.trim()} data-testid="look-save">{saved && !dirty ? "Saved" : "Save"}</Button>
      {/if}
    </div>
    {#if conflict}
      <div class="err conflict" role="alert" data-testid="look-conflict">
        <span>Changed by another operator since you started editing this look.</span>
        <Button size="sm" onclick={reloadAfterConflict} title="Discard your changes and load theirs" disabled={busy}>Reload</Button>
        <Button size="sm" variant="danger" onclick={overwrite} title="Replace their version with yours" disabled={busy}>Overwrite</Button>
      </div>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    {#if isLive && dirty}<p class="note">This look is live: saving changes the screens at once, with its transition.</p>{/if}
    {#if audience.length}
      <div class="matrix" style:--cols={audience.length} role="table" aria-label="Layers per screen">
        <span role="columnheader"></span>
        {#each audience as s (s.id)}<span class="col" role="columnheader">{s.name}</span>{/each}
        <span class="row" role="rowheader">Mask</span>
        {#each audience as s (s.id)}
          {@const m = maskLabel(s.id)}
          {@const cur = entryOf(view, s.id)?.mask ?? null}
          <span class="cell anchor">
            <button class="pickcell" class:set={!!m} disabled={!editable || busy} aria-expanded={maskFor === s.id} onclick={() => (maskFor = maskFor === s.id ? null : s.id)} title={m ?? "No mask"} data-testid="look-mask">
              {m ?? "None"}
            </button>
            {#if maskFor === s.id}
              <Popover role="none" width={240} onclose={() => (maskFor = null)}>
                <Menu
                  label={`Mask on ${s.name}`}
                  items={[
                    { id: "none", label: "None", checked: !cur },
                    ...masks.map((x) => ({ id: x.id, label: x.name || "Unnamed mask", checked: cur === x.id })),
                    { id: "sep", separator: true },
                    { id: "edit", label: "Edit masks…" },
                    { id: "new", label: "New mask" },
                  ]}
                  onselect={(id) => pickMask(s.id, id)}
                  onclose={() => (maskFor = null)}
                />
              </Popover>
            {/if}
          </span>
        {/each}
        {#each LAYER_ROWS as r (r.key)}
          <span class="row" role="rowheader">{r.label}{#if r.colour}<span class="ldot" style:border-color={r.colour}></span>{/if}</span>
          {#each audience as s (s.id)}
            <span class="cell">
              <input
                type="checkbox"
                checked={shows(view, s.id, r.key)}
                disabled={!editable || busy}
                onchange={(e) => {
                  const on = e.currentTarget.checked;
                  edit((l) => withLayer(l, s.id, r.key, on));
                }}
                aria-label={`${r.label} on ${s.name}`}
                data-testid="look-cell"
              />
            </span>
          {/each}
        {/each}
        <span class="row" role="rowheader">Theme override<span class="ldot" style:border-color="var(--accent)"></span></span>
        {#each audience as s (s.id)}
          {@const t = themeLabel(s.id)}
          <span class="cell anchor up">
            <button class="pickcell" class:set={!!t} disabled={!editable || busy} aria-expanded={themeFor === s.id} onclick={() => openTheme(s.id)} title={t ?? "No theme override"} data-testid="look-theme">
              {t ?? "None"}
            </button>
            {#if themeFor === s.id}
              <Popover label={`Theme override on ${s.name}`} width={340} onclose={() => (themeFor = null)}>
                <div class="themes" data-testid="look-theme-menu">
                  <p class="phead">Theme override · {s.name}</p>
                  {#each themes as th (th.id)}
                    <p class="tname">{th.name}</p>
                    <div class="tslides">
                      {#each th.slides as ts (ts.id)}
                        <button
                          class="tslide"
                          class:on={entryOf(draft, s.id)?.templateOverride?.slide === ts.id}
                          onclick={() => {
                            edit((l) => withTheme(l, s.id, { theme: { id: th.id, name: th.name }, slide: ts.id }));
                            themeFor = null;
                          }}
                        >
                          <img src={session.conn?.rest.themeThumbnailUrl(th.id, ts.id, 160) ?? ""} alt="" loading="lazy" />
                          <span>{ts.name || "Template"}</span>
                        </button>
                      {/each}
                    </div>
                  {:else}
                    <p class="note">No themes on this box.</p>
                  {/each}
                  <span>
                    <Button
                      size="sm"
                      onclick={() => {
                        edit((l) => withTheme(l, s.id, null));
                        themeFor = null;
                      }}>No theme override</Button
                    >
                  </span>
                </div>
              </Popover>
            {/if}
          </span>
        {/each}
        <span class="row" role="rowheader" title="How slides with a second language are shown">Second language</span>
        {#each audience as s (s.id)}
          {@const lang = languageOf(view, s.id)}
          <span class="cell">
            <select
              class="langsel"
              class:set={lang !== "auto"}
              value={lang}
              disabled={!editable || busy}
              title={LANGUAGE_DISPLAYS.find((d) => d.value === lang)?.hint}
              onchange={(e) => {
                const v = e.currentTarget.value as LanguageDisplay;
                edit((l) => withLanguage(l, s.id, v));
              }}
              aria-label={`Second language on ${s.name}`}
              data-testid="look-language"
            >
              {#each LANGUAGE_DISPLAYS as d (d.value)}<option value={d.value}>{d.label}</option>{/each}
            </select>
          </span>
        {/each}
      </div>
    {:else}
      <p class="note">No audience screen is running.</p>
    {/if}
    <div class="foot">
      <div class="trans">
        <span class="lab">Transition when it goes live</span>
        {#if editable && draft}
          <TransitionGrid transition={draft.transition ?? undefined} owner={draft.id} defaultLabel="None" label="Look transition" onchange={setTransition} />
        {:else}
          <span class="meta">{view?.transition ? "the look's own" : "a cut"}</span>
        {/if}
      </div>
      <span class="meta"
        >Looks only change audience screens; stage screens use stage layouts. <button class="link" onclick={() => (maskEditor = { initial: null })} data-testid="edit-masks">Edit masks…</button></span
      >
    </div>
  </main>
</div>

{#if maskEditor}
  {#await maskEditorModule() then M}
    <M.default
      initial={maskEditor.initial}
      onclose={() => (maskEditor = null)}
      onchanged={(m: MaskEntry[]) => {
        masks = m;
        void refreshLooks();
      }}
    />
  {:catch e}
    <Dialog title="Mask editor" width={460} onclose={() => (maskEditor = null)}><ChunkError error={e} what="The mask editor" /></Dialog>
  {/await}
{/if}

{#if pendingSwitch}
  <Dialog title="Unsaved changes" width={420} onclose={() => (pendingSwitch = null)}>
    <p>“{draft?.name || "This look"}” has changes that are not saved yet.</p>
    {#snippet footer()}
      <Button onclick={() => (pendingSwitch = null)}>Keep editing</Button>
      <Button variant="danger" onclick={discardAndRun} data-testid="look-discard">Discard changes</Button>
    {/snippet}
  </Dialog>
{/if}

{#if confirmDelete && entry}
  <Dialog title="Delete look" width={420} onclose={() => (confirmDelete = false)}>
    <p>Delete “{entry.name || "Unnamed look"}”? Slide actions that make it live are skipped from now on. This cannot be undone.</p>
    {#snippet footer()}
      <Button onclick={() => (confirmDelete = false)}>Cancel</Button>
      <Button variant="danger" onclick={remove} data-testid="look-delete-ok">Delete</Button>
    {/snippet}
  </Dialog>
{/if}

<style>
  .looks {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-wrap: wrap;
    overflow: auto;
  }
  .list {
    flex: 0 0 240px;
    max-width: 100%;
    max-height: 100%;
    box-sizing: border-box;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px;
    background: var(--surface-panel);
    border-right: 1px solid var(--line);
  }
  .items {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid transparent;
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
  .livebox {
    border-color: var(--line-strong);
    margin-bottom: 4px;
  }
  .livebox.on {
    border-color: var(--accent-ring);
  }
  .b {
    font-weight: 600;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--text-2);
  }
  .main {
    flex: 999 1 520px;
    min-width: 0;
    max-height: 100%;
    box-sizing: border-box;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 12px 16px;
  }
  .head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 10px;
    padding-bottom: 10px;
    border-bottom: 1px solid var(--line);
  }
  .title {
    flex: 1 1 200px;
    min-width: 0;
    margin: 0;
    font-size: var(--fs-xl);
    font-weight: 800;
  }
  input.title {
    background: transparent;
    border: 1px solid transparent;
    border-radius: var(--r);
    padding: 2px 4px;
    color: var(--text);
    font-family: inherit;
  }
  input.title:hover,
  input.title:focus {
    border-color: var(--line-strong);
  }
  .tools {
    display: flex;
    gap: 2px;
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
  .conflict {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .matrix {
    display: grid;
    grid-template-columns: 190px repeat(var(--cols), minmax(96px, 120px));
    gap: 6px 10px;
    align-items: center;
    justify-content: start;
  }
  .col {
    text-align: center;
    color: var(--text-2);
    font-size: 12.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    height: 40px;
  }
  .ldot {
    width: 12px;
    height: 12px;
    box-sizing: border-box;
    border-radius: 50%;
    border: 3px solid;
  }
  .cell {
    display: flex;
    justify-content: center;
  }
  .anchor {
    position: relative;
  }
  /* The theme row is the matrix's last: its picker opens upward. */
  .anchor.up :global(.ui-popover) {
    top: auto;
    bottom: calc(100% + 6px);
  }
  .cell input {
    width: 18px;
    height: 18px;
    accent-color: var(--accent);
  }
  .langsel {
    width: 76px;
    padding: 2px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r-xs);
    background: transparent;
    color: var(--text-label);
    font-size: 10.5px;
  }
  .langsel.set {
    border-color: var(--accent);
    color: var(--text);
  }
  .pickcell {
    width: 76px;
    height: 40px;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2px;
    overflow: hidden;
    border: 1px solid var(--line-strong);
    border-radius: var(--r-xs);
    background: transparent;
    color: var(--text-label);
    font-size: 10.5px;
    line-height: 1.2;
    text-align: center;
  }
  button.pickcell:not(:disabled):hover {
    border-color: var(--accent-ring);
  }
  .pickcell.set {
    color: var(--text);
    background: var(--accent-sel);
  }
  .themes {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    max-height: min(360px, 50vh);
    overflow-y: auto;
  }
  .phead {
    margin: 0;
    color: var(--text-label);
    font-size: 12px;
  }
  .tname {
    margin: 4px 0 0;
    font-weight: 600;
  }
  .tslides {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 6px;
  }
  .tslide {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 3px;
    border: 1px solid transparent;
    border-radius: var(--r-xs);
    background: transparent;
    color: var(--text);
    font-size: 11px;
    text-align: center;
    min-width: 0;
  }
  .tslide:hover {
    background: var(--surface-raised);
  }
  .tslide.on {
    border-color: var(--accent-ring);
  }
  .tslide img {
    width: 100%;
    aspect-ratio: 16 / 9;
    object-fit: cover;
    background: #000;
    border-radius: 2px;
  }
  .tslide span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .foot {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
    color: var(--text-2);
    font-size: var(--fs-sm);
  }
  .trans {
    display: flex;
    flex-direction: column;
    gap: 6px;
    max-width: 640px;
  }
  .lab {
    color: var(--text-label);
  }
  .foot > .meta {
    color: var(--text-meta);
  }
  .link {
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--accent-text, var(--accent));
    text-decoration: underline;
    font-size: inherit;
  }
</style>
