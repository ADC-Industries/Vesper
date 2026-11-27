<script lang="ts">
  // The bottom bin's Audio tab, left side: audio bins and audio playlists (the one playing is
  // marked). Right-click a playlist for New / Rename / Delete; audio files dragged from a bin
  // drop onto a playlist here.
  import ListMusic from "lucide-svelte/icons/list-music";
  import Music from "lucide-svelte/icons/music";
  import Volume2 from "lucide-svelte/icons/volume-2";
  import { session } from "../../state/session.svelte";
  import { flattenBins, mediaState } from "../../state/media.svelte";
  import { shell } from "../../state/shell.svelte";
  import { useLive } from "../../live";
  import { drag } from "../../dnd.svelte";
  import { audioPlaylists, type AudioSource } from "./audio";

  const live = useLive();
  const audio = live.audio;
  const audioLayer = live.audioLayer;

  let bins = $derived(flattenBins(mediaState.mediaBins).filter((b) => b.browser === "audio" && b.id));
  let playlists = $derived(audioPlaylists(session.playlistTree));
  let src = $derived(mediaState.audioSource);
  let ctx = $derived($audio?.playlist ?? null);

  /** The bin or playlist the audio layer is playing from. */
  function isPlayingSource(s: AudioSource): boolean {
    const l = $audioLayer;
    if (!l) return false;
    if (s.kind === "playlist") return ctx?.playlist === s.id;
    return !ctx && s.id === src?.id && !!mediaState.audioBin?.items.some((it) => it.asset === l.asset);
  }

  function same(a: AudioSource | null, b: AudioSource): boolean {
    return !!a && a.kind === b.kind && a.id === b.id;
  }

  function playlistMenu(e: MouseEvent, p: { id: string; name: string }) {
    e.preventDefault();
    e.stopPropagation();
    shell.popover = null;
    shell.contextMenu = {
      x: e.clientX,
      y: e.clientY,
      items: [
        { type: "item", id: "new", label: "New Audio Playlist…", run: () => void mediaState.newAudioPlaylist() },
        { type: "separator" },
        { type: "item", id: "rename", label: "Rename…", run: () => void mediaState.renameAudioPlaylist(p.id, p.name) },
        { type: "item", id: "delete", label: "Delete", run: () => void session.deletePlaylist(p.id, p.name, false) },
      ],
    };
  }
</script>

<ul class="list" aria-label="Audio bins and audio playlists">
  {#each bins as b (b.id)}
    {@const s: AudioSource = { kind: "bin", id: b.id! }}
    <li>
      <button class="row" class:sel={same(src, s)} onclick={() => mediaState.selectAudioSource(s)} title={`Audio bin: ${b.name}`}>
        <Music size={15} />
        <span class="name">{b.name}</span>
        {#if isPlayingSource(s)}<span class="now" title="Playing"><Volume2 size={13} /></span>{/if}
        <span class="detail">{b.itemCount}</span>
      </button>
    </li>
  {/each}
  {#each playlists as p (p.id)}
    {@const s: AudioSource = { kind: "playlist", id: p.id }}
    <li>
      <button
        class="row"
        class:sel={same(src, s)}
        class:drop={drag.overPlaylist === p.id}
        onclick={() => !drag.consumeClick() && mediaState.selectAudioSource(s)}
        oncontextmenu={(e) => playlistMenu(e, p)}
        title={`Audio playlist: ${p.label} (right-click to rename or delete)`}
        data-drop-audio-playlist={p.id}
      >
        <ListMusic size={15} />
        <span class="name">{p.label}</span>
        {#if isPlayingSource(s)}<span class="now" title="Playing"><Volume2 size={13} /></span>{/if}
        <span class="detail">{p.itemCount}</span>
      </button>
    </li>
  {/each}
  {#if !bins.length && !playlists.length}
    <li class="none">No audio bins or playlists. Press + to make a playlist.</li>
  {/if}
</ul>

<style>
  .list {
    list-style: none;
    margin: 0;
    padding: 0 6px;
  }
  .row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 8px;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-2);
    font-size: 12px;
    text-align: left;
  }
  .row:hover:not(.sel) {
    background: var(--surface-raised);
  }
  .row.sel {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .row.drop {
    outline: 1px solid var(--accent-ring);
    background: var(--accent-sel);
  }
  .row :global(svg) {
    flex: none;
    color: var(--text-label);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .detail {
    color: var(--text-meta);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
  }
  .now {
    display: grid;
    place-items: center;
  }
  .row .now :global(svg) {
    color: var(--live);
  }
  .none {
    padding: 6px 10px;
    color: var(--text-label);
    font-size: 12px;
  }
</style>
