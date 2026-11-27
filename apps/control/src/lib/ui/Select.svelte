<script lang="ts" generics="V extends string">
  // A native select in the field style (the base styles in app.css draw the chevron).
  import type { HTMLSelectAttributes } from "svelte/elements";

  interface Option {
    value: V;
    label: string;
    disabled?: boolean;
  }

  let {
    options,
    value = $bindable(),
    label,
    onchange,
    ...rest
  }: Omit<HTMLSelectAttributes, "value" | "onchange"> & {
    options: Option[];
    value: V;
    /** The accessible name (when no visible <label> points at it). */
    label?: string;
    onchange?: (v: V) => void;
  } = $props();
</script>

<select
  class="ui-select"
  aria-label={label}
  bind:value
  onchange={(e) => onchange?.(e.currentTarget.value as V)}
  {...rest}
>
  {#each options as o (o.value)}
    <option value={o.value} disabled={o.disabled}>{o.label}</option>
  {/each}
</select>

<style>
  .ui-select {
    min-height: var(--control-h);
    font-size: var(--fs);
  }
</style>
