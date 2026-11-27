<script lang="ts">
  // A labelled form field. Put the control inside; the label is tied to it by `for`/`id`
  // (pass `for` with the control's id) or by wrapping. `layout="row"`: label on the left (96 px).
  import type { Snippet } from "svelte";

  let {
    label,
    for: forId,
    hint,
    error,
    layout = "stack",
    children,
  }: {
    label: string;
    for?: string;
    hint?: string;
    error?: string | null;
    layout?: "stack" | "row";
    children: Snippet;
  } = $props();
</script>

<div class="ui-field {layout}" class:invalid={!!error}>
  {#if forId}
    <label class="lbl" for={forId}>{label}</label>
    <div class="ctl">{@render children()}</div>
  {:else}
    <label class="wrap">
      <span class="lbl">{label}</span>
      <span class="ctl">{@render children()}</span>
    </label>
  {/if}
  {#if error}
    <p class="msg err" role="alert">{error}</p>
  {:else if hint}
    <p class="msg">{hint}</p>
  {/if}
</div>

<style>
  .ui-field {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    min-width: 0;
  }
  .wrap {
    display: contents;
  }
  .lbl {
    font-size: var(--fs-sm);
    color: var(--text-label);
  }
  .ctl {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .row {
    display: grid;
    grid-template-columns: 96px minmax(0, 1fr);
    align-items: center;
    column-gap: var(--sp-4);
  }
  .row .lbl {
    text-align: right;
  }
  .row .msg {
    grid-column: 2;
  }
  .msg {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-meta);
  }
  .msg.err {
    color: var(--danger-text);
  }
  .invalid :global(input),
  .invalid :global(select),
  .invalid :global(textarea) {
    border-color: var(--danger);
  }
</style>
