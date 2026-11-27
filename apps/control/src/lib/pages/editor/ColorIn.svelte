<script lang="ts">
  // A colour swatch (the system colour picker); keeps the colour's alpha.
  import type { Color } from "$protocol";
  import { fromHex, toHex } from "../../edit/color";

  let { color, onchange, label, disabled = false }: { color: Color | undefined | null; onchange: (c: Color) => void; label: string; disabled?: boolean } = $props();
</script>

<input
  class="swatch"
  type="color"
  value={toHex(color)}
  {disabled}
  aria-label={label}
  title={label}
  onchange={(e) => onchange(fromHex(e.currentTarget.value, color?.a ?? 1))}
/>

<style>
  .swatch {
    width: 40px;
    height: 22px;
    padding: 0 2px;
    background: var(--surface-sunk);
    border: 1px solid var(--line-strong);
    border-radius: 2px;
    cursor: pointer;
  }
  .swatch:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>
