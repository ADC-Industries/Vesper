<script lang="ts">
  // A channel's schedule as a week calendar (Monday first): each slot a block on its days (a slot
  // past midnight continues on the next day), the "now" line, drag down a day to make a slot
  // (snapped to 15 minutes, the first playlist), click a block to edit its slot: playlist,
  // repeat days, times, priority, date range, remove. Changes save the channel's slots whole.
  import { onDestroy } from "svelte";
  import type { SignageChannel, SignagePlaylist, SignageSlot } from "$protocol";
  import { Button, Select } from "../../ui";
  import { signagePage } from "../../state/signage.svelte";
  import { DAYS, DAY_MINUTES, daysLabel, hhmm, nowMarker, parseHhmm, slotBlocks, slotFromDrag } from "./signage";

  let { channel, playlists }: { channel: SignageChannel; playlists: SignagePlaylist[] } = $props();

  const HEIGHT = 384;
  let now = $state(new Date());
  const t = setInterval(() => (now = new Date()), 30000);
  onDestroy(() => clearInterval(t));
  let marker = $derived(nowMarker(now));
  // Edits apply to a local copy of the slots at once and are saved one after another, each
  // whole: a quick second edit builds on the first, not on the slots before it (H10-4). The
  // server's slots show again once every save is in.
  let pending = $state<{ id: string; slots: SignageSlot[] } | null>(null);
  let slots = $derived(pending && pending.id === channel.id ? pending.slots : channel.slots);
  let saving: Promise<unknown> = Promise.resolve();
  let inFlight = 0;
  let blocks = $derived(slotBlocks(slots, playlists));
  let selSlot = $state<number | null>(null);
  let drag = $state<{ day: number; y0: number; y1: number } | null>(null);

  function save(next: SignageSlot[]) {
    const id = channel.id;
    pending = { id, slots: next };
    inFlight++;
    saving = saving
      .then(() => signagePage.putChannel(id, { slots: next }))
      .catch(() => undefined)
      .then(async () => {
        if (--inFlight > 0) return;
        await signagePage.load();
        if (inFlight === 0) pending = null;
      });
  }
  function ypos(e: PointerEvent): number {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    return e.clientY - r.top;
  }
  function down(day: number, e: PointerEvent) {
    if (!playlists.length || (e.target as HTMLElement).closest(".block")) return;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    const y = ypos(e);
    drag = { day, y0: y, y1: y };
  }
  function move(e: PointerEvent) {
    if (drag) drag = { ...drag, y1: ypos(e) };
  }
  function up() {
    if (!drag) return;
    const first = playlists[0];
    if (!first) return (drag = null);
    const s = slotFromDrag(drag.day, drag.y0, drag.y1, HEIGHT, first.id);
    drag = null;
    selSlot = slots.length;
    save([...slots, s]);
  }
  let cur = $derived(selSlot !== null ? (slots[selSlot] ?? null) : null);
  function edit(patch: Partial<SignageSlot>) {
    if (selSlot === null || !cur) return;
    save(slots.map((s, i) => (i === selSlot ? { ...s, ...patch } : s)));
  }
  function remove() {
    if (selSlot === null) return;
    save(slots.filter((_, i) => i !== selSlot));
    selSlot = null;
  }
  const pct = (m: number) => (m / DAY_MINUTES) * 100;
  const playlistChoices = $derived(playlists.map((p) => ({ value: p.id, label: p.name })));
  const HOURS = [0, 3, 6, 9, 12, 15, 18, 21];
</script>

