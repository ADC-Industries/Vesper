<script lang="ts">
  // The inspector panels' shared look (the canvas's `.ed-sec` sections, fields and toggles), for
  // the Edit page's inspector and the Library's song Edit: wrap the panels in it.
  import type { Snippet } from "svelte";

  let { children }: { children: Snippet } = $props();
</script>

<div class="insp-panel">{@render children()}</div>

<style>
  .insp-panel {
    font-size: 13px;
    color: var(--text);
  }
  .insp-panel :global(.sec) {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border-bottom: 1px solid var(--line);
  }
  .insp-panel :global(.sec:last-child) {
    border-bottom: 0;
  }
  .insp-panel :global(h4) {
    margin: 0;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text);
  }
  .insp-panel :global(.label) {
    font-size: var(--fs-label, 11px);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-label);
  }
  .insp-panel :global(.check h4) {
    margin: 0;
  }
  .insp-panel :global(.check),
  .insp-panel :global(.radio) {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  .insp-panel :global(.check) {
    font-weight: 600;
  }
  .insp-panel :global(.check.en) {
    font-weight: 400;
  }
  .insp-panel :global(.radio) {
    margin-right: 14px;
  }
  .insp-panel :global(input[type="checkbox"]),
  .insp-panel :global(input[type="radio"]) {
    width: 16px;
    height: 16px;
    margin: 0;
    accent-color: var(--accent);
  }
  .insp-panel :global(.line) {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .insp-panel :global(.line.between) {
    justify-content: space-between;
  }
  .insp-panel :global(.line.between h4) {
    margin: 0;
  }
  .insp-panel :global(.lab) {
    color: var(--text-label);
    font-size: 12px;
    min-width: 52px;
  }
  .insp-panel :global(.wide) {
    width: 100%;
  }
  .insp-panel :global(select),
  .insp-panel :global(input[type="text"]),
  .insp-panel :global(input[type="number"]:not(.num input)),
  .insp-panel :global(textarea) {
    height: 30px;
    padding: 0 8px;
    background: var(--surface-raised);
    border: 1px solid var(--line);
    border-radius: var(--r);
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
  }
  .insp-panel :global(select:disabled),
  .insp-panel :global(textarea:disabled) {
    opacity: 0.45;
  }
  .insp-panel :global(textarea) {
    height: auto;
    padding: 6px 8px;
    line-height: 1.45;
    user-select: text;
  }
  .insp-panel :global(.notes) {
    width: 100%;
    resize: vertical;
  }
  .insp-panel :global(.grow) {
    flex: 1;
    min-width: 0;
  }
  .insp-panel :global(.note) {
    margin: 0;
    color: var(--text-label);
    font-size: 11.5px;
    line-height: 1.45;
  }
  .insp-panel :global(.cur) {
    margin: 0;
    color: var(--text-meta);
    font-size: 12px;
  }
  .insp-panel :global(.form) {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr);
    gap: 8px;
    align-items: center;
  }
  .insp-panel :global(.form label) {
    text-align: right;
    color: var(--text-label);
    font-size: 12px;
  }
  .insp-panel :global(.form input) {
    width: 100%;
    min-width: 0;
  }
  .insp-panel :global(.geo) {
    display: grid;
    grid-template-columns: 14px auto 14px auto auto;
    gap: 8px 6px;
    align-items: center;
    color: var(--text-label);
  }
  .insp-panel :global(.geo .lockar) {
    grid-column: 5;
    grid-row: 1 / 3;
  }
  .insp-panel :global(.lockar),
  .insp-panel :global(.tg) {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text-label);
  }
  .insp-panel :global(.lockar.on),
  .insp-panel :global(.tg.on) {
    color: var(--text-strong);
    background: var(--accent-sel);
    border-color: var(--accent-ring);
  }
  .insp-panel :global(.grid2) {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 8px;
    align-items: center;
  }
  .insp-panel :global(.grid4) {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .insp-panel :global(.seg) {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--r);
    background: var(--surface-app);
    border: 1px solid var(--line);
  }
  .insp-panel :global(.seg button) {
    width: 30px;
    height: 26px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: var(--r-sm);
    background: transparent;
    color: var(--text-label);
  }
  .insp-panel :global(.seg button.on) {
    background: var(--accent-sel);
    color: var(--text-strong);
  }
  .insp-panel :global(.aligns) {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
  }
  .insp-panel :global(.aligns button) {
    width: 32px;
    height: 28px;
    display: grid;
    place-items: center;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
  }
  .insp-panel :global(.aligns button:hover:not(:disabled)) {
    background: var(--surface-overlay);
  }
  .insp-panel :global(.aligns button:disabled) {
    opacity: 0.45;
  }
  .insp-panel :global(.vsep) {
    width: 1px;
    height: 22px;
    margin: 0 4px;
    background: var(--line);
  }
  .insp-panel :global(.swatches) {
    display: flex;
    gap: 6px;
  }
  .insp-panel :global(.sw) {
    width: 20px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--line-strong);
    border-radius: 50%;
  }
  .insp-panel :global(.hk) {
    min-width: 120px;
    height: 30px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    text-align: left;
    padding: 0 8px;
  }
  .insp-panel :global(.hk.cap) {
    border-color: var(--accent-ring);
    color: var(--accent-link);
  }
  .insp-panel :global(.scope) {
    margin: 0;
    padding: 8px 12px;
    background: var(--accent-sel);
    color: var(--accent-link);
    font-size: 12px;
  }
  .insp-panel :global(.hint) {
    margin: 8px 12px 14px;
    color: var(--text-meta);
    font-size: 11.5px;
  }
  .insp-panel :global(.ename) {
    color: var(--text);
    font-size: 14px;
    font-weight: 700;
  }
  .insp-panel :global(.build) {
    opacity: 0.6;
  }
  .insp-panel :global(.pop-anchor) {
    position: relative;
  }
  .insp-panel :global(input[type="range"]) {
    width: 140px;
    accent-color: var(--accent);
  }
  .insp-panel :global(.btn2) {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
  }
  .insp-panel :global(.empty) {
    margin: 0;
    padding: 16px 14px;
    color: var(--text-label);
    line-height: 1.5;
  }
</style>
