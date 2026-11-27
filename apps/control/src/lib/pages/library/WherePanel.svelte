<script lang="ts">
  // The Library's "where" column (the board's first part): the collection card (opens the
  // Codex's collections; picking one switches this box), the tree (Songs by shelf, Media,
  // Design, Recently deleted) and the connection card (plan 3.4; "2 others have this
  // collection open", G8). While a song is in Edit the column folds into an icon
  // rail (as the board draws it); pointing at it opens the full column over the page.
  import Music from "lucide-svelte/icons/music";
  import Folder from "lucide-svelte/icons/folder";
  import ImageIcon from "lucide-svelte/icons/image";
  import Film from "lucide-svelte/icons/film";
  import Palette from "lucide-svelte/icons/palette";
  import Shapes from "lucide-svelte/icons/shapes";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import ChevronDown from "lucide-svelte/icons/chevron-down";
  import Library from "lucide-svelte/icons/library";
  import Images from "lucide-svelte/icons/images";
  import FolderSync from "lucide-svelte/icons/folder-sync";
  import ListVideo from "lucide-svelte/icons/list-video";
  import Tags from "lucide-svelte/icons/tags";
  import Plus from "lucide-svelte/icons/plus";
  import { binsPage } from "../../state/bins.svelte";
  import { binDropTarget, binLabel, mediaPlaylists } from "./bins";
  import { binDrag } from "./binDrag.svelte";
  import { syncedText } from "./sync";
  import DormantOffer from "./DormantOffer.svelte";
  import CollectionPresence from "./CollectionPresence.svelte";
  import { StatusDot } from "../../ui";
  import type { IconComponent } from "../../ui";
  import { libraryPage, libraryState, type LibraryView } from "../../state/library.svelte";
  import { siteState } from "../../state/site.svelte";
  import { session } from "../../state/session.svelte";
  import { cssColor } from "../../model";
  import { mediaKind, type ConnectionCard } from "./logic";

  let { card }: { card: ConnectionCard } = $props();

  let codex = $derived(siteState.codex);
  let active = $derived(libraryPage.collections.find((c) => c.active) ?? null);
  let collectionName = $derived(codex?.collectionName ?? active?.collection.name ?? "…");
  /** The box that holds the Codex, in words. */
  let codexBox = $derived(
    !codex || codex.role === "host" ? (session.server?.name ?? "this server") : (libraryPage.managedOn?.name ?? codex.sync?.name ?? "the site's server"),
  );
  let codexState = $derived<{ status: "ok" | "warn" | "error"; text: string }>(
    card.status === "error" ? { status: "error", text: "Offline" } : card.readOnly ? { status: "warn", text: "Unreachable" } : { status: "ok", text: "Connected" },
  );

  interface Node {
    view: LibraryView;
    label: string;
    icon: IconComponent;
    count: number;
    indent?: boolean;
    /** A bin or media playlist: media can be dropped on it (`data-drop-bin`). Not in the rail. */
    drop?: string;
    /** The row's tooltip when it isn't a drop target. */
    note?: string;
  }

  let shelves = $derived(libraryState.libraries);
  let mediaCount = (kind: "image" | "video") => libraryPage.media.filter((m) => mediaKind(m.asset) === kind).length;
  let tree = $derived<{ name: string; nodes: Node[] }[]>([
    {
      name: "Songs",
      nodes: [
        { view: { kind: "songs", shelf: null }, label: "All songs", icon: Music, count: libraryPage.songs.length },
        ...shelves.map((l) => ({
          view: { kind: "songs", shelf: l.id } as LibraryView,
          label: l.name,
          icon: Folder,
          count: libraryPage.songs.filter((s) => s.library === l.id).length,
          indent: true,
        })),
      ],
    },
    {
      name: "Media",
      nodes: [
        { view: { kind: "media", media: "image" }, label: "Images", icon: ImageIcon, count: mediaCount("image") },
        { view: { kind: "media", media: "video" }, label: "Videos", icon: Film, count: mediaCount("video") },
        ...binsPage.rows.map((r) => ({
          view: { kind: "bin", id: r.id } as LibraryView,
          label: binLabel(r),
          icon: r.kind === "watched" ? FolderSync : Images,
          count: r.itemCount,
          indent: true,
          // A watched folder's items follow the folder: the server refuses adds (D09-4).
          drop: binDropTarget(r),
          note: r.kind === "watched" ? `${binLabel(r)}: follows a folder on the server. Add files to that folder to add them here` : undefined,
        })),
        ...mediaPlaylists(session.playlistTree).map((p) => ({
          view: { kind: "mediaPlaylist", id: p.id } as LibraryView,
          label: p.name,
          icon: ListVideo,
          count: p.itemCount,
          indent: true,
          drop: `playlist:${p.id}`,
        })),
      ],
    },
    {
      name: "Design",
      nodes: [
        { view: { kind: "themes" }, label: "Themes", icon: Palette, count: libraryPage.themes.length },
        { view: { kind: "props" }, label: "Props", icon: Shapes, count: libraryPage.props.length },
        { view: { kind: "groups" }, label: "Groups", icon: Tags, count: session.globalGroups.length },
      ],
    },
  ]);

  function same(a: LibraryView, b: LibraryView): boolean {
    if (a.kind !== b.kind) return false;
    if (a.kind === "songs" && b.kind === "songs") return a.shelf === b.shelf;
    if (a.kind === "media" && b.kind === "media") return a.media === b.media;
    if ((a.kind === "bin" && b.kind === "bin") || (a.kind === "mediaPlaylist" && b.kind === "mediaPlaylist")) return a.id === b.id;
    return true;
  }

  function pick(v: LibraryView) {
    if (v.kind === "bin" || v.kind === "mediaPlaylist") {
      binsPage.open(v.kind, v.id);
      return;
    }
    libraryPage.go(v);
    void libraryPage.search();
  }
  const key = (n: Node) => JSON.stringify(n.view) + (n.indent ? "-" : "");

  let collapsed = $derived(libraryPage.mode === "edit");
  let showFull = $derived(!collapsed || libraryPage.railOpen);
  const enter = () => {
    if (collapsed) libraryPage.railOpen = true;
  };
  const leave = () => (libraryPage.railOpen = false);
  function manage() {
    libraryPage.collectionsOpen = false;
    libraryPage.go({ kind: "collections" });
  }
