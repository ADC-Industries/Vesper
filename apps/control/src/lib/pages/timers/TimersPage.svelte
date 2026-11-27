<script lang="ts">
  // The Timers page (plan 4.7): the timer cards with their live values and start / pause, the
  // system clock (the box-wide clock format: 12 / 24 h and seconds, `setClockFormat`), and the
  // selected timer: a big readout with its status, Start / Pause, Reset, −1 / +1 min
  // (`timerAdjust`), and its settings (name, type, duration or target time, an elapsed timer's
  // "Stop at", overrun, the amber warning time, a summary, duplicate, delete). Values come from the show state and `timerTick`; settings are the timer documents.
  // The Show page's compact Timers tab is show/CompactTimers.
  import { onDestroy } from "svelte";
  import { useLive } from "../../live";
  import Play from "lucide-svelte/icons/play";
  import Pause from "lucide-svelte/icons/pause";
  import type { TimerEntry, TimerKind } from "$protocol";
  import type { Connection } from "../../server";
  import PageFrame from "../PageFrame.svelte";
  import { Button, EmptyState, NewButton, SectionLabel } from "../../ui";
  import { session } from "../../state/session.svelte";
  import { shell } from "../../state/shell.svelte";
  import { timersState } from "../../state/timers.svelte";
  import { presence } from "../../state/presence.svelte";
  import PresenceBadge from "../../components/PresenceBadge.svelte";
  import LockBanner from "../../components/LockBanner.svelte";
  import {
    KIND_LABELS,
    applySetting,
    defaultKind,
    durationFrom,
    formatClock,
    isRunning,
    kindFromTimeInput,
    timeInputValue,
    timerSetting,
    timerStatus,
    timerSummary,
    timerTone,
    warningFrom,
    warningText,
  } from "./timers";

  let { conn }: { conn: Connection } = $props();
  // svelte-ignore state_referenced_locally
  const values = conn.timerValues;

  let now = $state(new Date());
  const tick = setInterval(() => (now = new Date()), 1000);
  onDestroy(() => clearInterval(tick));

  let timers = $derived(timersState.timerDocs);
  let sel = $derived(timers.find((t) => t.id === timersState.selected) ?? timers[0] ?? null);
  let selValue = $derived(sel ? $values[sel.id] : undefined);
  let selState = $derived(selValue?.state ?? "stopped");
  let selKind = $derived<TimerKind>(sel?.kind ?? { type: "countdown" });
  let running = $derived(isRunning(selState));
  let selTone = $derived(timerTone(selState, selValue?.currentSeconds, warnOf(sel)));
  const clockFormat = useLive().clockFormat;

  // Presence (G8): the selected timer's settings are open here; someone else editing them
  // (holding its lock) makes them read-only until this client takes over. Starting, pausing
  // and adjusting are commands: never locked.
  $effect(() => {
    presence.setOpen("timers", sel ? [{ kind: "timer", id: sel.id, place: "timers" }] : null);
  });
  $effect(() => () => presence.setOpen("timers", null));
  let lockedBy = $derived(presence.lockedBy("timer", sel?.id));

  /** A countdown's warning threshold (elapsed timers have none). */
  function warnOf(t: TimerEntry | null | undefined): number | null {
    return t && t.kind?.type !== "elapsed" ? (t.warningSeconds ?? null) : null;
  }

  function setClock(use24Hour: boolean, seconds: boolean) {
    timersState.setClock24(use24Hour);
    void session.run({ type: "setClockFormat", use24Hour, seconds });
  }

  function setWarning(input: HTMLInputElement) {
    if (!sel) return;
    const w = warningFrom(input.value);
    if (w === undefined) {
      conn.pushNotice("warning", "Enter the minutes left, like 2 or 0.5, or leave it empty.");
      input.value = warningText(sel.warningSeconds);
    } else if (w !== (sel.warningSeconds ?? null)) save({ warningSeconds: w ?? undefined }, "Warning time");
  }
  let clockTimer = $derived(selKind.type === "countdownToTime");

  const KIND_SHORT: Record<TimerKind["type"], string> = {
    countdown: "Countdown",
    countdownToTime: "Count down to a time",
    elapsed: "Elapsed",
    unknown: "Unknown",
  };

  function toggle(t: TimerEntry) {
    void session.run({ type: isRunning($values[t.id]?.state) ? "timerStop" : "timerStart", timer: t.id });
  }

  function adjust(delta: number) {
    if (sel) void session.run({ type: "timerAdjust", timer: sel.id, deltaSeconds: delta });
  }

  function save(patch: { name?: string; kind?: TimerKind; allowOverrun?: boolean; warningSeconds?: number }, what?: string) {
    if (sel) void timersState.update(sel, patch, what);
  }

  function setName(input: HTMLInputElement) {
    if (!sel) return;
    const n = input.value.trim();
    if (!n) input.value = sel.name ?? "";
    else if (n !== sel.name) save({ name: n }, "Rename");
  }

  function setType(type: TimerKind["type"]) {
    if (type !== selKind.type) save({ kind: defaultKind(type) });
  }

  function setDuration(min: number, sec: number) {
    const seconds = durationFrom(min, sec);
    if (selKind.type === "countdown" && seconds !== (selKind.seconds ?? 300)) save({ kind: { type: "countdown", seconds } });
  }

  function setTarget(input: HTMLInputElement) {
    const kind = kindFromTimeInput(selKind, input.value);
    if (!kind) input.value = timeInputValue(selKind.type === "countdownToTime" ? (selKind.secondsAfterMidnight ?? 0) : 0);
    else if (JSON.stringify(kind) !== JSON.stringify(selKind)) save({ kind });
  }

  function setStopAt(input: HTMLInputElement) {
    const kind = applySetting(selKind, input.value);
    if (!kind) {
      conn.pushNotice("warning", "Enter a duration like 5:00 or 1:30:00, or leave it empty.");
      input.value = selKind.type === "elapsed" && selKind.end != null ? timerSetting(selKind) : "";
    } else if (JSON.stringify(kind) !== JSON.stringify(selKind)) save({ kind });
  }

  async function remove() {
    if (!sel) return;
    const t = sel;
    if (await shell.confirm("Delete Timer", `Delete “${t.name || "Timer"}”? Slides and stage layouts that show it will show nothing.`)) await timersState.remove(t);
  }

  const blurOnEnter = (e: KeyboardEvent) => {
    e.stopPropagation();
    if (e.key === "Enter") (e.currentTarget as HTMLElement).blur();
  };
