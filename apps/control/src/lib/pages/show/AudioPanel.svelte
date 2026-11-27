<script lang="ts">
  // The bottom bin's Audio tab, right side: the selected audio bin's or audio playlist's files
  // (click to play on the Audio layer, right-click for Add to / Remove from Playlist, drag a
  // bin's file onto an audio playlist), shuffle / repeat and a filter. The transport and the
  // Audio layer's volume are in the right column.
  import ArrowDown from "lucide-svelte/icons/arrow-down";
  import Music from "lucide-svelte/icons/music";
  import Shuffle from "lucide-svelte/icons/shuffle";
  import Repeat from "lucide-svelte/icons/repeat";
  import CircleAlert from "lucide-svelte/icons/circle-alert";
  import ListFilter from "lucide-svelte/icons/list-filter";
  import { session } from "../../state/session.svelte";
  import { mediaState } from "../../state/media.svelte";
  import { shell } from "../../state/shell.svelte";
  import { useLive } from "../../live";
  import type { MenuItem } from "../../shell/menu";
  import { drag } from "../../dnd.svelte";
  import { audioPlaylists, binRows, durationLabel, isPlayingRow, playlistRows, type AudioRow } from "./audio";

  const live = useLive();
  const audio = live.audio;
  const audioLayer = live.audioLayer;

  let filter = $state("");
  let selectedRow = $state<string | null>(null);

  let playlists = $derived(audioPlaylists(session.playlistTree));
  let src = $derived(mediaState.audioSource);
  /** The list that belongs to the selection (a stale one is not shown while the next loads). */
  let rows = $derived.by((): AudioRow[] => {
    if (src?.kind === "bin" && mediaState.audioBin?.id === src.id) return binRows(mediaState.audioBin);
    if (src?.kind === "playlist" && mediaState.audioPlaylist?.id === src.id) return playlistRows(mediaState.audioPlaylist, mediaState.audioDurations);
    return [];
  });
  let q = $derived(filter.trim().toLowerCase());
  let shown = $derived(q ? rows.filter((r) => r.name.toLowerCase().includes(q)) : rows);
  let count = $derived(rows.filter((r) => r.asset).length);

  let layerState = $derived($audioLayer);
  let ctx = $derived($audio?.playlist ?? null);
  let playing = $derived(layerState?.playback.state === "playing");

  // ---- playing --------------------------------------------------------------------------------

  function play(r: AudioRow) {
    if (!r.asset || !src) return;
    if (src.kind === "playlist") void session.run({ type: "triggerPlaylistItem", playlist: src.id, item: r.id });
    else void session.run({ type: "triggerMedia", asset: r.asset, layer: "audio" });
  }

  function rowKey(e: KeyboardEvent, r: AudioRow) {
    if (e.key === "Enter") {
      e.preventDefault();
      play(r);
    }
  }

  // ---- menus ------------------------------------------------------------------------------------

  function openMenu(e: MouseEvent, items: MenuItem[]) {
    e.preventDefault();
    e.stopPropagation();
    shell.popover = null;
    shell.contextMenu = { x: e.clientX, y: e.clientY, items };
  }

  function rowMenu(e: MouseEvent, r: AudioRow) {
    selectedRow = r.id;
    const s = src;
    if (!s) return;
    const items: MenuItem[] = [{ type: "item", id: "play", label: "Play", disabled: !r.asset, run: () => play(r) }];
    if (s.kind === "bin" && r.asset) {
      const asset = r.asset;
      items.push({
        type: "item",
        id: "add",
        label: "Add to Playlist",
        disabled: !playlists.length,
        title: playlists.length ? undefined : "Make an audio playlist first (+ beside Playlists)",
        submenu: playlists.map((p) => ({
          type: "item" as const,
          id: `pl-${p.id}`,
          label: p.label,
          run: () => void mediaState.addAudioToPlaylist(p.id, [{ asset, name: r.name }]),
        })),
      });
    } else if (s.kind === "playlist") {
      items.push({ type: "separator" }, { type: "item", id: "remove", label: "Remove from Playlist", run: () => void mediaState.removeAudioItem(s.id, r.id) });
    }
    openMenu(e, items);
  }

  // ---- drag a bin's file onto an audio playlist ---------------------------------------------------

  function press(e: PointerEvent, r: AudioRow) {
    if (src?.kind !== "bin" || !r.asset) return;
    drag.press(
      e,
      { kind: "audio", id: r.asset, name: r.name },
      {
        onPlaylist: (pl, p) => void mediaState.addAudioToPlaylist(pl, [{ asset: p.id, name: p.name }]),
        onList: () => {},
        onSpring: () => {},
      },
    );
  }

  function setOption(o: { shuffle?: boolean; repeat?: boolean }) {
    void session.run({ type: "setAudioOptions", ...o });
  }
</script>