</script>

<div class="slot" class:collapsed role="presentation" onmouseenter={enter} onmouseleave={leave}>
{#if collapsed && !libraryPage.railOpen}
  <nav class="rail" aria-label="Library, folded while editing">
    <button type="button" class="rbtn lib" title={`${collectionName} in the Codex: point here to open the full column`} aria-label="Open the Library column" onclick={enter} onfocus={enter}>
      <Library size={18} strokeWidth={1.8} aria-hidden="true" />
    </button>
    {#each tree as sec, si (sec.name)}
      {#each sec.nodes.filter((x) => !x.drop) as n, ni (key(n))}
        {#if ni === 0 && si > 0}<span class="rsep" aria-hidden="true"></span>{/if}
        <button type="button" class="rbtn" class:cur={same(n.view, libraryPage.view)} title={`${n.label} (${n.count})`} aria-label={`${n.label} (${n.count})`} onclick={() => pick(n.view)}>
          {#if sec.name === "Songs"}<span class="letter">{ni === 0 ? "All" : n.label.charAt(0)}</span>{:else}<n.icon size={16} strokeWidth={1.8} aria-hidden="true" />{/if}
          <span class="rcount">{n.count}</span>
        </button>
      {/each}
    {/each}
    <span class="rconn" title={card.title}><StatusDot status={card.status} />{card.status === "ok" ? "ok" : card.status === "error" ? "off" : "!"}</span>
  </nav>
{/if}
{#if showFull}
<nav class="where" class:over={collapsed} aria-label="Library">
  <button
    type="button"
    class="collection"
    class:open={libraryPage.collectionsOpen}
    aria-expanded={libraryPage.collectionsOpen}
    onclick={() => (libraryPage.collectionsOpen = !libraryPage.collectionsOpen)}
  >
    <span class="cap">Collection</span>
    <span class="name"><span class="text">{collectionName}</span><ChevronDown size={15} strokeWidth={1.8} aria-hidden="true" /></span>
    <span class="sub">in Codex on {codexBox}</span>
  </button>

  {#if libraryPage.collectionsOpen}
    <div class="card list" role="group" aria-label="Collections in the Codex">
      <span class="cap">Collections in the Codex</span>
      {#each libraryPage.collections as c (c.collection.id)}
        <button
          type="button"
          class="coll"
          class:on={c.active}
          aria-pressed={c.active}
          disabled={libraryPage.busy || card.readOnly}
          onclick={() => libraryPage.switchCollection(c)}
        >
          <span class="swatch" style:background={cssColor(c.collection.color, 1) ?? "var(--line-strong)"} aria-hidden="true"></span>
          <span class="cname">{c.collection.name ?? "Untitled"}{#if syncedText(c.boxes)}<span class="synced" class:behind={!syncedText(c.boxes)?.ok}>{syncedText(c.boxes)?.text}</span>{/if}</span>
          {#if c.active}<span class="here">shown here</span>{/if}
          <span class="count">{c.counts.presentations}</span>
        </button>
      {:else}
        <span class="muted">Loading…</span>
      {/each}
      <span class="cap gap">Codex</span>
      <span class="box">
        <StatusDot status={codexState.status} />
        <span class="bname">
          <span class="bn">Codex on {codexBox}</span>
          <span class="bh">{codex?.role === "host" ? session.baseUrl.replace(/^https?:\/\//, "") : codex?.role === "client" ? "This box shows a copy" : ""}</span>
        </span>
        <span class="bstate" class:warn={codexState.status === "warn"} class:err={codexState.status === "error"}>{codexState.text}</span>
      </span>
      <span class="note">One Codex for the whole site. Other buildings reach it over the site's VPN.</span>
      <button type="button" class="link" onclick={manage}>Manage collections…</button>
    </div>
  {/if}

  {#each tree as sec (sec.name)}
    <h2 class="cap sec">{sec.name}</h2>
    {#each sec.nodes as n (key(n))}
      <button
        type="button"
        class="node"
        class:indent={n.indent}
        class:cur={same(n.view, libraryPage.view)}
        class:dropover={!!n.drop && binDrag.over === n.drop}
        class:droppable={!!n.drop && !!binDrag.files}
        data-drop-bin={n.drop}
        title={n.drop ? `${n.label}: drop images or videos here to add them` : n.note}
        aria-current={same(n.view, libraryPage.view) ? "page" : undefined}
        onclick={() => pick(n.view)}
      >
        <n.icon size={16} strokeWidth={1.8} aria-hidden="true" />
        <span class="label">{n.label}</span>
        <span class="count">{n.count}</span>
      </button>
    {/each}
    {#if sec.name === "Media"}
      <div class="adds">
        <button type="button" class="add" disabled={card.readOnly || binsPage.busy} onclick={() => binsPage.createBin()}><Plus size={13} strokeWidth={2} aria-hidden="true" />New bin</button>
        <button type="button" class="add" disabled={card.readOnly || binsPage.busy} onclick={() => binsPage.createMediaPlaylist()}><Plus size={13} strokeWidth={2} aria-hidden="true" />New media playlist</button>
      </div>
    {/if}
  {/each}

  <button
    type="button"
    class="node trash"
    class:cur={libraryPage.view.kind === "trash"}
    aria-current={libraryPage.view.kind === "trash" ? "page" : undefined}
    onclick={() => libraryPage.go({ kind: "trash" })}
  >
    <Trash2 size={16} strokeWidth={1.8} aria-hidden="true" />
    <span class="label">Recently deleted</span>
    <span class="count">{libraryPage.trash.length}</span>
  </button>

  <section class="card conn" aria-label="Codex connection">
    <div class="row">
      <StatusDot status={card.status} />
      <span class="ct">{card.title}</span>
    </div>
    <p class="detail">{card.detail}</p>
    <CollectionPresence collection={codex?.collection ?? null} />
    {#if libraryPage.managedOn}
      <p class="detail managed">Uploads and collections are managed on {libraryPage.managedOn.name ?? codex?.sync?.name ?? "the site's Codex box"}.</p>
    {/if}
  </section>
  <DormantOffer readOnly={card.readOnly} />
</nav>
{/if}
</div>

<style>
  .slot {
    position: relative;
    flex: 0 0 256px;
    display: flex;
    z-index: 6;
  }
  .slot.collapsed {
    flex-basis: 58px;
  }
  .where.over {
    position: absolute;
    left: 0;
    top: 0;
    bottom: 0;
    box-shadow: 12px 0 32px rgba(0, 0, 0, 0.55);
  }
  .rail {
    width: 58px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    padding: 10px 4px;
    border-right: 1px solid var(--line);
    background: var(--surface-panel);
    overflow: hidden;
    box-sizing: border-box;
  }
  .rbtn {
    width: 48px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 1px;
    padding: 5px 0;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--text-2);
  }
  .rbtn:hover {
    background: var(--surface-raised);
  }
  .rbtn.cur {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .rbtn.lib {
    height: 40px;
    justify-content: center;
    margin-bottom: 6px;
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    color: var(--accent-link);
    border-radius: 8px;
  }
  .letter {
    font-size: 12.5px;
    font-weight: 800;
    line-height: 16px;
  }
  .rcount {
    font-size: 10.5px;
    color: var(--text-meta);
    font-variant-numeric: tabular-nums;
  }
  .rbtn.cur .rcount {
    color: var(--accent-link);
  }
  .rsep {
    width: 32px;
    height: 1px;
    margin: 5px 0;
    background: var(--line);
  }
  .rconn {
    margin-top: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 3px;
    font-size: 9.5px;
    color: var(--text-label);
  }
  .where {
    width: 256px;
    flex: none;
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 10px 8px;
    border-right: 1px solid var(--line);
    background: var(--surface-panel);
    overflow-y: auto;
    box-sizing: border-box;
  }
  .cap {
    margin: 0;
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: var(--label-tracking);
    text-transform: uppercase;
    color: var(--text-label);
  }
  .sec {
    margin: 14px 4px 4px;
  }
  .collection {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    gap: 2px;
    padding: 8px 10px;
    border-radius: var(--r-card);
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    color: var(--text);
    text-align: left;
  }
  .collection.open {
    border-color: var(--accent);
  }
  .collection .name {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .collection .text {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-md);
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .collection .sub {
    font-size: var(--fs-xs);
    color: var(--text-label);
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border-radius: var(--r-card);
    background: var(--surface-raised);
    border: 1px solid var(--line-strong);
  }
  .list {
    margin-top: 4px;
  }
  .gap {
    margin-top: 4px;
  }
  .coll {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text);
    text-align: left;
    font-size: 12.5px;
  }
  .coll:hover:not(:disabled) {
    background: var(--surface-overlay);
  }
  .coll.on {
    background: var(--accent-sel);
  }
  .coll:disabled:not(.on) {
    opacity: 0.6;
  }
  .swatch {
    width: 10px;
    height: 10px;
    border-radius: 3px;
    flex: none;
  }
  .cname {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .here {
    font-size: 10.5px;
    color: var(--accent-link);
  }
  .count {
    color: var(--text-meta);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .muted {
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .box {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: var(--r-sm);
    background: var(--surface-overlay);
  }
  .bname {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .bn {
    font-size: 12.5px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bh {
    font-size: var(--fs-xs);
    color: var(--text-label);
  }
  .bstate {
    font-size: 10.5px;
    font-weight: 600;
    color: var(--ok-text);
  }
  .bstate.warn {
    color: var(--live);
  }
  .bstate.err {
    color: var(--danger-text);
  }
  .note {
    font-size: var(--fs-xs);
    color: var(--text-meta);
    line-height: 1.4;
  }
  .link {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-link);
    font-size: var(--fs-sm);
  }
  .link:hover:not(:disabled) {
    color: var(--accent-link-hover);
    text-decoration: underline;
  }
  .node {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
    text-align: left;
    font-size: var(--fs);
  }
  .node.indent {
    padding-left: 22px;
  }
  .node:hover {
    background: var(--surface-raised);
    color: var(--text);
  }
  .node.cur {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .node :global(svg) {
    flex: none;
    color: var(--text-label);
  }
  .node.cur :global(svg) {
    color: var(--accent-ring);
  }
  .node.droppable {
    outline: 1px dashed var(--line-strong);
    outline-offset: -1px;
  }
  .node.dropover {
    background: var(--accent-sel-2);
    outline: 2px solid var(--accent);
    outline-offset: -2px;
    color: var(--text-strong);
  }
  .synced {
    display: block;
    white-space: normal;
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--ok-text);
  }
  .synced.behind {
    color: var(--live);
  }
  .adds {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 10px;
    padding: 2px 8px 4px 22px;
  }
  .add {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 0;
    border: 0;
    background: transparent;
    color: var(--accent-link);
    font-size: var(--fs-sm);
  }
  .add:hover:not(:disabled) {
    text-decoration: underline;
  }
  .add:disabled {
    color: var(--text-label);
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .trash {
    margin-top: 14px;
  }
  .conn {
    margin-top: auto;
    padding: 10px;
    border-color: var(--line);
    gap: 6px;
  }
  .conn .row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .ct {
    font-weight: 600;
  }
  .detail {
    margin: 0;
    font-size: 11.5px;
    color: var(--text-label);
    line-height: 1.4;
  }
  .managed {
    color: var(--accent-link);
  }
</style>
