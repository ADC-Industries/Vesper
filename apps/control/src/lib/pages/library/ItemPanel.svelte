<script lang="ts">
  // The Library's item (the board's third part): kind, name and meta, the View | Edit switch,
  // "Add to *playlist*", and the item as drawn: a song's groups and slides (in Edit: the slide
  // editor's canvas and panel, with Undo / Redo; edits save as they happen, question 5), a
  // theme's template slides, a prop's or a media file's picture and properties (a missing one
  // with Relink), a deleted item's details with Restore.
  import Music from "lucide-svelte/icons/music";
  import ThemeSecondLanguage from "./ThemeSecondLanguage.svelte";
  import RotateCcw from "lucide-svelte/icons/rotate-ccw";
  import ListPlus from "lucide-svelte/icons/list-plus";
  import Undo2 from "lucide-svelte/icons/undo-2";
  import Redo2 from "lucide-svelte/icons/redo-2";
  import Link2 from "lucide-svelte/icons/link-2";
  import Pencil from "lucide-svelte/icons/pencil";
  import Copy from "lucide-svelte/icons/copy";
  import Trash2 from "lucide-svelte/icons/trash-2";
  import Palette from "lucide-svelte/icons/palette";
  // Song editing (the slide editor's canvas and inspector) loads on first use.
  const songEdit = () => import("./SongEdit.svelte");
  import PresenceBadge from "../../components/PresenceBadge.svelte";
  import LockBanner from "../../components/LockBanner.svelte";
  import { copyTyping, presence } from "../../state/presence.svelte";
  import { personLabel } from "../../state/presenceLogic";
  import type { DocumentUse, Presentation } from "$protocol";
  import type { Connection } from "../../server";
  import { CodexClient, assetBlob } from "../../server/codex";
  import { ScriptureClient } from "../../server/scripture";
  import { Button, EmptyState, Segmented, SlideTile } from "../../ui";
  import { flattenSlides, cssColor } from "../../model";
  import { flatPlaylists, libraryPage, libraryState } from "../../state/library.svelte";
  import { session } from "../../state/session.svelte";
  import { shell } from "../../state/shell.svelte";
  import { daysLeft, formatDuration, mediaKind, plural, shortTime, songThemeState } from "./logic";
  import ChunkError from "../../components/ChunkError.svelte";
  import MediaPrepCard from "./MediaPrepCard.svelte";

  let { conn, readOnly }: { conn: Connection; readOnly: boolean } = $props();

  let view = $derived(libraryPage.view);
  let id = $derived(libraryPage.selected);
  let shelfName = $derived(new Map(libraryState.libraries.map((l) => [l.id, l.name])));

  let song = $derived(view.kind === "songs" && id ? (libraryPage.songs.find((p) => p.id === id) ?? null) : null);
  /** A song of another collection (a search over every collection). */
  let foreign = $derived(view.kind === "songs" && id && !song ? (libraryPage.hits?.find((h) => h.id === id) ?? null) : null);
  let theme = $derived(view.kind === "themes" && id ? (libraryPage.themes.find((t) => t.id === id) ?? null) : null);
  let prop = $derived(view.kind === "props" && id ? (libraryPage.props.find((p) => p.id === id) ?? null) : null);
  let media = $derived(view.kind === "media" && id ? (libraryPage.media.find((m) => m.asset.id === id) ?? null) : null);
  let trashed = $derived(view.kind === "trash" && id ? (libraryPage.trash.find((t) => t.id === id) ?? null) : null);

  // The song's document (its groups and slides).
  let doc = $state.raw<Presentation | null>(null);
  let docError = $state<string | null>(null);
  $effect(() => {
    const pid = song?.id;
    const rev = song?.modifiedAt;
    void rev;
    doc = null;
    docError = null;
    if (!pid) return;
    let gone = false;
    conn.rest
      .presentation(pid)
      .then((p) => {
        if (!gone) doc = p;
      })
      .catch((e: unknown) => {
        if (!gone) docError = e instanceof Error ? e.message : String(e);
      });
    return () => {
      gone = true;
    };
  });

  let slides = $derived(doc ? flattenSlides(doc) : []);
  /** The arrangement (or the groups in document order) as chips. */
  let arrangement = $derived.by(() => {
    const d = doc;
    if (!d) return [];
    const arr = d.arrangements?.find((a) => a.id === d.selectedArrangement) ?? null;
    const secs = d.sections ?? [];
    const order = arr?.sections?.length ? arr.sections.map((sid) => secs.find((s) => s.id === sid)).filter((s) => !!s) : secs;
    return order.map((s) => ({ name: s.name ?? "Group", color: cssColor(s.color, 1) }));
  });

  let codex = $derived(new CodexClient(conn.rest));
  let mediaSrc = $derived.by(() => {
    // A missing file has no thumbnail (404): show the frame without a broken-image icon.
    if (!media || media.availability === "missing") return null;
    const b = assetBlob(media.asset);
    return b ? codex.blobThumbnailUrl(b.sha256, b.ext) : conn.rest.mediaThumbnailUrl(media.asset.id, 960);
  });

  let target = $derived(flatPlaylists(session.playlistTree).find((p) => p.id === libraryPage.target) ?? null);

  // "Used by": the documents that use the prop or the media file (refetched when it changes).
  let usedBy = $state.raw<DocumentUse[] | null>(null);
  let usedByError = $state<string | null>(null);
  $effect(() => {
    const what = prop
      ? ({ kind: "prop", id: prop.id, rev: prop.revision } as const)
      : media
        ? ({ kind: "media", id: media.asset.id, rev: media.revision } as const)
        : theme
          ? ({ kind: "theme", id: theme.id, rev: String(libraryPage.themesVersion) } as const)
          : null;
    usedBy = null;
    usedByError = null;
    if (!what) return;
    void what.rev;
    let gone = false;
    (what.kind === "theme" ? conn.rest.themeUsedBy(what.id) : codex.usedBy(what.kind, what.id))
      .then((u) => {
        if (!gone) usedBy = u;
      })
      .catch((e: unknown) => {
        if (!gone) usedByError = e instanceof Error ? e.message : String(e);
      });
    return () => {
      gone = true;
    };
  });
  const USE_KIND: Record<string, string> = { presentation: "Song", playlistNode: "Playlist", prop: "Prop", macro: "Macro", mediaBin: "Bin", theme: "Theme", stageLayout: "Stage layout", message: "Message", timer: "Timer" };

  let kind = $derived(song || foreign ? "Song" : theme ? "Theme" : prop ? "Prop" : media ? (mediaKind(media.asset) === "video" ? "Video" : "Image") : trashed ? "Deleted" : "");
  let name = $derived(song?.name ?? foreign?.title ?? theme?.name ?? prop?.name ?? (media ? (media.asset.name ?? "").replace(/\.[^.]+$/, "") : null) ?? trashed?.name ?? "");
  let meta = $derived(
    song
      ? [
          doc?.songInfo?.authors ?? doc?.songInfo?.artist ?? null,
          song.library ? shelfName.get(song.library) : null,
          plural(song.slideCount, "slide"),
          song.modifiedAt ? `changed ${shortTime(song.modifiedAt)}` : null,
        ]
          .filter(Boolean)
          .join(" · ")
      : foreign
        ? `In ${foreign.collectionName}`
        : theme
          ? `${plural(theme.slides.length, "template slide")} · ${theme.canvasSize.width}×${theme.canvasSize.height}`
          : prop
            ? "Props layer"
            : media
              ? [media.asset.probe?.width ? `${media.asset.probe.width}×${media.asset.probe.height}` : null, media.asset.probe?.duration ? formatDuration(media.asset.probe.duration) : null]
                  .filter(Boolean)
                  .join(" · ")
              : "",
  );

  const KIND: Record<string, string> = { presentation: "Song", playlistNode: "Playlist or folder", prop: "Prop", timer: "Timer", message: "Message", stageLayout: "Stage layout", library: "Shelf" };

  // Presence (G8): the song, theme or prop shown here is open in the Library; someone else
  // holding its edit lock keeps it in View until this client takes over.
  let presKind = $derived(theme ? ("theme" as const) : prop ? ("prop" as const) : ("presentation" as const));
  let presId = $derived(song?.id ?? theme?.id ?? prop?.id ?? null);
  $effect(() => {
    presence.setOpen("library", presId ? [{ kind: presKind, id: presId, place: "library" }] : null);
  });
  $effect(() => () => presence.setOpen("library", null));
  let lockedBy = $derived(presence.lockedBy(presKind, presId));
  // The Library can't edit a theme or prop, so it offers no take-over for them (D09-6, E09-1);
  // a theme's Rename and Delete wait while someone edits it (they'd be refused: E09-2).
  let lockedTitle = $derived(lockedBy ? `${personLabel(lockedBy)} is editing it: rename or delete it when they're done, or take over in the slide editor` : undefined);
  let modes = $derived([
    { value: "view" as const, label: "View" },
    {
      value: "edit" as const,
      label: "Edit",
      disabled: !song || readOnly || !!lockedBy,
      title: readOnly ? "Editing needs the site's server" : lockedBy ? "Someone else is editing it: take over first" : song ? undefined : "Only songs are edited here",
    },
  ]);
  let mode = $state<"view" | "edit">("view");
  $effect(() => {
    mode = libraryPage.mode;
  });
  function setMode(v: "view" | "edit") {
    if (v === "edit" && song) {
      void presence.claim("presentation", song.id);
      void libraryPage.startEdit(song.id);
    }
    else void libraryPage.endEdit();
  }
  // Someone else took over while this client edited: the song shows as in View (the banner says
  // who) until this client takes editing back; typing it couldn't save is kept and offered.
  let editing = $derived(libraryPage.mode === "edit" && !!song && !!libraryPage.editState && !lockedBy);
  let kept = $derived(libraryPage.editState?.lockedEdits.length ?? 0);
  $effect(() => {
    const st = libraryPage.editState;
    if (!lockedBy && st?.lockedEdits.length) void st.sendLocked();
  });
  let undo = $derived(session.undoState);

  function add() {
    if (song) void libraryPage.addToPlaylist({ id: song.id, name: song.name });
  }

  // The song's theme: which one its slides follow, and whether any slide overrides it.
  let songTheme = $derived(doc ? songThemeState(doc) : null);
  let followed = $derived(songTheme?.theme ? (libraryPage.themes.find((t) => t.id === songTheme!.theme) ?? null) : null);
  let themeChoice = $state("");
  $effect(() => {
    themeChoice = songTheme?.theme ?? "";
  });
  async function applyTheme(id: string) {
    if (!song || !id || id === songTheme?.theme) return;
    const t = libraryPage.themes.find((x) => x.id === id);
    const ok = await shell.confirm(
      `Apply “${t?.name ?? "the theme"}”`,
      `Every slide of “${song.name}” follows “${t?.name ?? "the theme"}” from now on: its look changes when the theme does. The words stay as they are.`,
      "Apply theme",
      false,
    );
    if (!ok) {
      themeChoice = songTheme?.theme ?? "";
      return;
    }
    if (await libraryPage.applyTheme(song.id, id)) doc = await conn.rest.presentation(song.id).catch(() => doc);
  }
  /** Unlinked slides (an imported song) follow the theme that fits them, looking the same. */
  async function linkKeepingLook() {
    if (!song || !songTheme) return;
    const unlinked = songTheme.total - songTheme.linked;
    const n = await libraryPage.linkKeepingLook(song.id);
    if (n === null) return;
    doc = await conn.rest.presentation(song.id).catch(() => doc);
    conn.pushNotice(
      "info",
      n === 0
        ? `No slide of “${song.name}” matches a theme closely enough to follow it without changing its look.`
        : `${plural(n, "slide")} of “${song.name}” now ${n === 1 ? "follows" : "follow"} a theme, looking as before${n < unlinked ? `; ${plural(unlinked - n, "slide")} stay as they are` : ""}.`,
    );
  }
  /** A passage made on the Bible page whose theme changed after it was edited: split it
   * again for the theme (the server does it on its own for passages nobody edited). */
  let stalePassage = $derived(!!doc?.scripture?.generated?.stale);
  async function resplit() {
    if (!song) return;
    const ok = await shell.confirm(
      `Split “${song.name}” again`,
      "Its theme changed after the passage was edited. Splitting it again fits the verses to the theme as it is now; the changes made to the passage are replaced.",
      "Split again",
      false,
    );
    if (!ok) return;
    try {
      const r = await new ScriptureClient(conn.rest).resplit(song.id, true);
      doc = await conn.rest.presentation(song.id).catch(() => doc);
      conn.pushNotice("info", `“${song.name}” is split again: ${plural(r.slideCount, "slide")}.`);
    } catch (e) {
      conn.pushNotice("error", `Could not split the passage again: ${e instanceof Error ? e.message : String(e)}`);
    }
  }
  async function resetSong() {
    if (!song || !songTheme) return;
    if (await libraryPage.resetSongToTheme(song.id, songTheme.overridden)) doc = await conn.rest.presentation(song.id).catch(() => doc);
  }

  // Deleting a theme songs follow: move them to another theme, or let them keep their look.
  let deleting = $state(false);
  let reassignTo = $state("");
  let otherThemes = $derived(theme ? libraryPage.themes.filter((t) => t.id !== theme!.id) : []);
  $effect(() => {
    void theme?.id;
    deleting = false;
    reassignTo = otherThemes[0]?.id ?? "";
  });
  async function askDelete() {
    if (!theme) return;
    if (!theme.usedBy) await libraryPage.deleteTheme(theme);
    else deleting = true;
  }
  async function deleteFollowed(opts: { reassign?: string; unlink?: boolean }) {
    if (!theme) return;
    if (await libraryPage.deleteTheme(theme, opts)) deleting = false;
  }

  function slideThumb(slideId: string): string | null {
    return song ? conn.rest.thumbnailUrl(song.id, slideId, 320) : null;
  }
