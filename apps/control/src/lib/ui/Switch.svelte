<script lang="ts">
  // An on/off switch (38 × 22). Green when on, grey when off. With `onchange` it is
  // controlled: it reports the wanted state and the caller updates `checked` (e.g. when the
  // server's state changes); without it, it toggles its own (bindable) `checked`.
  let {
    checked = $bindable(false),
    label,
    disabled = false,
    showLabel = false,
    title,
    onchange,
    ...rest
  }: {
    checked?: boolean;
    /** The accessible name (shown beside the switch with `showLabel`). */
    label: string;
    disabled?: boolean;
    showLabel?: boolean;
    title?: string;
    onchange?: (checked: boolean) => void;
    [attr: `data-${string}`]: unknown;
  } = $props();

  function toggle() {
    if (onchange) onchange(!checked);
    else checked = !checked;
  }
</script>

{#snippet sw()}
  <button
    type="button"
    role="switch"
    class="ui-switch"
    class:on={checked}
    aria-checked={checked}
    aria-label={showLabel ? undefined : label}
    {title}
    {disabled}
    onclick={toggle}
    {...rest}
  >
    <span class="knob" aria-hidden="true"></span>
  </button>
{/snippet}

{#if showLabel}
  <label class="ui-switch-wrap" class:disabled>{@render sw()}<span>{label}</span></label>
{:else}
  {@render sw()}
{/if}

<style>
  .ui-switch-wrap {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-4);
  }
  .ui-switch-wrap.disabled,
  .ui-switch:disabled {
    opacity: 0.45;
  }
  .ui-switch {
    position: relative;
    flex: none;
    width: 38px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: 11px;
    background: var(--line-strong);
    transition: background-color 0.15s;
  }
  .ui-switch.on {
    background: var(--ok);
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--text-strong);
    transition: left 0.15s;
  }
  .on .knob {
    left: 19px;
  }
  @media (prefers-reduced-motion: reduce) {
    .ui-switch,
    .knob {
      transition: none;
    }
  }
</style>
