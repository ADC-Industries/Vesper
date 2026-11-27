<script lang="ts">
  // The right column's Timers tab: every timer with its live value, start / pause and reset.
  // Making and editing timers is the Timers page's.
  import Play from "lucide-svelte/icons/play";
  import Pause from "lucide-svelte/icons/pause";
  import RotateCcw from "lucide-svelte/icons/rotate-ccw";
  import type { TimerEntry } from "$protocol";
  import type { Connection } from "../../server";
  import { session } from "../../state/session.svelte";
  import { shell } from "../../state/shell.svelte";
  import { timersState } from "../../state/timers.svelte";
  import { isRunning, timerSetting, timerTone } from "../timers/timers";

  let { conn }: { conn: Connection } = $props();
  // svelte-ignore state_referenced_locally
  const values = conn.timerValues;

  function describe(t: TimerEntry): string {
    const k = t.kind ?? { type: "countdown" };
    if (k.type === "countdownToTime") return `Counts down to ${timerSetting(k)}`;
    if (k.type === "elapsed") return "Counts up";
    return `${timerSetting(k)} countdown`;
  }
</script>

<ul class="timers" data-testid="compact-timers">
  {#each timersState.timerDocs as t (t.id)}
    {@const v = $values[t.id]}
    {@const st = v?.state ?? "stopped"}
    {@const running = isRunning(st)}
    <li class="timer" data-state={st}>
      <div class="what">
        <span class="name" title={t.name}>{t.name || "Timer"}</span>
        <span class="desc">{describe(t)}</span>
      </div>
      <span class="value" class:running class:warn={t.kind?.type !== "elapsed" && timerTone(st, v?.currentSeconds, t.warningSeconds) === "warn"} class:over={st === "overrun" || st === "expired"}>{v?.display ?? "--:--"}</span>
      <button class="tb" onclick={() => session.run({ type: "timerReset", timer: t.id })} title="Reset" aria-label={`Reset ${t.name}`}><RotateCcw size={14} /></button>
      <button
        class="tb"
        class:on={running}
        onclick={() => session.run({ type: running ? "timerStop" : "timerStart", timer: t.id })}
        title={running ? "Pause" : "Start"}
        aria-label={`${running ? "Pause" : "Start"} ${t.name}`}
      >
        {#if running}<Pause size={14} fill="currentColor" />{:else}<Play size={14} fill="currentColor" />{/if}
      </button>
    </li>
  {:else}
    <li class="none">No timers yet.</li>
  {/each}
  <li class="more"><button class="link" onclick={() => shell.go("timers")}>Open the Timers page</button></li>
</ul>

<style>
  .timers {
    list-style: none;
    margin: 0;
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .timer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px 7px 10px;
    border: 1px solid var(--line);
    border-radius: var(--r-card);
    background: var(--surface-raised);
  }
  .what {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .desc {
    color: var(--text-meta);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .value {
    flex: none;
    font-size: 17px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .value.running {
    color: var(--info-text);
  }
  .value.over,
  .value.warn {
    color: var(--live);
  }
  .tb {
    flex: none;
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-panel);
    color: var(--text);
  }
  .tb:hover {
    background: var(--surface-overlay);
  }
  .tb.on {
    border-color: var(--accent-ring);
    background: var(--accent-sel);
  }
  .none {
    padding: 4px 2px;
    color: var(--text-label);
    font-size: 12px;
  }
  .more {
    padding: 2px;
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-link);
    font-size: 12px;
  }
  .link:hover {
    color: var(--accent-link-hover);
    text-decoration: underline;
  }
</style>