</script>

<main class="item" aria-label="Item">
  {#if kind}
    <div class="head">
      <span class="kind">{kind}</span>
      <h2>{name}</h2>
      <span class="meta" title={meta}>{meta}</span>
      <PresenceBadge kind={presKind} id={presId} />
      {#if editing}
        <span class="saved">Saved as you edit</span>
        <span class="history" role="toolbar" aria-label="History">
          <Button size="sm" variant="ghost" icon={Undo2} disabled={!undo.undoLabel || undo.busy} title={undo.undoLabel ? `Undo ${undo.undoLabel}` : "Nothing to undo"} onclick={() => session.undo()}>Undo</Button>
          <Button size="sm" variant="ghost" icon={Redo2} disabled={!undo.redoLabel || undo.busy} title={undo.redoLabel ? `Redo ${undo.redoLabel}` : "Nothing to redo"} onclick={() => session.redo()}>Redo</Button>
        </span>
      {/if}
      {#if !trashed}
        <Segmented size="sm" label="Mode" options={modes} bind:value={mode} onchange={setMode} />
      {/if}
      {#if song || foreign}
        <Button size="sm" variant="outline" class="add" icon={ListPlus} disabled={!song || !target} title={foreign ? "Only songs of the collection this box shows can be added" : undefined} onclick={add}>
          Add to {target?.name ?? "playlist"}
        </Button>
      {:else if trashed}
        <Button size="sm" variant="primary" icon={RotateCcw} disabled={readOnly} onclick={() => libraryPage.restore(trashed!)}>Restore</Button>
      {/if}
    </div>
  {/if}

  {#if presId}
    <LockBanner
      kind={presKind}
      id={presId}
      noun={theme ? "theme" : prop ? "prop" : "song"}
      canTake={!!song}
      {kept}
      oncopy={() => libraryPage.editState && void copyTyping(libraryPage.editState.takeLockedText())}
      ontaken={() => song && libraryPage.mode !== "edit" && libraryPage.startEdit(song.id)}
    />
  {/if}
  <div class="body">
    {#if editing && libraryPage.editState}
      {#await songEdit() then m}<m.default {conn} st={libraryPage.editState} />{:catch e}<ChunkError error={e} what="The song editor" />{/await}
    {:else if song}
      <div class="song">
        <div class="arr">
          <span class="cap">Groups</span>
          {#each arrangement as a, i (i)}
            <span class="chip" style:background={a.color ?? "var(--group-neutral)"}>{a.name}</span>
          {/each}
        </div>
        {#if doc && stalePassage}
          <div class="song-theme" data-testid="passage-stale">
            <span class="cap">Passage</span>
            <span class="follows">Its theme changed after it was edited, so it kept its slides.</span>
            <Button size="sm" variant="outline" icon={RotateCcw} disabled={readOnly || !!lockedBy} title={lockedBy ? "Someone else is editing it: take over first" : undefined} onclick={resplit}>Split again for the theme</Button>
          </div>
        {/if}
        {#if doc}
          <div class="song-theme" data-testid="song-theme">
            <span class="cap">Theme</span>
            <select bind:value={themeChoice} disabled={readOnly || libraryPage.busy || !libraryPage.themes.length} onchange={() => applyTheme(themeChoice)} aria-label="Theme">
              {#if !songTheme?.theme}<option value="">{songTheme?.madeFrom ? `Made from “${songTheme.madeFrom}” (a copy)` : "No theme"}</option>{/if}
              {#each libraryPage.themes as t (t.id)}
                <option value={t.id}>{t.name}</option>
              {/each}
            </select>
            {#if songTheme && songTheme.linked < songTheme.total && libraryPage.themes.length}
              <Button size="sm" variant="outline" icon={Link2} disabled={readOnly || libraryPage.busy} onclick={linkKeepingLook} title="Slides that don't follow a theme follow the one they were made from (or the closest), keeping exactly how they look: later theme changes restyle them" data-testid="link-keep-look">Link to theme, keeping the look</Button>
            {/if}
            {#if songTheme?.theme}
              <span class="follows">{songTheme.linked === songTheme.total ? "Every slide follows it" : `${songTheme.linked} of ${plural(songTheme.total, "slide")} follow it`}</span>
              {#if songTheme.overridden.length}
                <span class="over" title="These slides change something the theme sets">{plural(songTheme.overridden.length, "slide")} with changes</span>
                <Button size="sm" variant="outline" icon={RotateCcw} disabled={readOnly || libraryPage.busy} onclick={resetSong}>Reset to theme</Button>
              {/if}
              {#if followed}
                <Button size="sm" variant="ghost" onclick={() => {
                    libraryPage.go({ kind: "themes" });
                    libraryPage.select(followed!.id);
                  }}>Open theme →</Button>
              {/if}
            {/if}
          </div>
        {/if}
        {#if docError}
          <p class="muted">Could not load the song: {docError}</p>
        {:else if !doc}
          <p class="muted">Loading…</p>
        {:else}
          <div class="slides">
            {#each slides as s (s.id)}
              <SlideTile src={slideThumb(s.id)} number={s.index + 1} label={s.sectionStart ? s.sectionName : s.label || s.sectionName} groupColor={s.color} disabled={!s.enabled} />
            {/each}
          </div>
        {/if}
      </div>
    {:else if foreign}
      <EmptyState icon={Music} title="In another collection">
        “{foreign.title}” is in {foreign.collectionName}. This box shows {libraryPage.collections.find((c) => c.active)?.collection.name ?? "another collection"}; switch the collection
        (the card at the top left) to use it here.
      </EmptyState>
    {:else if theme}
      <div class="theme">
        <div class="tactions" role="toolbar" aria-label="Theme">
          <Button size="sm" variant="primary" icon={Pencil} disabled={readOnly} onclick={() => libraryPage.openTheme(theme!.id)}>Edit in slide editor</Button>
          <Button size="sm" variant="outline" icon={Copy} disabled={readOnly || libraryPage.busy} onclick={() => libraryPage.duplicateTheme(theme!)}>Duplicate</Button>
          <Button size="sm" variant="outline" disabled={readOnly || libraryPage.busy || !!lockedBy} title={lockedTitle} onclick={() => libraryPage.renameTheme(theme!)}>Rename</Button>
          <Button size="sm" variant="ghost" icon={Trash2} disabled={readOnly || libraryPage.busy || !!lockedBy} title={lockedTitle} onclick={askDelete}>Delete…</Button>
        </div>
        <ThemeSecondLanguage themeId={theme.id} name={theme.name} disabled={readOnly || !!lockedBy} />
        {#if deleting}
          <div class="delete-box" role="group" aria-label="Delete the theme" data-testid="theme-delete">
            <p><strong>{plural(theme.usedBy ?? 0, "song")} follow “{theme.name}”.</strong> Pick the theme they follow instead, or let them keep how they look now as plain copies.</p>
            <div class="row">
              <label class="pick">
                <span>Move them to</span>
                <select bind:value={reassignTo} disabled={!otherThemes.length} aria-label="Move them to">
                  {#each otherThemes as o (o.id)}<option value={o.id}>{o.name}</option>{/each}
                </select>
              </label>
              <Button size="sm" variant="danger" disabled={!reassignTo || libraryPage.busy} onclick={() => deleteFollowed({ reassign: reassignTo })}>Move and delete</Button>
            </div>
            <div class="row">
              <Button size="sm" variant="outline" disabled={libraryPage.busy} onclick={() => deleteFollowed({ unlink: true })}>Keep their look and delete</Button>
              <Button size="sm" variant="ghost" onclick={() => (deleting = false)}>Cancel</Button>
            </div>
          </div>
        {/if}
        <span class="cap">Template slides</span>
        <div class="slides">
          {#each theme.slides as t, i (t.id)}
            <SlideTile src={t.slide ? conn.rest.themeThumbnailUrl(theme.id, t.slide, 320, String(libraryPage.themesVersion)) : null} number={i + 1} label={t.name} groupColor={null} />
          {/each}
        </div>
        <span class="cap used-cap">Used by</span>
        <div class="used" data-testid="used-by">
          {#if usedByError}
            <p class="muted">Couldn't tell: {usedByError}</p>
          {:else if !usedBy}
            <p class="muted">Looking…</p>
          {:else if !usedBy.length}
            <p class="muted">No song follows this theme yet. Pick it as a song's theme to use it.</p>
          {:else}
            {#each usedBy as u (u.kind + u.id)}
              <div class="kv"><span>{USE_KIND[u.kind] ?? u.kind}</span><span title={u.name}>{u.name || "Untitled"}</span></div>
            {/each}
          {/if}
        </div>
        <p class="muted"><Palette size={13} aria-hidden="true" /> Songs follow their theme: a change here restyles every song that uses it. A slide can still change a box or a colour on its own (“Reset to theme” undoes that).</p>
      </div>
    {:else if prop || media}
      <div class="asset">
        <div class="stage">
          <div class="frame">
            {#if prop}
              <img src={conn.rest.propThumbnailUrl(prop.id, 960, prop.revision)} alt="" draggable="false" />
            {:else if mediaSrc}
              <img src={mediaSrc} alt="" draggable="false" />
            {/if}
            {#if media && media.availability !== "online"}<span class="flag">{media.availability === "missing" ? "Missing" : "Copying to this box"}</span>{/if}
          </div>
        </div>
        <aside class="props" aria-label="Properties">
          <h3 class="cap">Properties</h3>
          {#if prop}
            <div class="kv"><span>Name</span><span>{prop.name ?? "Untitled"}</span></div>
            <div class="kv"><span>Layer</span><span>Props</span></div>
            {#if prop.size}<div class="kv"><span>Size</span><span>{prop.size.width}×{prop.size.height}</span></div>{/if}
          {:else if media}
            <div class="kv"><span>File</span><span>{media.asset.name}</span></div>
            <div class="kv"><span>Kind</span><span>{mediaKind(media.asset) === "video" ? "Video" : mediaKind(media.asset) === "audio" ? "Sound" : "Image"}</span></div>
            {#if media.asset.probe?.width}<div class="kv"><span>Size</span><span>{media.asset.probe.width}×{media.asset.probe.height}</span></div>{/if}
            {#if media.asset.probe?.duration}<div class="kv"><span>Duration</span><span>{formatDuration(media.asset.probe.duration)}</span></div>{/if}
            {#if media.asset.probe?.codec}<div class="kv"><span>Codec</span><span>{media.asset.probe.codec}</span></div>{/if}
            <div class="kv">
              <span>On this box</span>
              <span class:ok={media.availability === "online"} class:warn={media.availability !== "online"}>{media.availability === "online" ? "Online" : media.availability === "missing" ? "Missing" : "Copying"}</span>
            </div>
            {#if media.problem}<p class="muted">{media.problem}</p>{/if}
            {#if media.availability === "missing"}
              <Button size="sm" variant="primary" icon={Link2} disabled={readOnly} onclick={() => (libraryPage.relinkFor = media)}>Relink…</Button>
            {/if}
            {#if mediaKind(media.asset) === "video" || mediaKind(media.asset) === "audio"}<MediaPrepCard {media} {readOnly} />{/if}
          {/if}
          <h3 class="cap used-cap">Used by</h3>
          <div class="used" data-testid="used-by">
            {#if usedByError}
              <p class="muted">Couldn't tell: {usedByError}</p>
            {:else if !usedBy}
              <p class="muted">Looking…</p>
            {:else if !usedBy.length}
              <p class="muted">Nothing in this collection uses it.</p>
            {:else}
              {#each usedBy as u (u.kind + u.id)}
                <div class="kv"><span>{USE_KIND[u.kind] ?? u.kind}</span><span title={u.name}>{u.name || "Untitled"}</span></div>
              {/each}
            {/if}
          </div>
        </aside>
      </div>
    {:else if trashed}
      <div class="deleted">
        <div class="kv"><span>Kind</span><span>{KIND[trashed.kind] ?? trashed.kind}</span></div>
        <div class="kv"><span>Deleted</span><span>{shortTime(trashed.deletedAt)}{trashed.by.client ? ` by ${trashed.by.client}` : ""}</span></div>
        <div class="kv"><span>Collection</span><span>{libraryPage.collections.find((c) => c.collection.id === trashed!.collection)?.collection.name ?? "—"}</span></div>
        <div class="kv"><span>Gone for good in</span><span>{plural(daysLeft(trashed.expiresAt), "day")}</span></div>
        {#if trashed.subtree}<div class="kv"><span>With it</span><span>{plural(trashed.subtree, "item")}</span></div>{/if}
      </div>
    {:else}
      <EmptyState icon={Music} title={view.kind === "trash" ? "Pick a deleted item" : "Pick something to look at"}>
        {view.kind === "songs" ? "Click a song to see its slides. Double-click or Enter adds it to the playlist in “Add to”." : view.kind === "trash" ? "Restore brings it back where it was." : "Click an item to see it here."}
      </EmptyState>
    {/if}
  </div>
</main>

<style>
  .item {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface-app);
  }
  .head {
    flex: none;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 16px;
    border-bottom: 1px solid var(--surface-raised);
  }
  .kind {
    padding: 2px 8px;
    border-radius: 10px;
    background: var(--surface-raised);
    font-size: 10.5px;
    font-weight: 800;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-2);
    white-space: nowrap;
  }
  h2 {
    margin: 0;
    flex: 0 1 auto;
    min-width: 80px;
    max-width: 50%;
    font-size: 18px;
    font-weight: 800;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .meta {
    flex: 1 1 0;
    color: var(--text-meta);
    font-size: var(--fs-sm);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .saved {
    font-size: var(--fs-xs);
    color: var(--text-meta);
    white-space: nowrap;
  }
  .history {
    display: flex;
    gap: 2px;
  }
  .head :global(.add) {
    border-color: var(--accent);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    overflow: hidden;
  }
  .body > :global(.ui-empty) {
    margin: auto;
  }
  .cap {
    margin: 0;
    font-size: 10.5px;
    font-weight: 700;
    letter-spacing: var(--label-tracking);
    text-transform: uppercase;
    color: var(--text-label);
  }
  .song,
  .theme {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .arr {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .chip {
    padding: 2px 9px;
    border-radius: 10px;
    font-size: 11.5px;
    font-weight: 700;
    color: var(--text-strong);
  }
  .slides {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
    gap: 12px;
  }
  .song-theme,
  .tactions,
  .delete-box .row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .song-theme select,
  .delete-box select {
    min-width: 180px;
    height: 28px;
    padding: 0 8px;
    border-radius: 6px;
    border: 1px solid var(--line-strong);
    background: var(--surface-sunk);
    color: var(--text-strong);
    font: inherit;
    font-size: var(--fs-sm);
  }
  .follows {
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .over {
    padding: 2px 8px;
    border-radius: 10px;
    background: color-mix(in srgb, var(--live) 18%, transparent);
    color: var(--live);
    font-size: 11.5px;
    font-weight: 700;
  }
  .delete-box {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border-radius: 8px;
    border: 1px solid color-mix(in srgb, var(--danger, #e5484d) 55%, transparent);
    background: color-mix(in srgb, var(--danger, #e5484d) 8%, var(--surface-sunk));
  }
  .delete-box p {
    margin: 0;
    font-size: 12.5px;
  }
  .pick {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-sm);
    color: var(--text-label);
  }
  .theme .used {
    max-width: 520px;
  }
  .theme > .muted {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .muted {
    margin: 0;
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .asset {
    flex: 1;
    min-width: 0;
    display: flex;
  }
  .stage {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    margin: 14px 16px;
    border-radius: 10px;
    background: repeating-conic-gradient(#17181d 0% 25%, #121317 0% 50%) 50% / 24px 24px;
  }
  .frame {
    position: relative;
    width: 92%;
    max-width: 980px;
    aspect-ratio: 16 / 9;
    overflow: hidden;
    box-shadow:
      0 10px 40px rgba(0, 0, 0, 0.6),
      0 0 0 1px var(--line-strong);
  }
  .frame img {
    width: 100%;
    height: 100%;
    object-fit: contain;
    display: block;
  }
  .flag {
    position: absolute;
    left: 12px;
    top: 12px;
    padding: 3px 9px;
    border-radius: 4px;
    background: var(--live);
    color: var(--live-ink);
    font-size: var(--fs-sm);
    font-weight: 700;
  }
  .props {
    flex: 0 0 300px;
    overflow-y: auto;
    padding: 14px;
    border-left: 1px solid var(--surface-raised);
    background: var(--surface-sunk);
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .kv {
    display: grid;
    grid-template-columns: 110px minmax(0, 1fr);
    gap: 10px;
    align-items: center;
    min-height: 26px;
    font-size: 12.5px;
  }
  .kv > span:first-child {
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  .kv > span:last-child {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .used-cap {
    margin-top: 8px;
  }
  .used {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .used .muted {
    margin: 0;
  }
  .ok {
    color: var(--ok-text);
  }
  .warn {
    color: var(--live);
  }
  .deleted {
    padding: 16px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 420px;
  }
</style>
