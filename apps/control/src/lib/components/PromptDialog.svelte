<script lang="ts">
  // The app's small question dialogs: a name, a confirmation, a playlist header (name and
  // colour). Opened through shell.askText / confirm / askHeader, which return promises. The
  // redesign's dialog: the question in the body, the buttons in the footer band.
  import Button from "../ui/Button.svelte";
  import Dialog from "../ui/Dialog.svelte";
  import Field from "../ui/Field.svelte";
  import { HEADER_COLORS } from "../state/session.svelte";
  import { shell } from "../state/shell.svelte";
  import { cssColor } from "../model";

  const p = $derived(shell.prompt!);
  let value = $state(shell.prompt?.value ?? "");
  let color = $state(shell.prompt?.color ?? HEADER_COLORS[0]!);

  function finish(ok: boolean) {
    const req = shell.prompt;
    if (!req) return;
    shell.prompt = null;
    if (!ok) return req.resolve(null);
    if (req.kind === "confirm") return req.resolve({ value: "" });
    const v = value.trim();
    if (!v) return req.resolve(null);
    req.resolve({ value: v, color });
  }

  function select(node: HTMLInputElement) {
    queueMicrotask(() => {
      node.focus();
      node.select();
    });
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      finish(true);
    }
  }
</script>

<Dialog title={p.title} onclose={() => finish(false)} width={440}>
  <div class="prompt" role="presentation" onkeydown={onKey}>
    {#if p.message}<p>{p.message}</p>{/if}
    {#if p.kind !== "confirm"}
      <Field label={p.label ?? "Name"}>
        <input type="text" bind:value spellcheck="false" {@attach select} data-prompt-input />
      </Field>
    {/if}
    {#if p.kind === "header"}
      <div class="colours">
        <span class="lbl" id="prompt-colour">Colour</span>
        <div class="swatches" role="radiogroup" aria-labelledby="prompt-colour">
          {#each HEADER_COLORS as c, i (i)}
            <button
              type="button"
              class="sw"
              class:on={c === color}
              style:background={cssColor(c)}
              role="radio"
              aria-checked={c === color}
              aria-label={`Colour ${i + 1}`}
              onclick={() => (color = c)}
            ></button>
          {/each}
        </div>
      </div>
    {/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => finish(false)}>Cancel</Button>
    {#if p.kind === "confirm"}
      <Button variant={p.danger ? "danger" : "primary"} onclick={() => finish(true)} {@attach (n: HTMLButtonElement) => queueMicrotask(() => n.focus())}>{p.confirm}</Button>
    {:else}
      <Button variant="primary" onclick={() => finish(true)}>{p.confirm}</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .prompt {
    display: flex;
    flex-direction: column;
    gap: var(--sp-6);
  }
  p {
    margin: 0;
    color: var(--text-2);
    line-height: 1.5;
  }
  input {
    height: var(--control-h);
    font-size: var(--fs-md);
  }
  .colours {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .lbl {
    font-size: var(--fs-sm);
    color: var(--text-label);
  }
  .swatches {
    display: flex;
    gap: var(--sp-4);
  }
  .sw {
    width: 26px;
    height: 26px;
    padding: 0;
    border-radius: 50%;
    border: 2px solid var(--surface-panel);
    box-shadow: 0 0 0 1px var(--line-strong);
  }
  .sw:hover {
    box-shadow: 0 0 0 1px var(--line-hover);
  }
  .sw.on {
    box-shadow: 0 0 0 2px var(--accent-ring);
  }
</style>
