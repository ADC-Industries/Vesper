<script lang="ts">
  // A range slider with the violet fill. `format` shows the value beside it.
  let {
    value = $bindable(0),
    min = 0,
    max = 100,
    step = 1,
    label,
    disabled = false,
    format,
    width,
    oninput,
    onchange,
  }: {
    value?: number;
    min?: number;
    max?: number;
    step?: number;
    label: string;
    disabled?: boolean;
    /** Shown beside the slider (and as `aria-valuetext`). */
    format?: (v: number) => string;
    width?: number;
    oninput?: (v: number) => void;
    onchange?: (v: number) => void;
  } = $props();

  let pct = $derived(max > min ? ((value - min) / (max - min)) * 100 : 0);
</script>

<span class="ui-slider" class:disabled>
  <input
    type="range"
    {min}
    {max}
    {step}
    {disabled}
    aria-label={label}
    aria-valuetext={format?.(value)}
    style:--pct="{pct}%"
    style:width={width ? `${width}px` : undefined}
    bind:value
    oninput={(e) => oninput?.(Number(e.currentTarget.value))}
    onchange={(e) => onchange?.(Number(e.currentTarget.value))}
  />
  {#if format}<output>{format(value)}</output>{/if}
</span>

<style>
  .ui-slider {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-4);
    min-width: 0;
  }
  .ui-slider.disabled {
    opacity: 0.45;
  }
  input {
    flex: 1;
    min-width: 60px;
    height: 16px;
    margin: 0;
    appearance: none;
    background: transparent;
    cursor: pointer;
  }
  input::-webkit-slider-runnable-track {
    height: 4px;
    border-radius: 2px;
    background: linear-gradient(to right, var(--accent) var(--pct), var(--line-strong) var(--pct));
  }
  input::-moz-range-track {
    height: 4px;
    border-radius: 2px;
    background: var(--line-strong);
  }
  input::-moz-range-progress {
    height: 4px;
    border-radius: 2px;
    background: var(--accent);
  }
  input::-webkit-slider-thumb {
    appearance: none;
    width: 14px;
    height: 14px;
    margin-top: -5px;
    border-radius: 50%;
    background: var(--text);
    border: 0;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.5);
  }
  input::-moz-range-thumb {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--text);
    border: 0;
  }
  input:focus-visible {
    outline: none;
  }
  input:focus-visible::-webkit-slider-thumb {
    box-shadow: 0 0 0 3px var(--accent-ring);
  }
  output {
    min-width: 3ch;
    font-size: var(--fs-sm);
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
  }
</style>
