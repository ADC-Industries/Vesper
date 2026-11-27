<script lang="ts">
  // A colour well: the colour as a swatch that opens the system colour picker.
  let {
    value = $bindable("#000000"),
    label,
    disabled = false,
    showHex = false,
    onchange,
    oninput,
  }: {
    /** `#rrggbb`. */
    value?: string;
    label: string;
    disabled?: boolean;
    showHex?: boolean;
    /** When the picker closes on a colour. */
    onchange?: (v: string) => void;
    /** While the colour is being dragged. */
    oninput?: (v: string) => void;
  } = $props();
</script>

<span class="ui-color" class:disabled>
  <input
    type="color"
    aria-label={label}
    title={label}
    {disabled}
    bind:value
    oninput={(e) => oninput?.(e.currentTarget.value)}
    onchange={(e) => onchange?.(e.currentTarget.value)}
  />
  {#if showHex}<span class="hex">{value.toUpperCase()}</span>{/if}
</span>

<style>
  .ui-color {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-4);
    min-width: 0;
  }
  .ui-color.disabled {
    opacity: 0.45;
  }
  input {
    width: 44px;
    height: var(--control-h);
    padding: 3px;
    border: 1px solid var(--line-strong);
    border-radius: var(--r-sm);
    background: var(--surface-panel);
    cursor: pointer;
  }
  input:disabled {
    cursor: default;
  }
  input::-webkit-color-swatch-wrapper {
    padding: 0;
  }
  input::-webkit-color-swatch {
    border: 0;
    border-radius: var(--r-xs);
  }
  input::-moz-color-swatch {
    border: 0;
    border-radius: var(--r-xs);
  }
  .hex {
    font-family: var(--mono);
    font-size: var(--fs-sm);
    color: var(--text-2);
  }
</style>