<div class="body">
  <div class="items">
    {#if shown.length}
      <ul class="files" aria-label="Audio files">
        {#each shown as r (r.id)}
          {@const now = !!src && isPlayingRow(r, src, layerState, ctx)}
          {@const label = now ? (playing ? "Playing" : "Paused") : "Play"}
          <li>
            <button
              class="file"
              class:sel={selectedRow === r.id}
              class:now
              class:off={!r.asset}
              class:missing={!r.online}
              onclick={() => {
                if (drag.consumeClick()) return;
                selectedRow = r.id;
                play(r);
              }}
              onkeydown={(e) => rowKey(e, r)}
              onpointerdown={(e) => press(e, r)}
              oncontextmenu={(e) => rowMenu(e, r)}
              title={r.asset ? `${r.name}: click to play on the Audio layer${src?.kind === "bin" ? ", drag onto an audio playlist to add" : ""}` : r.name}
              aria-label={r.asset ? `${label} ${r.name}` : r.name}
              aria-current={now ? "true" : undefined}
            >
              <span class="state">
                {#if !r.online}<CircleAlert size={13} />{:else if r.asset}{label}{/if}
              </span>
              <span class="name">{r.name}</span>
              {#if r.asset}<span class="dur">{durationLabel(now ? (layerState?.playback.duration ?? r.duration) : r.duration)}</span>{/if}
            </button>
          </li>
        {/each}
      </ul>
    {:else if rows.length}
      <p class="hint">Nothing matches “{filter}”.</p>
    {:else if src?.kind === "playlist"}
      <p class="hint">This playlist is empty. Drag audio files here from an audio bin, or right-click a file ▸ Add to Playlist.</p>
    {:else}
      <div class="tile-empty">
        <button class="tile small" disabled title="Add audio files to an audio bin on the server" aria-label="Import Audio"><ArrowDown size={38} strokeWidth={1.3} /></button>
        <span>Import Audio</span>
      </div>
    {/if}
  </div>

  <div class="bottom">
    <button
      class="ibtn"
      class:on={$audio?.shuffle}
      aria-pressed={$audio?.shuffle ?? false}
      onclick={() => setOption({ shuffle: !$audio?.shuffle })}
      title="Shuffle audio playlists"
      aria-label="Shuffle"
    >
      <Shuffle size={15} />
    </button>
    <button
      class="ibtn"
      class:on={$audio?.repeat}
      aria-pressed={$audio?.repeat ?? false}
      onclick={() => setOption({ repeat: !$audio?.repeat })}
      title="Repeat: start the playlist over after the last track"
      aria-label="Repeat"
    >
      <Repeat size={15} />
    </button>
    <span class="meta">{count} {count === 1 ? "track" : "tracks"}</span>
    <span class="grow"></span>
    <label class="filter">
      <ListFilter size={14} />
      <input type="search" placeholder="Filter" bind:value={filter} spellcheck="false" aria-label="Filter audio files" />
    </label>
  </div>
</div>

{#if drag.payload?.kind === "audio"}
  <div class="ghost" style:left={`${drag.x + 12}px`} style:top={`${drag.y + 8}px`} aria-hidden="true">
    <Music size={13} />{drag.payload.name}
  </div>
{/if}

<style>
  .body {
    flex: 1;
    min-height: 0;
    height: 100%;
    display: flex;
    flex-direction: column;
  }
  .items {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .files {
    list-style: none;
    margin: 0;
    padding: 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .file {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 12px;
    height: 30px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    font-size: 13px;
    text-align: left;
  }
  .file:hover:not(.now) {
    background: var(--surface-overlay);
  }
  .file.sel:not(.now) {
    outline: 1px solid var(--line-strong);
  }
  .file.now {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .file.off {
    color: var(--text-meta);
    font-style: italic;
  }
  .file.missing .name {
    color: var(--warn);
  }
  .state {
    flex: none;
    width: 52px;
    display: flex;
    align-items: center;
    color: var(--accent-link);
    font-size: 11px;
    font-weight: 600;
  }
  .file.missing .state {
    color: var(--warn);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dur {
    flex: none;
    color: var(--text-meta);
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  .hint {
    margin: 0;
    padding: 14px 16px;
    color: var(--text-label);
    font-size: 12.5px;
  }
  .tile.small {
    width: 54px;
    height: 54px;
  }
  .items :global(.tile-empty) {
    min-height: 0;
    padding: 10px;
  }
  .bottom {
    flex: none;
    height: 30px;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 8px;
    border-top: 1px solid var(--line);
  }
  .meta {
    margin-left: 6px;
    color: var(--text-meta);
    font-size: 12px;
  }
  .grow {
    flex: 1;
  }
  .bottom .filter {
    width: 200px;
    min-width: 60px;
  }
  .ghost {
    position: fixed;
    z-index: 95;
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 260px;
    padding: 4px 9px;
    border-radius: 3px;
    background: var(--accent-sel);
    border: 1px solid var(--accent-ring);
    color: var(--text-strong);
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    pointer-events: none;
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.5);
  }
</style>
