<script lang="ts">
  // Master bus meters overlaid on the live preview (the master mute is beside the screen
  // picker). Levels arrive up to 10 times a second (`Connection.onAudioLevels`); the bars are
  // drawn straight into the DOM in animation frames so nothing else re-renders.
  import { onDestroy } from "svelte";
  import type { Connection } from "../../server";
  import { useLive } from "../../live";
  import { MeterBallistics, formatDb, meterFraction, meterZone } from "./audio";

  let { conn }: { conn: Connection } = $props();

  const audio = useLive().audio;
  let muted = $derived($audio?.muted ?? false);
  let masterDb = $derived($audio?.masterDb ?? 0);
  let outputState = $derived($audio?.outputState ?? "null");

  const meter = new MeterBallistics();
  let channels = $state(2);
  let bars: HTMLElement | null = null;
  let raf = 0;

  function draw() {
    const el = bars;
    if (!el) return;
    const fills = el.querySelectorAll<HTMLElement>(".mask");
    const peaks = el.querySelectorAll<HTMLElement>(".peak");
    meter.channels.forEach((c, i) => {
      const f = fills[i];
      const p = peaks[i];
      if (f) f.style.height = `${(1 - meterFraction(c.level)) * 100}%`;
      if (p) {
        const pf = meterFraction(c.peak);
        p.style.bottom = `${pf * 100}%`;
        p.style.opacity = pf > 0 ? "1" : "0";
        p.dataset.zone = meterZone(c.peak);
      }
    });
  }

  function frame(now: number) {
    const moving = meter.step(now);
    draw();
    raf = moving ? requestAnimationFrame(frame) : 0;
  }

  function kick() {
    if (raf) return;
    meter.resetClock();
    raf = requestAnimationFrame(frame);
  }

  function onLevels(levels: number[]) {
    const n = Math.max(1, Math.min(8, levels.length || 2));
    if (n !== channels) channels = n;
    meter.setLevels(levels, performance.now());
    kick();
  }

  // The view is re-created for a new connection, so `conn` never changes here.
  // svelte-ignore state_referenced_locally
  const off = conn.onAudioLevels((l) => onLevels(l.levels));
  // svelte-ignore state_referenced_locally
  onLevels(conn.audioLevels.levels);
  onDestroy(() => {
    off();
    if (raf) cancelAnimationFrame(raf);
  });

  function attach(node: HTMLElement) {
    bars = node;
    draw();
    return () => {
      bars = null;
    };
  }

  let title = $derived(
    `Master ${formatDb(masterDb)} dB${muted ? " (muted)" : ""}` +
      (outputState === "error" ? " · audio device problem (Settings ▸ Audio)" : outputState === "null" ? " · no local audio device" : ""),
  );
</script>

<div class="meters" class:muted {@attach attach} title={title} aria-hidden="true" data-testid="master-meters">
  {#each { length: channels }, i (i)}
    <div class="bar">
      <span class="mask" style:height="100%"></span>
      <span class="peak"></span>
    </div>
  {/each}
</div>

<style>
  /* Overlaid on the preview's left edge: −60 … 0 dBFS bottom to top, green to −18 (70 %),
     yellow to −6 (90 %), red above. */
  .meters {
    position: absolute;
    left: 6px;
    top: 28px;
    bottom: 8px;
    z-index: 2;
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: 3px;
    background: rgba(0, 0, 0, 0.6);
  }
  .bar {
    position: relative;
    width: 4px;
    height: 100%;
    overflow: hidden;
    border-radius: 1px;
    background: linear-gradient(to top, #2fae4e 0%, #3fc75e 70%, #e3c234 70%, #e3c234 90%, #e5413b 90%, #e5413b 100%);
  }
  .muted .bar {
    filter: grayscale(0.85) brightness(0.65);
  }
  .mask {
    position: absolute;
    left: 0;
    right: 0;
    top: 0;
    background: var(--surface-panel);
  }
  .peak {
    position: absolute;
    left: 0;
    right: 0;
    height: 2px;
    margin-bottom: -1px;
    bottom: 0;
    opacity: 0;
    background: var(--ok);
  }
  .bar :global(.peak[data-zone="yellow"]) {
    background: var(--live);
  }
  .bar :global(.peak[data-zone="red"]) {
    background: var(--danger);
  }
</style>
