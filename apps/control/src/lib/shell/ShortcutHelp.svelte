<script lang="ts">
  // The "?" dialog: the shortcut table (shortcuts.ts), grouped as in the canvas.
  import Dialog from "../ui/Dialog.svelte";
  import { isTauri } from "../tauri";
  import { shell } from "../state/shell.svelte";
  import { SHORTCUT_GROUPS, rowsOf } from "./shortcuts";

  const desktop = isTauri();
</script>

<Dialog title="Keyboard shortcuts" onclose={() => (shell.showHelp = false)} width={600}>
  <div class="help">
    {#each SHORTCUT_GROUPS as g (g)}
      <h3>{g}</h3>
      {#each rowsOf(g) as r (r.action)}
        <div class="row" class:dim={r.desktopOnly && !desktop}>
          <span class="what">{r.action}{#if r.note}<span class="note">{r.note}</span>{/if}</span>
          <span class="keys">
            {#each r.keys as k, i (k)}{#if i > 0}<span class="or">or</span>{/if}<kbd>{k}</kbd>{/each}
          </span>
        </div>
      {/each}
    {/each}
    <p class="foot">Shortcuts are left to a search or text box while you type in it. Keys are matched by their place on the keyboard, so they work on every layout.</p>
  </div>
</Dialog>

<style>
  .help {
    max-height: min(70vh, 720px);
    overflow-y: auto;
    margin: -4px -6px;
    padding: 0 6px;
  }
  h3 {
    margin: 14px 0 6px;
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: var(--label-tracking);
    text-transform: uppercase;
    color: var(--text-label);
  }
  h3:first-child {
    margin-top: 0;
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 5px 0;
    border-bottom: 1px solid var(--surface-raised);
  }
  .row.dim {
    opacity: 0.6;
  }
  .what {
    display: flex;
    flex-direction: column;
  }
  .note {
    color: var(--text-meta);
    font-size: var(--fs-xs);
  }
  .keys {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }
  kbd {
    font-family: inherit;
    font-size: var(--fs-sm);
    padding: 2px 8px;
    border-radius: 4px;
    border: 1px solid var(--line-strong);
    background: var(--surface-raised);
    color: var(--text);
  }
  .or {
    color: var(--text-meta);
    font-size: var(--fs-xs);
  }
  .foot {
    margin: 14px 0 0;
    color: var(--text-meta);
    font-size: var(--fs-sm);
  }
</style>
