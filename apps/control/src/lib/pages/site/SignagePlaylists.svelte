<script lang="ts">
  // Signage playlists: the list (new, rename, delete) and the selected one's items in order, each
  // with what it shows, its duration and its transition; items move up and down, go, and are
  // added from the library's presentations (a slide), the media bins' images and videos, an
  // announcement (title and text) or a feed's rows. Saved whole with "Save playlist".
  import { untrack } from "svelte";
  import type { SignageContent, SignageFeed, SignageItem, SignagePlaylist, Transition } from "$protocol";
  import { Button, Select } from "../../ui";
  import { signagePage } from "../../state/signage.svelte";
  import { shell } from "../../state/shell.svelte";
  import { contentKind, contentLabel, playlistLength, type Names } from "./signage";

  let { playlists, feeds }: { playlists: SignagePlaylist[]; feeds: SignageFeed[] } = $props();

  let selId = $state<string | null>(null);
  let sel = $derived(playlists.find((p) => p.id === selId) ?? playlists[0] ?? null);
  let draft = $state<SignagePlaylist | null>(null);
  let dirty = $state(false);
  $effect(() => {
    // A fresh copy when another playlist is picked or the saved one changed (not while editing).
    const s = sel;
    untrack(() => {
      if (!s) draft = null;
      else if (!draft || draft.id !== s.id || !dirty) {
        draft = JSON.parse(JSON.stringify(s)) as SignagePlaylist;
        dirty = false;
      }
    });
  });
  let newName = $state("");

  /** Picks another playlist: the one already picked keeps its edits; unsaved edits of the
   * current one go only when the admin says so (H10-7). */
  async function pick(p: SignagePlaylist) {
    if (sel?.id === p.id) return;
    if (dirty && draft && !(await shell.confirm("Unsaved changes", `Discard the changes to “${draft.name}”?`, "Discard"))) return;
    selId = p.id;
    draft = JSON.parse(JSON.stringify(p)) as SignagePlaylist;
    dirty = false;
  }

  const names: Names = {
    presentation: (id) => signagePage.presentations.find((p) => p.id === id)?.name,
    media: (id) => signagePage.media.find((m) => m.asset === id)?.name,
    get feeds() {
      return feeds;
    },
  };
  const TRANSITIONS: { value: string; label: string; t: Transition }[] = [
    { value: "dissolve", label: "Dissolve 1 s", t: { kind: { type: "dissolve" }, duration: 1 } },
    { value: "dissolve2", label: "Dissolve 2 s", t: { kind: { type: "dissolve" }, duration: 2 } },
    { value: "cut", label: "Cut", t: { kind: { type: "cut" } } },
    { value: "push", label: "Push", t: { kind: { type: "push", direction: "left" }, duration: 0.8 } },
  ];
  function transValue(t: Transition | undefined): string {
    if (!t) return "dissolve";
    const k = t.kind?.type ?? "cut";
    if (k === "dissolve") return (t.duration ?? 0) >= 1.5 ? "dissolve2" : "dissolve";
    return k === "push" ? "push" : "cut";
  }
  function change(fn: (items: SignageItem[]) => SignageItem[]) {
    if (!draft) return;
    draft = { ...draft, items: fn(draft.items) };
    dirty = true;
  }
  function moveItem(i: number, by: number) {
    change((items) => {
      const j = i + by;
      if (j < 0 || j >= items.length) return items;
      const n = [...items];
      [n[i], n[j]] = [n[j]!, n[i]!];
      return n;
    });
  }
  async function save() {
    if (!draft) return;
    const r = await signagePage.putPlaylist(draft);
    if (r) dirty = false;
  }
  async function create() {
    if (!newName.trim()) return;
    const p = await signagePage.createPlaylist(newName.trim());
    if (p) {
      newName = "";
      selId = p.id;
      dirty = false;
      draft = null;
    }
  }

  // Adding an item.
  let addKind = $state<SignageContent["kind"]>("text");
  let addPres = $state("");
  let addSlide = $state(1);
  let addMedia = $state("");
  let addTitle = $state("");
  let addBody = $state("");
  let addFeed = $state("");
  /** Another kind starts its media afresh: an image picked for "Image" never becomes a video
   * (H10-6). */
  function pickKind(k: SignageContent["kind"]) {
    if (k !== addKind) addMedia = "";
    addKind = k;
  }
  const KINDS = [
    { value: "text", label: "Announcement" },
    { value: "slide", label: "Slide" },
    { value: "image", label: "Image" },
    { value: "video", label: "Video loop" },
    { value: "feed", label: "Feed rows" },
  ];
  let presChoices = $derived(signagePage.presentations.map((p) => ({ value: p.id, label: `${p.name} (${p.slideCount})` })));
  let mediaChoices = $derived(signagePage.media.filter((m) => m.kind === (addKind === "video" ? "video" : "image")).map((m) => ({ value: m.asset, label: m.name })));
  let feedChoices = $derived(feeds.map((f) => ({ value: f.id, label: f.name })));
  function addItem() {
    let content: SignageContent | null = null;
    if (addKind === "text" && addTitle.trim()) content = { kind: "text", title: addTitle.trim(), body: addBody.trim() };
    if (addKind === "slide" && addPres) content = { kind: "slide", presentation: addPres, slide: Math.max(0, addSlide - 1) };
    if ((addKind === "image" || addKind === "video") && mediaChoices.some((m) => m.value === addMedia)) content = { kind: addKind, asset: addMedia };
    if (addKind === "feed" && addFeed) content = { kind: "feed", feed: addFeed, rows: 4 };
    if (!content) return;
    const item: SignageItem = { id: "", content, duration: addKind === "video" ? 30 : 10 };
    change((items) => [...items, item]);
    addTitle = "";
    addBody = "";
  }
