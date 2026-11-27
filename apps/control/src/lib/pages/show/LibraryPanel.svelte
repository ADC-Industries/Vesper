<script lang="ts">
  // The Show page's left column: the sources (the collection's libraries, or shelves, and the
  // playlist tree) over the contents of whichever is picked. Playlists and folders drag to
  // reorder or move (spring-loaded folders, a "move to the top level" zone); presentations drag
  // onto playlists or into the open playlist; playlist items drag to reorder. Double-click
  // renames inline; headers carry their colour and a ✕.
  import Plus from "lucide-svelte/icons/plus";
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import ChevronRight from "lucide-svelte/icons/chevron-right";
  import LibraryIcon from "lucide-svelte/icons/library";
  import ListMusic from "lucide-svelte/icons/list-music";
  import Folder from "lucide-svelte/icons/folder";
  import FolderOpen from "lucide-svelte/icons/folder-open";
  import FolderPlus from "lucide-svelte/icons/folder-plus";
  import FileText from "lucide-svelte/icons/file-text";
  import Film from "lucide-svelte/icons/film";
  import Play from "lucide-svelte/icons/play";
  import Search from "lucide-svelte/icons/search";
  import X from "lucide-svelte/icons/x";
  import type { PlaylistItemDetail, PlaylistTreeNode, PresentationSummary } from "$protocol";
  import { session, type Renaming } from "../../state/session.svelte";
  import { libraryState } from "../../state/library.svelte";
  import { siteState } from "../../state/site.svelte";
  import { shell } from "../../state/shell.svelte";
  import { showView } from "../../state/show.svelte";
  import { useLive } from "../../live";
  import { cssColor } from "../../model";
  import type { MenuItem } from "../../shell/menu";
  import { drag, type DragPayload, type DropHandlers } from "../../dnd.svelte";
  import Chip from "../../ui/Chip.svelte";
  import Splitter from "../../ui/Splitter.svelte";
  import { withoutAudioPlaylists } from "./audio";
  import { KIND_DOT, nodePlace, summaryMeta, treeMove } from "./sections";
  import { moveNode } from "./slideActions";

  const live = useLive();
  const livePlaylistItem = live.livePlaylistItem;
  const liveSlide = live.slide;

  /** Audio playlists live in the right column's audio panel. */
  let serviceTree = $derived(withoutAudioPlaylists(session.playlistTree));
  let collection = $derived(siteState.codex?.collectionName || "Library");

  let q = $derived(libraryState.search.trim().toLowerCase());
  let presentations = $derived(q ? libraryState.libraryItems.filter((p) => p.name.toLowerCase().includes(q)) : libraryState.libraryItems);
  let playlistItems = $derived.by(() => {
    const items = session.playlist?.items ?? [];
    return q ? items.filter((i) => i.kind.type === "header" || i.displayName.toLowerCase().includes(q)) : items;
  });
  let fromLibrary = $derived(showView.listSource === "library");
  let srcName = $derived(
    fromLibrary ? (libraryState.libraries.find((l) => l.id === libraryState.libraryId)?.name ?? "Library") : (session.playlist?.name ?? "Playlist"),
  );
  let srcCount = $derived.by(() => {
    if (fromLibrary) return q ? `${presentations.length} of ${libraryState.libraryItems.length}` : `${libraryState.libraryItems.length} items`;
    const n = (session.playlist?.items ?? []).filter((i) => i.kind.type !== "header").length;
    return `${n} ${n === 1 ? "item" : "items"}`;
  });

  function countPlaylists(n: PlaylistTreeNode): number {
    return n.children.reduce((c, x) => c + (x.kind === "folder" ? countPlaylists(x) : 1), 0);
  }

  /** The kind's dot for a playlist item (the summary's song, scripture, text or media). */
  function itemDot(it: PlaylistItemDetail): string {
    if (it.kind.type !== "presentation") return KIND_DOT.media;
    return KIND_DOT[it.presentation?.kind ?? "text"];
  }

  function scrollToSection(itemId: string) {
    // After the centre switched to the playlist (when it showed a library item).
    setTimeout(() => document.querySelector(`[data-section="item:${CSS.escape(itemId)}"]`)?.scrollIntoView({ block: "start", behavior: "smooth" }));
  }

  function clickItem(it: PlaylistItemDetail) {
    if (drag.consumeClick()) return;
    const pl = session.playlist;
    if (!pl) return;
    showView.listSource = "playlist";
    showView.selectedItemId = it.id;
    showView.selectedPresentationId = null;
    if (it.kind.type === "presentation" && it.presentation) void session.openPresentation(it.presentation.id, { playlist: pl.id, item: it.id });
    else if (it.kind.type !== "header") session.open = null;
    scrollToSection(it.id);
  }

  function goLive(it: PlaylistItemDetail) {
    const pl = session.playlist;
    if (pl) void session.run({ type: "triggerPlaylistItem", playlist: pl.id, item: it.id });
  }

  function pickPresentation(p: PresentationSummary) {
    if (drag.consumeClick()) return;
    showView.listSource = "library";
    void session.openPresentation(p.id);
  }

  /** Double-click a library item: add it to the open playlist (as drawn). */
  function addPresentation(p: PresentationSummary) {
    const pl = session.playlist;
    if (!pl) {
      session.conn?.pushNotice("info", "Pick a playlist first, then double-click a song to add it.", "addNoPlaylist");
      return;
    }
    void session.addToPlaylist(pl.id, p).then(() => session.selectPlaylist(pl.id, true));
  }

  function onFilterKey(e: KeyboardEvent) {
    if (e.key === "Escape" && libraryState.search) {
      libraryState.search = "";
      e.stopPropagation();
    } else if (e.key === "Enter" && fromLibrary && presentations[0]) {
      void session.openPresentation(presentations[0].id);
    }
  }

  // ---- context menus -------------------------------------------------------------------------

  function item(id: string, label: string, run?: () => void, opts: Partial<Extract<MenuItem, { type: "item" }>> = {}): MenuItem {
    return { type: "item", id, label, run, ...opts };
  }
  const sep: MenuItem = { type: "separator" };

  /** Every playlist (not folders), with its folder path, for "Add to Playlist ▸". */
  function allPlaylists(nodes: PlaylistTreeNode[], path = ""): { id: string; label: string }[] {
    return nodes.flatMap((n) =>
      n.kind === "folder" ? allPlaylists(n.children, `${path}${n.name} / `) : [{ id: n.id, label: `${path}${n.name}` }],
    );
  }

  function addToPlaylistMenu(p: { id: string; name: string }): MenuItem {
    const pls = allPlaylists(serviceTree);
    return item("add-to-playlist", "Add to Playlist", undefined, {
      disabled: !pls.length,
      submenu: pls.map((pl) => item(`pl-${pl.id}`, pl.label, () => void session.addToPlaylist(pl.id, p))),
    });
  }

  function openMenu(e: MouseEvent, items: MenuItem[]) {
    e.preventDefault();
    e.stopPropagation();
    shell.popover = null;
    shell.contextMenu = { x: e.clientX, y: e.clientY, items };
  }

  function rename(kind: Renaming["kind"], id: string) {
    session.renaming = { kind, id };
  }

  function libraryMenu(e: MouseEvent, l: { id: string; name: string }) {
    void libraryState.selectLibrary(l.id);
    openMenu(e, [
      item("rename", "Rename", () => rename("library", l.id)),
      item("delete", "Delete…", () => void session.deleteLibrary(l.id)),
      sep,
      item("new-pres", "New Presentation…", () => void session.newPresentation()),
      item("new-lib", "New Library", () => void session.newLibrary()),
    ]);
  }

  function presentationMenu(e: MouseEvent, p: PresentationSummary) {
    showView.selectedPresentationId = p.id;
    const libs = libraryState.libraries.filter((l) => l.id !== p.library);
    openMenu(e, [
      item("open-reflow", "Open in Reflow", () => {
        void session.openPresentation(p.id).then(() => shell.setMode("reflow"));
      }),
      sep,
      item("rename", "Rename", () => rename("presentation", p.id)),
      item("duplicate", "Duplicate", () => void session.duplicatePresentation(p.id)),
      item("delete", "Delete…", () => void session.deletePresentation(p.id, p.name)),
      sep,
      addToPlaylistMenu(p),
      item("move-lib", "Move to Library", undefined, {
        disabled: !libs.length,
        submenu: libs.map((l) => item(`lib-${l.id}`, l.name, () => void session.movePresentation(p.id, p.library ?? null, l.id))),
      }),
    ]);
  }

  function playlistMenu(e: MouseEvent, n: PlaylistTreeNode) {
    const folder = n.kind === "folder";
    if (!folder) void session.selectPlaylist(n.id);
    openMenu(e, [
      item("rename", "Rename", () => rename("playlist", n.id)),
      ...(folder ? [] : [item("duplicate", "Duplicate", () => void session.duplicatePlaylist(n.id))]),
      item("delete", "Delete…", () => void session.deletePlaylist(n.id, n.name, folder)),
      sep,
      item("new-playlist", "New Playlist", () => void session.newPlaylist(false, folder ? n.id : null)),
      item("new-folder", "New Folder", () => void session.newPlaylist(true, folder ? n.id : null)),
      ...(folder ? [] : [sep, item("add-header", "Add Header…", () => void session.addHeader(n.id))]),
    ]);
  }

  function itemMenu(e: MouseEvent, it: PlaylistItemDetail) {
    const pl = session.playlist;
    if (!pl) return;
    showView.selectedItemId = it.id;
    const pres = it.kind.type === "presentation" && it.presentation ? it.presentation : null;
    const items: MenuItem[] = [
      item("rename", "Rename", () => rename("item", it.id)),
      ...(it.kind.type === "header" ? [item("edit-header", "Edit Header…", () => void session.editHeader(it))] : []),
      item("duplicate", "Duplicate", () => void session.duplicateItem(it)),
      item("delete", "Remove from Playlist…", () => void session.removeItem(it)),
      sep,
      item("add-header", "Add Header After…", () => void session.addHeader(pl.id, it.id)),
    ];
    if (pres) {
      items.push(
        addToPlaylistMenu(pres),
        item("reveal", "Reveal in Library", () => void session.revealInLibrary(pres.id)),
        item("open-reflow", "Open in Reflow", () => {
          void session.openPresentation(pres.id, { playlist: pl.id, item: it.id }).then(() => shell.setMode("reflow"));
        }),
      );
    }
    openMenu(e, items);
  }

  // ---- inline rename -------------------------------------------------------------------------

  function isRenaming(kind: Renaming["kind"], id: string) {
    return session.renaming?.kind === kind && session.renaming.id === id;
  }

  function renameField(node: HTMLInputElement, from: string) {
    const r = session.renaming!;
    let done = false;
    const finish = (commit: boolean) => {
      if (done) return;
      done = true;
      if (commit) void session.rename(r, from, node.value);
      else session.renaming = null;
    };
    const onKey = (e: KeyboardEvent) => {
      e.stopPropagation();
      if (e.key === "Enter") finish(true);
      else if (e.key === "Escape") finish(false);
    };
    const onBlur = () => finish(true);
    node.value = from;
    queueMicrotask(() => {
      node.focus();
      node.select();
    });
    node.addEventListener("keydown", onKey);
    node.addEventListener("blur", onBlur);
    return () => {
      node.removeEventListener("keydown", onKey);
      node.removeEventListener("blur", onBlur);
    };
  }

  // ---- drag and drop -------------------------------------------------------------------------

  const drops: DropHandlers = {
    onPlaylist: (id, p) => {
      if (p.kind === "presentation") void session.addToPlaylist(id, p).then(() => {
          if (id === session.playlistId) void session.selectPlaylist(id, true);
        });
    },
    onList: (after, p) => {
      const pl = session.playlist;
      if (showView.listSource !== "playlist" || !pl) return;
      if (p.kind === "presentation") void session.addToPlaylist(pl.id, p, after);
      else if (p.id !== after) void session.moveItem(p.id, after);
    },
    onSpring: (id) => {
      void session.selectPlaylist(id);
    },
    onNode: (d, p) => {
      const to = treeMove(session.playlistTree, p.id, d);
      const from = nodePlace(session.playlistTree, p.id);
      if (to && from) void moveNode(p.id, from, to);
    },
    onFolderSpring: (id) => {
      showView.expandedFolders[id] = true;
    },
  };

  function press(e: PointerEvent, payload: DragPayload) {
    if (session.renaming) return;
    drag.press(e, payload, drops);
  }

  let nodeDrag = $derived(drag.payload?.kind === "node");
