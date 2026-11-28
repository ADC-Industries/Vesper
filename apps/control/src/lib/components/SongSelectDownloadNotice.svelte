<script lang="ts">
  // The SongSelect Downloads watcher's notice (state/songselectWatch.svelte.ts): a song that
  // just landed in the Downloads folder, "New SongSelect song: <title>". Import opens the
  // SongSelect import preview with the file; Not now forgets it. It never blocks anything and
  // never imports by itself. Bottom right, above the status bar.
  import Button from "../ui/Button.svelte";
  import Music from "lucide-svelte/icons/music";
  import { songSelectWatch } from "../state/songselectWatch.svelte";

  let song = $derived(songSelectWatch.found[0] ?? null);
  let more = $derived(Math.max(0, songSelectWatch.found.length - 1));
</script>

{#if song}
  <div class="notice" role="status" aria-live="polite" data-testid="songselect-download-notice">
    <Music size={18} />
    <div class="text">
      <strong>New SongSelect song: {song.title ?? song.name}</strong>
      <span class="meta">{song.name} in your Downloads folder{more ? ` · ${more} more` : ""}. Import it?</span>
    </div>
    <div class="actions">
      <Button size="sm" variant="primary" onclick={() => songSelectWatch.import(song!)}>Import…</Button>
      <Button size="sm" onclick={() => songSelectWatch.dismiss(song!)}>Not now</Button>
    </div>
  </div>
{/if}

<style>
  .notice {
    position: fixed;
    right: 16px;
    bottom: 44px;
    z-index: 70;
    display: flex;
    align-items: center;
    gap: 12px;
    max-width: 520px;
    padding: 10px 12px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r-card);
    background: var(--surface-raised);
    color: var(--text);
    box-shadow: var(--shadow-toast);
  }
  .text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .text strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
  .actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }
</style>
