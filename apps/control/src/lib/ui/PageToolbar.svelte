<script lang="ts">
  // The second row on every page but Show: the page or document name, page controls, then
  // Done (violet or outline) or Close.
  import type { Snippet } from "svelte";
  import Button from "./Button.svelte";

  let {
    title,
    subtitle,
    children,
    doneLabel = "Done",
    doneVariant = "primary",
    ondone,
  }: {
    title: string;
    subtitle?: string;
    children?: Snippet;
    doneLabel?: "Done" | "Close";
    doneVariant?: "primary" | "outline";
    ondone?: () => void;
  } = $props();
</script>

<div class="ui-page-toolbar" role="toolbar" aria-label={title}>
  <h1>{title}{#if subtitle}<span class="sub">{subtitle}</span>{/if}</h1>
  <div class="controls">{#if children}{@render children()}{/if}</div>
  {#if ondone}<Button variant={doneVariant} size="sm" onclick={ondone}>{doneLabel}</Button>{/if}
</div>

<style>
  .ui-page-toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-6);
    min-height: 42px;
    padding: 6px 12px;
    background: var(--surface-sunk);
    border-bottom: 1px solid var(--line);
  }
  h1 {
    margin: 0;
    min-width: 0;
    font-size: var(--fs-title);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    margin-left: var(--sp-4);
    font-size: var(--fs-sm);
    font-weight: 400;
    color: var(--text-label);
  }
  .controls {
    flex: 1;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: var(--sp-4);
  }
</style>
