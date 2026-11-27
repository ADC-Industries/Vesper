<script lang="ts">
  import { screensState } from "../../state/screens.svelte";
  import { useLive } from "../../live";

  const live = useLive();
  const screens = live.screens;
</script>

<table>
  <thead>
    <tr><th>Screen</th><th>On</th><th>FPS</th><th>Render</th><th>Outputs</th></tr>
  </thead>
  <tbody>
    {#each $screens as s (s.id)}
      {@const target = s.frameRate.num / Math.max(1, s.frameRate.den)}
      {@const bad = s.outputs.filter((o) => o.status !== "ok").length}
      <tr class:sel={s.id === screensState.previewScreen}>
        <td class="sn" title={s.name}>{s.name}</td>
        <td>{#if s.enabled}<span class="on">●</span>{:else}<span class="dim">off</span>{/if}</td>
        <td class:warn={s.measuredFps < target * 0.9}>{s.measuredFps.toFixed(0)}<span class="dim">/{target.toFixed(0)}</span></td>
        <td>{s.lastRenderMs == null ? "–" : `${s.lastRenderMs.toFixed(1)} ms`}</td>
        <td title={s.outputs.map((o) => `${o.name}: ${o.status}${o.detail ? ` (${o.detail})` : ""}`).join("\n")}>
          {#if s.outputs.length === 0}
            <span class="dim">none</span>
          {:else if bad}
            <span class="warn">{bad} of {s.outputs.length} down</span>
          {:else}
            <span class="good">{s.outputs.length} ok</span>
          {/if}
        </td>
      </tr>
    {:else}
      <tr><td colspan="5" class="dim">No screens</td></tr>
    {/each}
  </tbody>
</table>

<style>
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
  }
  th {
    text-align: left;
    font-weight: 600;
    color: var(--text-label);
    padding: 2px 4px;
    font-size: 11px;
  }
  td {
    padding: 3px 4px;
    color: var(--text-2);
    white-space: nowrap;
  }
  tr.sel td {
    color: var(--text-strong);
  }
  td.sn {
    max-width: 130px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dim {
    color: var(--text-meta);
  }
  .warn {
    color: var(--warn);
  }
  .good {
    color: var(--ok-text);
  }
  .on {
    color: var(--ok);
  }
</style>
