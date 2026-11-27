<script lang="ts">
  // A ruler in canvas px along the top or left of the Edit canvas (labels every major tick,
  // minor ticks between, the selection's extent shaded).
  import { rulerLabel, rulerTicks } from "../../edit/geometry";

  let {
    vertical = false,
    zoom,
    pan,
    mark = null,
  }: {
    vertical?: boolean;
    zoom: number;
    /** Screen offset of the model origin along this ruler. */
    pan: number;
    /** Selection extent in model px [from, to]. */
    mark?: [number, number] | null;
  } = $props();

  let el = $state<HTMLCanvasElement | null>(null);
  let w = $state(0);
  let h = $state(0);

  $effect(() => {
    const c = el;
    if (!c || !w || !h) return;
    const dpr = typeof devicePixelRatio === "number" ? devicePixelRatio : 1;
    c.width = Math.round(w * dpr);
    c.height = Math.round(h * dpr);
    const g = c.getContext("2d");
    if (!g) return;
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, w, h);
    const len = vertical ? h : w;
    const depth = vertical ? w : h;
    if (mark) {
      const a = mark[0] * zoom + pan;
      const b = mark[1] * zoom + pan;
      g.fillStyle = "rgba(10, 122, 255, 0.35)";
      if (vertical) g.fillRect(0, a, depth, b - a);
      else g.fillRect(a, 0, b - a, depth);
    }
    g.strokeStyle = "#cfcfcf";
    g.fillStyle = "#e6e6e6";
    g.lineWidth = 1;
    g.font = "10px system-ui, sans-serif";
    g.textBaseline = "top";
    g.beginPath();
    for (const t of rulerTicks(zoom, pan, len)) {
      const p = Math.round(t.pos) + 0.5;
      const tick = t.major ? depth : depth * 0.3;
      if (vertical) {
        g.moveTo(depth - tick, p);
        g.lineTo(depth, p);
      } else {
        g.moveTo(p, depth - tick);
        g.lineTo(p, depth);
      }
      if (t.major) {
        const label = rulerLabel(t.value);
        if (vertical) {
          // Turned a quarter clockwise so the label starts just below its tick and reads
          // downwards, as the top ruler's labels start just right of theirs ("-200" stays with
          // the tick above 0 instead of reading as "200" next to 0).
          g.save();
          g.translate(1, p + 3);
          g.rotate(Math.PI / 2);
          g.textAlign = "left";
          g.textBaseline = "bottom";
          g.fillText(label, 0, 0);
          g.restore();
        } else {
          g.fillText(label, p + 3, 1);
        }
      }
    }
    g.stroke();
  });
</script>

<div class="ruler" class:vertical bind:clientWidth={w} bind:clientHeight={h}>
  <canvas bind:this={el} style:width={`${w}px`} style:height={`${h}px`}></canvas>
</div>

<style>
  .ruler {
    position: relative;
    overflow: hidden;
    background: var(--surface-panel);
    min-width: 0;
    min-height: 0;
  }
  canvas {
    position: absolute;
    inset: 0;
    display: block;
  }
</style>
