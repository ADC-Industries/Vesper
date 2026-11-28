<script lang="ts">
  // Under the Show page's layer buttons: live captions (docs/CAPTIONS-PLAN.md). On / off, what
  // the engine is doing, the newest line as a live preview, clearing the lines off the screens
  // and a way to Settings ▸ Captions. Hidden for a server without captions.
  import Captions from "lucide-svelte/icons/captions";
  import X from "lucide-svelte/icons/x";
  import Settings from "lucide-svelte/icons/settings";
  import { session } from "../../state/session.svelte";
  import { shell } from "../../state/shell.svelte";
  import { useLive } from "../../live";
  import { captionStatus, lastLine } from "./captions";

  const live = useLive();
  const captions = live.captions;

  let busy = $state(false);
  let on = $derived(!!$captions?.enabled);
  let status = $derived(captionStatus($captions));
  let line = $derived(lastLine($captions));

  async function toggle() {
    const conn = session.conn;
    if (!conn || busy) return;
    busy = true;
    try {
      const cfg = await conn.rest.captionsConfig();
      await conn.rest.saveCaptionsConfig({ ...cfg, enabled: !cfg.enabled });
    } catch (e) {
      conn.pushNotice("error", `Captions: ${e instanceof Error ? e.message : String(e)}`);
    } finally {
      busy = false;
    }
  }

  function settings() {
    shell.openSettings("Captions");
  }
</script>

{#if $captions}
  <div class="cc" class:on class:bad={status.cls === "bad"} data-testid="captions-strip">
    <div class="row">
      <button
        class="sw"
        class:on
        role="switch"
        aria-checked={on}
        disabled={busy}
        onclick={toggle}
        title={on ? "Captions on: click to switch them off" : "Switch live captions on"}
        data-testid="captions-toggle"
      >
        <Captions size={14} aria-hidden="true" /><span>Captions</span>
      </button>
      <span class="what {status.cls}" title={[status.text, $captions.device, $captions.note].filter(Boolean).join(" · ")}>{status.text}</span>
      {#if on && $captions.lines.length}
        <button class="x" onclick={() => session.run({ type: "clearLayer", layer: "captions" })} title="Take the lines off the screens (captions keep running)" aria-label="Clear captions"><X size={13} /></button>
      {/if}
      <button class="x" onclick={settings} title="Captions settings" aria-label="Captions settings"><Settings size={13} /></button>
    </div>
    {#if on && line}
      <p class="line" aria-live="polite" data-testid="captions-line">{line}</p>
    {/if}
  </div>
{/if}

<style>
  .cc {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    min-width: 0;
    padding: 3px 4px 3px 4px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text-2);
    font-size: 11.5px;
  }
  .cc.on {
    border-color: color-mix(in srgb, var(--ok) 45%, var(--line));
  }
  .cc.on.bad {
    border-color: var(--danger);
    background: var(--danger-soft);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .sw {
    flex: none;
    display: flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding: 0 7px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r);
    background: var(--surface-panel);
    color: var(--text-2);
    font-size: 11.5px;
    font-weight: 600;
  }
  .sw.on {
    border-color: var(--ok);
    background: var(--ok-bg);
    color: var(--text);
  }
  .what {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-label);
  }
  .what.bad {
    color: var(--danger-text);
  }
  .x {
    flex: none;
    display: flex;
    padding: 3px;
    border: 0;
    border-radius: var(--r);
    background: none;
    color: var(--text-2);
  }
  .x:hover {
    color: var(--text);
    background: var(--surface-overlay);
  }
  .line {
    margin: 0;
    padding: 2px 4px 1px;
    color: var(--text);
    font-size: 12px;
    line-height: 1.35;
    overflow: hidden;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
  }
</style>