</script>

<div class="st-card" data-testid="signage-playlists">
  <div class="st-row">
    <h2>Playlists</h2>
    <span class="st-grow"></span>
    <input type="text" placeholder="New playlist name" bind:value={newName} onkeydown={(e) => e.key === "Enter" && create()} aria-label="New playlist name" />
    <Button size="sm" onclick={create} disabled={!newName.trim()}>New playlist</Button>
  </div>
  {#if !playlists.length}
    <p class="st-empty">No playlist yet: a playlist is the slides, pictures, videos, announcements and feed rows a channel plays in turn.</p>
  {:else}
    <div class="split">
      <ul class="list">
        {#each playlists as p (p.id)}
          <li>
            <button type="button" class:cur={sel?.id === p.id} onclick={() => pick(p)}>
              <span class="nm">{p.name}</span>
              <span class="st-meta">{p.items.length} item{p.items.length === 1 ? "" : "s"} · {playlistLength(p.items)}</span>
            </button>
          </li>
        {/each}
      </ul>
      {#if draft}
        <div class="items" data-testid="signage-items">
          <div class="st-row">
            <input class="pname" type="text" value={draft.name} onchange={(e) => { if (draft && e.currentTarget.value.trim()) { draft = { ...draft, name: e.currentTarget.value.trim() }; dirty = true; } }} aria-label="Playlist name" />
            <span class="st-meta">{playlistLength(draft.items)} a round</span>
            <span class="st-grow"></span>
            <Button size="sm" onclick={() => draft && signagePage.deletePlaylist(draft.id)}>Delete playlist</Button>
            <Button size="sm" variant="primary" onclick={save} disabled={!dirty}>Save playlist</Button>
          </div>
          <div class="st-table-wrap">
            <table class="st">
              <thead><tr><th>#</th><th>Shows</th><th>Seconds</th><th>Comes in with</th><th></th></tr></thead>
              <tbody>
                {#each draft.items as it, i (it.id || `new-${i}`)}
                  <tr data-testid="signage-item">
                    <td class="st-meta">{i + 1}</td>
                    <td><span class="kind">{contentKind(it.content)}</span> {contentLabel(it.content, names)}</td>
                    <td><input class="dur" type="number" min="1" max="3600" value={it.duration} onchange={(e) => { const v = Math.min(3600, Math.max(1, Number(e.currentTarget.value) || 10)); change((items) => items.map((x, j) => (j === i ? { ...x, duration: v } : x))); }} aria-label="Seconds" /></td>
                    <td><Select label="Transition" options={TRANSITIONS.map(({ value, label }) => ({ value, label }))} value={transValue(it.transition)} onchange={(v) => { const t = TRANSITIONS.find((x) => x.value === v)?.t; change((items) => items.map((x, j) => (j === i ? { ...x, transition: t } : x))); }} /></td>
                    <td class="actions">
                      <Button size="sm" variant="ghost" onclick={() => moveItem(i, -1)} disabled={i === 0} aria-label="Move up">↑</Button>
                      <Button size="sm" variant="ghost" onclick={() => moveItem(i, 1)} disabled={i === draft.items.length - 1} aria-label="Move down">↓</Button>
                      <Button size="sm" variant="ghost" onclick={() => change((items) => items.filter((_, j) => j !== i))} aria-label="Remove">✕</Button>
                    </td>
                  </tr>
                {:else}
                  <tr><td colspan="5" class="st-empty">No items yet: add one below.</td></tr>
                {/each}
              </tbody>
            </table>
          </div>
          <div class="add st-row">
            <Select label="Item kind" options={KINDS} value={addKind} onchange={(v) => pickKind(v as SignageContent["kind"])} />
            {#if addKind === "text"}
              <input type="text" placeholder="Title" bind:value={addTitle} aria-label="Title" />
              <input class="body" type="text" placeholder="Text (optional)" bind:value={addBody} aria-label="Text" />
            {:else if addKind === "slide"}
              <Select label="Presentation" options={[{ value: "", label: "Choose a presentation" }, ...presChoices]} bind:value={addPres} />
              <label class="st-meta">Slide <input class="dur" type="number" min="1" bind:value={addSlide} /></label>
            {:else if addKind === "image" || addKind === "video"}
              <Select label="Media" options={[{ value: "", label: mediaChoices.length ? "Choose from the media bins" : "No media in the bins" }, ...mediaChoices]} bind:value={addMedia} />
            {:else}
              <Select label="Feed" options={[{ value: "", label: feedChoices.length ? "Choose a feed" : "No feed yet (Feeds, below)" }, ...feedChoices]} bind:value={addFeed} />
            {/if}
            <Button size="sm" onclick={addItem}>Add item</Button>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .split {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    flex: 0 0 200px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .list button {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 7px 9px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: inherit;
    text-align: left;
    cursor: pointer;
  }
  .list button.cur {
    background: var(--accent-sel);
  }
  .nm {
    font-weight: 600;
  }
  .items {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .pname {
    font-weight: 600;
    width: 220px;
  }
  .dur {
    width: 70px;
  }
  .kind {
    display: inline-block;
    padding: 1px 6px;
    margin-right: 4px;
    border-radius: 4px;
    background: var(--surface-raised);
    color: var(--text-label);
    font-size: 11px;
  }
  .body {
    flex: 1;
    min-width: 200px;
  }
</style>