</script>

<PageFrame page="timers">
  <div class="layout" data-testid="timers-page">
    <aside class="list" aria-label="Timers">
      <SectionLabel>
        Timers
        {#snippet actions()}<NewButton label="New timer" onclick={() => void timersState.create()} data-testid="new-timer" />{/snippet}
      </SectionLabel>
      {#each timers as t (t.id)}
        {@const v = $values[t.id]}
        {@const on = sel?.id === t.id}
        {@const run = isRunning(v?.state)}
        <div class="card" class:on data-testid="timer-card">
          <button class="pick" onclick={() => (timersState.selected = t.id)} aria-current={on ? "true" : undefined}>
            <span class="names">
              <span class="name">{t.name || "Timer"}</span>
              <span class="kind">{KIND_SHORT[t.kind?.type ?? "countdown"]}</span>
            </span>
            <span class="value tone-{timerTone(v?.state, v?.currentSeconds, warnOf(t))}">{v?.display ?? "--:--"}</span>
          </button>
          <button class="toggle" onclick={() => toggle(t)} aria-label={`${run ? "Pause" : "Start"} ${t.name || "Timer"}`} title={run ? "Pause" : "Start"}>
            {#if run}<Pause size={15} fill="currentColor" />{:else}<Play size={15} fill="currentColor" />{/if}
          </button>
        </div>
      {:else}
        <p class="none">No timers yet.</p>
      {/each}

      <div class="gap"></div>
      <SectionLabel>System clock</SectionLabel>
      <div class="card clock">
        <span class="value" data-testid="system-clock">{formatClock(now, $clockFormat.use24Hour, $clockFormat.seconds)}</span>
        <select
          value={$clockFormat.use24Hour ? "24" : "12"}
          onchange={(e) => setClock(e.currentTarget.value === "24", $clockFormat.seconds)}
          aria-label="Clock format"
          title="The clock format of this server's screens: stage clocks and clock text that don't set their own"
        >
          <option value="12">12-hour</option>
          <option value="24">24-hour</option>
        </select>
        <label class="secs" title="Show seconds on this server's clocks">
          <input type="checkbox" checked={$clockFormat.seconds} onchange={(e) => setClock($clockFormat.use24Hour, e.currentTarget.checked)} />
          Seconds
        </label>
      </div>
    </aside>

    <main class="main">
      {#if sel}
        <div class="readout" data-state={selState}>
          <span class="label">{(sel.name || "Timer").toUpperCase()}</span>
          <span class="big tone-{selTone}" data-testid="timer-readout">{selValue?.display ?? "--:--"}</span>
          <span class="status tone-{selTone}">{timerStatus(selKind, selState)}</span>
        </div>

        <div class="controls" role="group" aria-label="Timer controls">
          <Button variant={running ? "secondary" : "primary"} class="ss" onclick={() => toggle(sel)} data-testid="timer-start-stop">{running ? "Pause" : "Start"}</Button>
          <Button onclick={() => void session.run({ type: "timerReset", timer: sel.id })}>Reset</Button>
          <Button onclick={() => adjust(-60)} disabled={clockTimer} title={clockTimer ? "This timer follows the clock" : "Take a minute off"}>−1 min</Button>
          <Button onclick={() => adjust(60)} disabled={clockTimer} title={clockTimer ? "This timer follows the clock" : "Add a minute"}>+1 min</Button>
        </div>

        {#if lockedBy}<div class="banner"><LockBanner kind="timer" id={sel.id} noun="timer" /></div>{/if}
        <section class="settings" aria-label="Timer settings">
          <div class="who"><span class="cap">Settings</span><PresenceBadge kind="timer" id={sel.id} /></div>
          <fieldset class="lockable" disabled={!!lockedBy} data-timer-settings>
          {#key sel.revision}
            <div class="row2">
              <label class="f">
                Name
                <input type="text" value={sel.name ?? ""} onchange={(e) => setName(e.currentTarget)} onkeydown={blurOnEnter} spellcheck="false" />
              </label>
              <label class="f">
                Type
                <select value={selKind.type} onchange={(e) => setType(e.currentTarget.value as TimerKind["type"])} disabled={selKind.type === "unknown"}>
                  <option value="countdown">Countdown</option>
                  <option value="countdownToTime">Count down to a time</option>
                  <option value="elapsed">Elapsed (count up)</option>
                  {#if selKind.type === "unknown"}<option value="unknown">{KIND_LABELS.unknown}</option>{/if}
                </select>
              </label>
            </div>

            {#if selKind.type === "countdown"}
              {@const d = selKind.seconds ?? 300}
              <div class="row2">
                <label class="f">
                  Minutes
                  <input
                    type="number"
                    min="0"
                    max="5999"
                    value={Math.floor(d / 60)}
                    onchange={(e) => setDuration(e.currentTarget.valueAsNumber, d % 60)}
                    onkeydown={blurOnEnter}
                    data-testid="timer-minutes"
                  />
                </label>
                <label class="f">
                  Seconds
                  <input type="number" min="0" max="59" value={d % 60} onchange={(e) => setDuration(Math.floor(d / 60), e.currentTarget.valueAsNumber)} onkeydown={blurOnEnter} />
                </label>
              </div>
            {:else if selKind.type === "countdownToTime"}
              <label class="f">
                Counts down to
                <input type="time" value={timeInputValue(selKind.secondsAfterMidnight ?? 0)} onchange={(e) => setTarget(e.currentTarget)} onkeydown={blurOnEnter} />
              </label>
            {:else if selKind.type === "elapsed"}
              <label class="f">
                Stop at
                <input
                  type="text"
                  value={selKind.end != null ? timerSetting(selKind) : ""}
                  placeholder="never (runs until paused)"
                  onchange={(e) => setStopAt(e.currentTarget)}
                  onkeydown={blurOnEnter}
                  spellcheck="false"
                />
              </label>
            {/if}
          {/key}

          {#if selKind.type === "countdown" || selKind.type === "countdownToTime"}
            <div class="row2 end">
              <label class="check">
                <input type="checkbox" checked={!!sel.allowOverrun} onchange={() => save({ allowOverrun: !sel.allowOverrun }, "Allow overrun")} />
                Keep counting up after zero (overrun)
              </label>
              <label class="f" title="The readout here, on the Show page and on stage screens turns amber this close to zero (timer elements with their own colour changes keep those)">
                Turn amber with this many minutes left
                {#key sel.id + (sel.warningSeconds ?? "")}
                  <input type="text" inputmode="decimal" value={warningText(sel.warningSeconds)} placeholder="Never" onchange={(e) => setWarning(e.currentTarget)} onkeydown={blurOnEnter} data-testid="timer-warning" />
                {/key}
              </label>
            </div>
          {/if}

          </fieldset>
          <div class="foot">
            <span class="summary">{timerSummary(selKind, !!sel.allowOverrun)}</span>
            <Button size="sm" onclick={() => void timersState.create(sel)}>Duplicate</Button>
            <Button size="sm" variant="danger" disabled={!!lockedBy} onclick={() => void remove()} data-testid="delete-timer">Delete timer</Button>
          </div>
        </section>
      {:else}
        <EmptyState title="No timers yet">
          Timers count down to the end of the sermon or the start of the service, or count up.
          {#snippet action()}<Button variant="primary" onclick={() => void timersState.create()}>New timer</Button>{/snippet}
        </EmptyState>
      {/if}
    </main>
  </div>
</PageFrame>

<style>
  .layout {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-wrap: wrap;
    overflow: auto;
  }
  .list {
    flex: 0 0 300px;
    max-width: 100%;
    max-height: 100%;
    box-sizing: border-box;
    overflow-y: auto;
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: var(--surface-panel);
    border-right: 1px solid var(--line);
  }
  .gap {
    height: 10px;
  }
  .card {
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: var(--r-card);
    border: 1px solid var(--line);
    background: var(--surface-raised);
  }
  .card.on {
    background: var(--accent-sel);
    border-color: var(--accent-ring);
  }
  .pick {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px;
    border: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
  }
  .names {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .kind {
    color: var(--text-label);
    font-size: var(--fs-xs);
  }
  .value {
    font-size: 18px;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .toggle {
    flex: none;
    width: 32px;
    height: 32px;
    margin-right: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-panel);
    color: var(--text);
  }
  .toggle:hover {
    background: var(--surface-overlay);
  }
  .clock {
    flex-wrap: wrap;
    justify-content: space-between;
    padding: 10px;
    gap: 8px;
  }
  .none {
    margin: 4px 2px;
    color: var(--text-meta);
  }
  .tone-run {
    color: var(--ok-text);
  }
  .tone-idle {
    color: var(--text-2);
  }
  .tone-over {
    color: var(--danger-text);
  }
  .tone-warn {
    color: var(--live);
  }
  .secs {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    color: var(--text-label);
    white-space: nowrap;
  }
  .main {
    flex: 999 1 460px;
    min-width: 0;
    max-height: 100%;
    box-sizing: border-box;
    overflow-y: auto;
    padding: 28px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 22px;
    background: var(--surface-backdrop);
  }
  .readout {
    width: 100%;
    max-width: 640px;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 28px 20px;
    border-radius: var(--r-dialog);
    background: #000;
    box-shadow: 0 0 0 1px var(--line);
  }
  .readout .label {
    font-size: var(--fs-sm);
    font-weight: 700;
    letter-spacing: 0.1em;
    color: var(--text-label);
  }
  .big {
    font-size: 96px;
    line-height: 1;
    font-weight: 700;
    font-variant-numeric: tabular-nums;
  }
  .status {
    font-size: 12.5px;
    font-weight: 600;
  }
  .controls {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 8px;
  }
  .controls :global(button) {
    height: 38px;
    padding: 0 16px;
    border-radius: var(--r-card);
  }
  .controls :global(.ss) {
    min-width: 110px;
    font-weight: 700;
  }
  .lockable {
    display: contents;
  }
  .banner {
    width: 100%;
    max-width: 640px;
  }
  .who {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 22px;
  }
  .who .cap {
    color: var(--text-label);
    font-size: var(--fs-xs);
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
  }
  .settings {
    width: 100%;
    max-width: 640px;
    box-sizing: border-box;
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 16px;
    border-radius: var(--r-popover);
    background: var(--surface-panel);
    border: 1px solid var(--line);
  }
  .row2 {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }
  .row2.end {
    align-items: end;
  }
  .f {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--text-label);
    font-size: 11.5px;
  }
  .f input,
  .f select {
    width: 100%;
    box-sizing: border-box;
    color: var(--text);
    font-size: var(--fs);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-bottom: 8px;
  }
  .check input {
    width: 16px;
    height: 16px;
  }
  .foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding-top: 10px;
    border-top: 1px solid var(--line);
  }
  .summary {
    flex: 1 1 200px;
    color: var(--text-label);
    font-size: var(--fs-sm);
  }
  @media (max-width: 760px) {
    .row2 {
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
