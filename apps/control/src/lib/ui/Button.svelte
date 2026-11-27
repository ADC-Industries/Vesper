<script lang="ts">
  // Buttons: primary (violet), secondary, outline, danger, amber "live" and ghost (icon or text
  // only). Extra attributes (title, aria-*, data-testid, form, …) pass through.
  import type { Snippet } from "svelte";
  import type { HTMLButtonAttributes } from "svelte/elements";
  import { ICON_SIZE, ICON_STROKE, type ButtonVariant, type IconComponent } from "./types";

  let {
    variant = "secondary",
    size = "md",
    icon,
    iconOnly = false,
    pressed,
    type = "button",
    children,
    class: cls = "",
    ...rest
  }: HTMLButtonAttributes & {
    variant?: ButtonVariant;
    size?: "sm" | "md";
    icon?: IconComponent;
    /** A square icon button: give it an `aria-label`. */
    iconOnly?: boolean;
    /** A toggle button's state (`aria-pressed`). */
    pressed?: boolean;
    children?: Snippet;
  } = $props();
</script>

<button
  {type}
  class="ui-btn {variant} {size} {cls}"
  class:icon-only={iconOnly}
  class:on={pressed}
  aria-pressed={pressed}
  {...rest}
>
  {#if icon}{@const Icon = icon}<Icon size={ICON_SIZE} strokeWidth={ICON_STROKE} aria-hidden="true" />{/if}
  {#if children}{@render children()}{/if}
</button>

<style>
  .ui-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--sp-3);
    min-height: var(--control-h);
    padding: 6px 14px;
    border: 1px solid transparent;
    border-radius: var(--r);
    font-size: var(--fs);
    font-weight: 600;
    line-height: 1.2;
    white-space: nowrap;
    transition:
      background-color 0.12s,
      border-color 0.12s,
      color 0.12s;
  }
  .ui-btn.sm {
    min-height: 26px;
    padding: 4px 10px;
    font-size: var(--fs-sm);
  }
  .ui-btn.icon-only {
    width: var(--control-h);
    padding: 0;
  }
  .ui-btn.sm.icon-only {
    width: 26px;
  }
  .ui-btn:disabled {
    opacity: 0.45;
  }
  .primary {
    background: var(--accent);
    color: var(--text-strong);
  }
  .primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }
  .secondary {
    background: var(--surface-overlay);
    border-color: var(--line-strong);
    color: var(--text);
    font-weight: 500;
  }
  .secondary:hover:not(:disabled) {
    background: var(--line-strong);
  }
  .outline {
    background: transparent;
    border-color: var(--line-strong);
    color: var(--text);
    font-weight: 500;
  }
  .outline:hover:not(:disabled) {
    background: var(--surface-raised);
    border-color: var(--line-hover);
  }
  .danger {
    background: var(--danger-bg);
    color: var(--text-strong);
  }
  .danger:hover:not(:disabled) {
    background: var(--danger);
  }
  .live {
    background: var(--live);
    color: var(--live-ink);
    font-weight: 700;
  }
  .live:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .ghost {
    background: transparent;
    color: var(--text-2);
    font-weight: 500;
  }
  .ghost:hover:not(:disabled) {
    background: var(--surface-overlay);
    color: var(--text);
  }
  .ui-btn.on {
    background: var(--accent-sel);
    border-color: var(--accent);
    color: var(--text);
  }
</style>