</script>

{#snippet renameInput(from: string)}
  <input class="rename" aria-label="New name" spellcheck="false" {@attach (n: HTMLInputElement) => renameField(n, from)} />
{/snippet}

{#snippet tree(nodes: PlaylistTreeNode[], depth: number)}
  {#each nodes as n (n.id)}
    {@const folder = n.kind === "folder"}
    {@const open = folder && !!showView.expandedFolders[n.id]}
    {@const ring = drag.overNode === `into:${n.id}` || (folder && drag.overFolder === n.id) || (!folder && drag.overPlaylist === n.id)}
    <li>
      {#if isRenaming("playlist", n.id)}
        <div class="renaming" style:padding-left={`${6 + depth * 16}px`}>{@render renameInput(n.name)}</div>
      {:else}
        <button
          class="row"
          class:folder
          class:selected={!folder && showView.listSource === "playlist" && session.playlistId === n.id}
          class:live={!folder && $liveSlide?.playlist?.playlist === n.id}
          class:ring
          class:before={drag.overNode === `before:${n.id}`}
          class:dragging={drag.payload?.kind === "node" && drag.payload.id === n.id}
          style:padding-left={`${6 + depth * 16}px`}
          aria-expanded={folder ? open : undefined}
          aria-current={!folder && session.playlistId === n.id && showView.listSource === "playlist" ? "page" : undefined}
          aria-label={folder ? `${open ? "Collapse" : "Expand"} folder ${n.name}` : `Playlist ${n.name}`}
          title="Double-click to rename · drag to move"
          onclick={() => {
            if (drag.consumeClick()) return;
            if (folder) showView.toggleFolder(n.id);
            else void session.selectPlaylist(n.id);
          }}
          ondblclick={() => rename("playlist", n.id)}
          oncontextmenu={(e) => playlistMenu(e, n)}
          onpointerdown={(e) => press(e, { kind: "node", id: n.id, name: n.name, folder })}
          data-tree-node={n.id}
          data-tree-folder={folder ? (open ? "open" : "closed") : undefined}
          data-drop-playlist={folder ? undefined : n.id}
        >
          {#if folder}
            <span class="tw">{#if open}<ChevronDown size={12} />{:else}<ChevronRight size={12} />{/if}</span>
            {#if open}<FolderOpen size={15} />{:else}<Folder size={15} />{/if}
          {:else}
            <span class="tw"></span>
            <ListMusic size={15} />
          {/if}
          <span class="label">{n.name}</span>
          <span class="n">{folder ? countPlaylists(n) : n.itemCount}</span>
        </button>
      {/if}
      {#if open && n.children.length}<ul>{@render tree(n.children, depth + 1)}</ul>{/if}
    </li>
  {/each}
{/snippet}

<div class="libpanel">
  <div class="sources" style:height={`min(${showView.layout.libraryHeight}px, 60%)`}>
    <div class="shdr">
      <h2 title={siteState.codex ? `The ${collection} collection` : undefined}>{collection}</h2>
      <button class="ibtn" onclick={() => session.newLibrary()} title="New library" aria-label="New library"><Plus size={15} /></button>
    </div>
    <ul class="list" aria-label="Libraries">
      {#each libraryState.libraries as l (l.id)}
        <li>
          {#if isRenaming("library", l.id)}
            <div class="renaming">{@render renameInput(l.name)}</div>
          {:else}
            <button
              class="row"
              class:selected={fromLibrary && libraryState.libraryId === l.id}
              aria-current={fromLibrary && libraryState.libraryId === l.id ? "page" : undefined}
              onclick={() => libraryState.selectLibrary(l.id)}
              ondblclick={() => rename("library", l.id)}
              oncontextmenu={(e) => libraryMenu(e, l)}
              title={`${l.name} (${l.presentationCount})`}
            >
              <LibraryIcon size={15} />
              <span class="label">{l.name}</span>
              <span class="n">{l.presentationCount}</span>
            </button>
          {/if}
        </li>
      {:else}
        <li class="empty">No libraries</li>
      {/each}
    </ul>
    <div class="shdr">
      <h2>Playlists</h2>
      <button class="ibtn" onclick={() => session.newPlaylist(true)} title="New folder" aria-label="New playlist folder"><FolderPlus size={15} /></button>
      <button class="ibtn" onclick={() => session.newPlaylist(false)} title="New playlist" aria-label="New playlist"><Plus size={15} /></button>
    </div>
    <ul class="list" aria-label="Playlists">
      {@render tree(serviceTree, 0)}
      {#if !serviceTree.length}<li class="empty">No playlists</li>{/if}
      {#if nodeDrag}
        <li>
          <div class="root-drop" class:on={drag.overNode === "root"} data-tree-root>Drop here to move to the top level</div>
        </li>
      {/if}
    </ul>
  </div>

  <Splitter dir="row" label="Resize the library and playlist panes" value={showView.layout.libraryHeight} onchange={(v) => showView.setLayout({ libraryHeight: v })} />

  <div class="contents">
    <div class="chdr">
      <h2 title={srcName}>{srcName}</h2>
      <span class="n">{srcCount}</span>
      {#if fromLibrary}
        <button class="ibtn" onclick={() => session.newPresentation()} title="New presentation (Ctrl+N)" aria-label="New presentation"><Plus size={15} /></button>
      {:else}
        <button class="hdr-btn" onclick={() => session.playlist && session.addHeader(session.playlist.id)} disabled={!session.playlist} title="Add a header to this playlist">+ Header</button>
      {/if}
    </div>
    <label class="search">
      <Search size={13} />
      <input
        type="search"
        placeholder={fromLibrary ? `Search ${srcName}` : "Filter the playlist"}
        bind:value={libraryState.search}
        onkeydown={onFilterKey}
        spellcheck="false"
        aria-label={fromLibrary ? "Filter library" : "Filter the playlist"}
      />
      {#if libraryState.search}
        <button class="ibtn clear" onclick={() => (libraryState.search = "")} aria-label="Clear filter"><X size={12} /></button>
      {/if}
    </label>

    <div class="items">
      {#if fromLibrary}
        <ul class="list" aria-label="Presentations">
          {#each presentations as p (p.id)}
            {@const isLive = $liveSlide?.presentation === p.id}
            {@const on = showView.selectedPresentationId === p.id}
            <li>
              {#if isRenaming("presentation", p.id)}
                <div class="renaming">{@render renameInput(p.name)}</div>
              {:else}
                <button
                  class="doc"
                  class:selected={on}
                  aria-pressed={on}
                  onclick={() => pickPresentation(p)}
                  ondblclick={() => addPresentation(p)}
                  oncontextmenu={(e) => presentationMenu(e, p)}
                  onpointerdown={(e) => press(e, { kind: "presentation", id: p.id, name: p.name })}
                  title="Double-click or drag onto a playlist to add"
                  data-presentation={p.id}
                >
                  <span class="dname">{p.name}{#if isLive}<Chip kind="live" />{/if}</span>
                  <span class="dsub">{[p.category, `${p.slideCount} ${p.slideCount === 1 ? "slide" : "slides"}`].filter(Boolean).join(" · ")}</span>
                </button>
              {/if}
            </li>
          {:else}
            <li class="empty">{libraryState.libraryLoading ? "Loading…" : q ? `Nothing matches “${libraryState.search}”.` : "No presentations"}</li>
          {/each}
        </ul>
      {:else if session.playlist}
        <ul class="list pl" aria-label="Playlist items" data-drop-list>
          {#each playlistItems as it (it.id)}
            {@const dragging = drag.payload?.kind === "item" && drag.payload.id === it.id}
            {#if it.kind.type === "header"}
              <li class="hdr-li" data-item-id={it.id} class:dragging>
                {#if isRenaming("item", it.id)}
                  <div class="renaming">{@render renameInput(it.displayName)}</div>
                {:else}
                  <div class="hdr-row" style:--hc={cssColor(it.kind.color, 1) ?? "var(--text-meta)"}>
                    <button
                      class="hdr-name"
                      onclick={() => clickItem(it)}
                      ondblclick={() => rename("item", it.id)}
                      oncontextmenu={(e) => itemMenu(e, it)}
                      onpointerdown={(e) => press(e, { kind: "item", id: it.id, name: it.displayName })}
                      title="Double-click to rename · drag to move · right-click for the colour"
                    >{it.displayName}</button>
                    <button class="hdr-x" onclick={() => session.removeItem(it)} aria-label={`Remove header ${it.displayName}`} title="Remove header"><X size={12} /></button>
                  </div>
                {/if}
              </li>
            {:else}
              {@const isLive = $livePlaylistItem === it.id}
              {@const missing = it.kind.type === "presentation" && !it.presentation}
              {@const playable = it.kind.type === "media" || it.kind.type === "videoInput"}
              <li class="item-li" data-item-id={it.id}>
                {#if isRenaming("item", it.id)}
                  <div class="renaming">{@render renameInput(it.displayName)}</div>
                {:else}
                  <button
                    class="row item"
                    class:selected={showView.listSource === "playlist" && showView.selectedItemId === it.id}
                    class:disabled={!it.enabled || missing}
                    class:dragging
                    class:playable
                    onclick={() => clickItem(it)}
                    ondblclick={() => (playable ? goLive(it) : rename("item", it.id))}
                    oncontextmenu={(e) => itemMenu(e, it)}
                    onpointerdown={(e) => press(e, { kind: "item", id: it.id, name: it.displayName })}
                    title={missing ? "Presentation not found in the library" : "Drag to reorder"}
                  >
                    <span class="kdot" style:background={itemDot(it)}></span>
                    {#if it.kind.type === "media"}<Film size={13} />{/if}
                    <span class="label" class:two={!!summaryMeta(it.presentation)}>{it.displayName}{#if summaryMeta(it.presentation)}<small>{summaryMeta(it.presentation)}</small>{/if}</span>
                    {#if isLive}<Chip kind="live" />{/if}
                    {#if missing}<span class="tag">missing</span>{/if}
                  </button>
                  {#if playable}
                    <button class="go" onclick={() => goLive(it)} aria-label={`Show ${it.displayName}`} title="Show on screen"><Play size={12} /></button>
                  {/if}
                {/if}
              </li>
            {/if}
          {:else}
            <li class="empty">{q ? "No matches" : "Empty playlist: drag presentations here"}</li>
          {/each}
        </ul>
      {:else}
        <p class="empty">Select a library or a playlist.</p>
      {/if}
    </div>
  </div>
</div>

{#if drag.payload && drag.payload.kind !== "audio"}
  <div class="ghost" style:left={`${drag.x + 12}px`} style:top={`${drag.y + 8}px`} aria-hidden="true">
    {#if drag.payload.kind === "node"}<ListMusic size={13} />{:else if drag.payload.kind === "media"}<Film size={13} />{:else}<FileText size={13} />{/if}{drag.payload.name}
  </div>
  {#if drag.line}
    <div class="insert-line" style:top={`${drag.line.y - 1}px`} style:left={`${drag.line.left}px`} style:width={`${drag.line.width}px`} aria-hidden="true"></div>
  {/if}
{/if}

<style>
  .libpanel {
    height: 100%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface-panel);
  }
  .sources {
    flex: none;
    min-height: 0;
    overflow-y: auto;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--line);
  }
  .shdr {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 10px 8px 4px 12px;
  }
  h2 {
    margin: 0;
    flex: 1;
    min-width: 0;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--text-label);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list,
  .list :global(ul) {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .sources > .list {
    padding: 0 6px;
  }
  .row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 7px;
    min-height: 27px;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
    text-align: left;
    font-size: 13px;
    outline: 2px dashed transparent;
    outline-offset: -2px;
  }
  .row.folder {
    color: var(--text);
    font-weight: 600;
  }
  .row :global(svg) {
    flex: none;
    color: var(--text-label);
  }
  .row:hover {
    background: var(--surface-raised);
  }
  .row.selected {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .row.selected :global(svg) {
    color: var(--accent-link);
  }
  .row.live:not(.selected) .label {
    color: var(--live);
  }
  .row.ring {
    outline-color: var(--ok);
  }
  .row.before {
    box-shadow: inset 0 2px 0 var(--ok);
  }
  .row.dragging,
  .dragging {
    opacity: 0.4;
  }
  .row.disabled {
    opacity: 0.5;
  }
  .tw {
    width: 12px;
    flex: none;
    display: grid;
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .label.two {
    display: flex;
    flex-direction: column;
    line-height: 1.25;
  }
  .label small {
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text-meta);
    font-size: 11px;
  }
  .n {
    flex: none;
    color: var(--text-meta);
    font-size: 11px;
    font-weight: 400;
  }
  .row.selected .n {
    color: var(--accent-link);
  }
  .renaming {
    padding: 2px 0;
  }
  .rename {
    width: 100%;
    height: 26px;
    padding: 0 6px;
    border: 1px solid var(--accent);
    border-radius: var(--r-sm);
    background: var(--surface-app);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    user-select: text;
  }
  .root-drop {
    margin: 4px 0 0;
    padding: 6px 8px;
    border: 1px dashed var(--line-strong);
    border-radius: var(--r);
    color: var(--text-label);
    font-size: 11px;
    text-align: center;
  }
  .root-drop.on {
    border-color: var(--ok);
    color: var(--text);
  }
  .contents {
    flex: 1 1 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .chdr {
    flex: none;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 8px 6px 12px;
  }
  .chdr h2 {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0;
    text-transform: none;
    color: var(--text);
  }
  .hdr-btn {
    flex: none;
    padding: 2px 8px;
    border-radius: var(--r-sm);
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    color: var(--text);
    font-size: 11px;
  }
  .hdr-btn:hover:not(:disabled) {
    background: var(--surface-overlay);
  }
  .search {
    flex: none;
    margin: 0 8px 6px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 6px 0 8px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text-label);
  }
  .search input {
    flex: 1;
    min-width: 0;
    height: 28px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12px;
    outline: none;
  }
  .clear {
    width: 18px;
    height: 18px;
  }
  .items {
    position: relative;
    flex: 1 1 0;
    min-height: 0;
    overflow-y: auto;
    padding: 0 6px 8px;
  }
  .doc {
    width: 100%;
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 5px 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
  }
  .doc:hover {
    background: var(--surface-raised);
  }
  .doc.selected {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .dname {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .dsub {
    font-size: 11px;
    color: var(--text-label);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }
  .doc.selected .dsub {
    color: #d8d3ff;
  }
  .list.pl {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 2px 0;
    min-height: 40px;
  }
  .row.item {
    gap: 8px;
    padding: 6px 8px;
  }
  .kdot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }
  .item-li {
    position: relative;
  }
  .row.playable {
    padding-right: 34px;
  }
  .go {
    position: absolute;
    right: 5px;
    top: 50%;
    transform: translateY(-50%);
    width: 24px;
    height: 19px;
    display: grid;
    place-items: center;
    border-radius: var(--r-xs);
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    color: var(--text);
  }
  .go:hover {
    background: var(--live);
    color: var(--live-ink);
  }
  .hdr-li {
    margin-top: 6px;
  }
  .hdr-row {
    display: flex;
    align-items: center;
    border-left: 4px solid var(--hc);
    border-radius: 3px;
    background: var(--surface-raised);
  }
  .hdr-name {
    flex: 1;
    min-width: 0;
    padding: 5px 8px;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
    font-size: 11px;
    font-weight: 800;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .hdr-x {
    flex: none;
    width: 22px;
    height: 22px;
    margin-right: 2px;
    display: grid;
    place-items: center;
    padding: 0;
    border: 0;
    border-radius: var(--r-xs);
    background: transparent;
    color: var(--text-label);
  }
  .hdr-x:hover {
    background: var(--surface-overlay);
    color: var(--text);
  }
  .tag {
    font-size: 10px;
    text-transform: uppercase;
    color: var(--warn);
    border: 1px solid rgba(240, 180, 41, 0.5);
    border-radius: 3px;
    padding: 0 4px;
  }
  .empty {
    margin: 0;
    padding: 8px 12px;
    color: var(--text-meta);
    font-size: 12px;
    list-style: none;
  }
  .ghost {
    position: fixed;
    z-index: 95;
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 260px;
    padding: 4px 9px;
    border-radius: var(--r-sm);
    background: var(--accent-sel);
    border: 1px solid var(--accent-ring);
    color: var(--text-strong);
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    pointer-events: none;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
  }
  .insert-line {
    position: fixed;
    z-index: 94;
    height: 3px;
    border-radius: 2px;
    background: var(--ok);
    pointer-events: none;
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.4);
  }
</style>
