<script lang="ts">
  // The Show page's status bar: one dot per screen (name, role, "off", output problems), then
  // the sound card's problem, then "Configure screens".
  import { useLive } from "../live";
  import { shell } from "../state/shell.svelte";
  import { audioProblem, screenStatuses } from "./statusBar";
  import { serverDetails } from "../state/details.svelte";
  import { detailsShort } from "./details";

  const live = useLive();
  const screens = live.screens;
  const enabled = live.screensEnabled;
  const audio = live.audio;

  let items = $derived(screenStatuses($screens, $enabled));
  // A configured sound card that isn't playing (lost, or never opened): the server retries it.
  let sound = $derived(audioProblem($audio));
  // This Vesper's version and uptime (G9), its CPU and GPU in the tooltip.
  $effect(() => serverDetails.use());
  let details = $derived(serverDetails.details ? detailsShort(serverDetails.details) : null);
</script>

<footer class="status-bar" aria-label="Screens">
  {#each items as s (s.id)}
    <span class="screen {s.level}" title={s.title}><span class="dot" aria-hidden="true"></span>{s.text}</span>
  {:else}
    <span class="screen off"><span class="dot" aria-hidden="true"></span>No screens</span>
  {/each}
  {#if sound}
    <span class="screen warn" title={sound.title} data-testid="audio-problem"><span class="dot" aria-hidden="true"></span>{sound.text}</span>
  {/if}
  <span class="fill"></span>
  {#if details}<span class="box" title={details.title} data-testid="server-details">{details.text}</span>{/if}
  {#if shell.available("screens")}
    <button class="link" onclick={() => shell.go("screens")}>Configure screens</button>
  {/if}
</footer>

<style>
  .status-bar {
    flex: none;
    display: flex;
    flex-wrap: wrap;
    gap: 4px 16px;
    align-items: center;
    padding: 5px 12px;
    background: var(--surface-app);
    border-top: 1px solid var(--line);
    color: var(--text-label);
    font-size: var(--fs-xs);
  }
  .screen {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--ok);
  }
  .off .dot {
    background: var(--off);
  }
  .warn .dot {
    background: var(--warn);
  }
  .error .dot {
    background: var(--danger);
  }
  .error {
    color: var(--danger-text);
  }
  .fill {
    flex: 1 1 10px;
  }
  .box {
    color: var(--text-label);
    white-space: nowrap;
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-link);
    font-size: var(--fs-xs);
  }
  .link:hover {
    color: var(--accent-link-hover);
    text-decoration: underline;
  }
</style>
