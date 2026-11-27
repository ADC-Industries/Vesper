<script lang="ts">
  // A number field that commits on Enter / blur (one op per change, not per keystroke), or on a
  // press elsewhere in the window.
  let {
    value,
    onchange,
    label,
    step = 1,
    min,
    max,
    unit = "",
    disabled = false,
    width = 64,
    digits = 2,
  }: {
    value: number | undefined | null;
    onchange: (v: number) => void;
    label: string;
    step?: number;
    min?: number;
    max?: number;
    unit?: string;
    disabled?: boolean;
    width?: number;
    digits?: number;
  } = $props();

  const show = (v: number | undefined | null) => (v === undefined || v === null || !Number.isFinite(v) ? "" : String(Math.round(v * 10 ** digits) / 10 ** digits));

  let input = $state<HTMLInputElement | null>(null);
  /** The text last committed while the field had the focus (a later blur doesn't send it twice). */
  let sent: string | null = null;
  function commit(el: HTMLInputElement) {
    const t = el.value.trim();
    if (t === show(value) || t === sent) return;
    let v = Number(t);
    if (!t || !Number.isFinite(v)) {
      el.value = show(value);
      return;
    }
    if (min !== undefined) v = Math.max(min, v);
    if (max !== undefined) v = Math.min(max, v);
    el.value = show(v);
    sent = el.value;
    onchange(v);
  }

  // A press elsewhere (the canvas selecting another object) commits first, while the field
  // still belongs to the object it was typed for: the selection changes before the blur.
  function pressElsewhere(e: PointerEvent) {
    if (input && document.activeElement === input && !input.contains(e.target as Node)) commit(input);
  }
</script>

<svelte:window onpointerdowncapture={pressElsewhere} />

<label class="num" style:--w={`${width}px`}>
  <input
    type="number"
    value={show(value)}
    {step}
    {min}
    {max}
    {disabled}
    aria-label={label}
    title={label}
    bind:this={input}
    onfocus={() => (sent = null)}
    onchange={(e) => commit(e.currentTarget)}
    onkeydown={(e) => {
      e.stopPropagation();
      if (e.key === "Enter") commit(e.currentTarget);
    }}
  />
  {#if unit}<span class="unit">{unit}</span>{/if}
</label>

<style>
  .num {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  input {
    width: var(--w);
    height: 22px;
    padding: 0 4px;
    background: var(--surface-sunk);
    border: 1px solid var(--line-strong);
    border-radius: 2px;
    font-variant-numeric: tabular-nums;
  }
  input:focus {
    outline: none;
    border-color: var(--accent-bright);
  }
  input:disabled {
    opacity: 0.5;
  }
  .unit {
    color: var(--text-faint);
    font-size: 11.5px;
  }
</style>