<div class="cal-wrap" data-testid="signage-calendar">
  <div class="st-row">
    <h2 class="st-section">Schedule</h2>
    <span class="st-meta">{playlists.length ? "Drag down a day to add a slot; the highest priority plays, else the default playlist." : "Make a playlist first (Playlists, below)."}</span>
  </div>
  <div class="cal">
    <div class="hours" style:height="{HEIGHT}px">
      {#each HOURS as h (h)}<span style:top="{pct(h * 60)}%">{hhmm(h * 60)}</span>{/each}
    </div>
    {#each DAYS as d, day (d)}
      <div class="day">
        <div class="dname" class:today={marker.day === day}>{d}</div>
        <div class="col" style:height="{HEIGHT}px" role="presentation" onpointerdown={(e) => down(day, e)} onpointermove={move} onpointerup={up} data-testid="signage-day">
          {#each HOURS as h (h)}<span class="hl" style:top="{pct(h * 60)}%"></span>{/each}
          {#each blocks.filter((b) => b.day === day) as b (`${b.slot}-${b.from}`)}
            <button type="button" class="block" class:cur={selSlot === b.slot} style:top="{pct(b.from)}%" style:height="{pct(b.to - b.from)}%" onclick={() => (selSlot = b.slot)} title={`${b.label} · ${hhmm(b.from)}–${hhmm(b.to)} · priority ${b.priority}`} data-testid="signage-block">
              <span class="bl">{b.label}</span>
              <span class="bt">{hhmm(b.from)}–{hhmm(b.to)}{b.priority ? ` · P${b.priority}` : ""}</span>
            </button>
          {/each}
          {#if drag && drag.day === day}
            <span class="ghost" style:top="{(Math.min(drag.y0, drag.y1) / HEIGHT) * 100}%" style:height="{(Math.abs(drag.y1 - drag.y0) / HEIGHT) * 100}%"></span>
          {/if}
          {#if marker.day === day}<span class="now" style:top="{pct(marker.minute)}%" title="Now" data-testid="signage-now"></span>{/if}
        </div>
      </div>
    {/each}
  </div>
  {#if cur}
    <div class="slot" data-testid="signage-slot">
      <label>Plays<Select label="Playlist" options={playlistChoices} value={cur.playlist} onchange={(v) => edit({ playlist: v })} /></label>
      <span class="days">
        {#each DAYS as d, i (d)}
          <label class="chk"><input type="checkbox" checked={cur.days.length === 0 || cur.days.includes(i)} onchange={(e) => {
            const all = cur!.days.length ? cur!.days : [0, 1, 2, 3, 4, 5, 6];
            const next = e.currentTarget.checked ? [...all, i] : all.filter((x) => x !== i);
            edit({ days: next.length === 7 ? [] : next.length ? next : [i] });
          }} />{d}</label>
        {/each}
        <span class="st-meta">{daysLabel(cur.days)}</span>
      </span>
      <label>From<input type="time" value={hhmm(cur.start)} onchange={(e) => { const m = parseHhmm(e.currentTarget.value); if (m !== null && m < DAY_MINUTES) edit({ start: m }); }} /></label>
      <label>To<input type="time" value={hhmm(cur.end % DAY_MINUTES)} onchange={(e) => { const m = parseHhmm(e.currentTarget.value); if (m !== null) edit({ end: m || DAY_MINUTES }); }} /></label>
      <label>Priority<input type="number" min="-100" max="100" value={cur.priority} onchange={(e) => edit({ priority: Number(e.currentTarget.value) || 0 })} /></label>
      <label>Starting<input type="date" value={cur.from ?? ""} onchange={(e) => edit({ from: e.currentTarget.value || undefined })} /></label>
      <label>Until<input type="date" value={cur.until ?? ""} onchange={(e) => edit({ until: e.currentTarget.value || undefined })} /></label>
      <Button size="sm" onclick={remove}>Remove slot</Button>
    </div>
  {/if}
</div>

<style>
  .cal-wrap {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .cal {
    display: flex;
    gap: 4px;
    user-select: none;
  }
  .hours {
    position: relative;
    width: 40px;
    margin-top: 22px;
    flex: 0 0 auto;
  }
  .hours span {
    position: absolute;
    right: 4px;
    transform: translateY(-50%);
    font-size: 10.5px;
    color: var(--text-meta);
    font-variant-numeric: tabular-nums;
  }
  .day {
    flex: 1;
    min-width: 0;
  }
  .dname {
    height: 22px;
    text-align: center;
    font-size: var(--fs-sm);
    color: var(--text-label);
  }
  .dname.today {
    color: var(--accent);
    font-weight: 700;
  }
  .col {
    position: relative;
    background: var(--surface-raised);
    border-radius: var(--r);
    cursor: crosshair;
    touch-action: none;
  }
  .hl {
    position: absolute;
    left: 0;
    right: 0;
    border-top: 1px solid var(--line);
    opacity: 0.6;
  }
  .block {
    position: absolute;
    left: 2px;
    right: 2px;
    min-height: 14px;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    padding: 2px 4px;
    overflow: hidden;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background: var(--accent-sel);
    color: var(--text-strong);
    font-size: 11px;
    text-align: left;
    cursor: pointer;
  }
  .block.cur {
    background: var(--accent);
  }
  .bl {
    font-weight: 700;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }
  .bt {
    font-size: 10px;
    opacity: 0.85;
    white-space: nowrap;
  }
  .ghost {
    position: absolute;
    left: 2px;
    right: 2px;
    border: 1px dashed var(--accent);
    border-radius: 4px;
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    pointer-events: none;
  }
  .now {
    position: absolute;
    left: -2px;
    right: -2px;
    height: 2px;
    background: var(--live);
    pointer-events: none;
  }
  .slot {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 8px 12px;
    padding: 10px;
    border: 1px solid var(--line);
    border-radius: var(--r);
  }
  .slot label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    font-size: var(--fs-sm);
    color: var(--text-label);
  }
  .slot label.chk {
    flex-direction: row;
    align-items: center;
    gap: 3px;
  }
  .days {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .slot input[type="number"] {
    width: 70px;
  }
</style>
