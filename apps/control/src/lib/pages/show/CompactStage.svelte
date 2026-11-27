<script lang="ts">
  // The right column's Stage tab: the layout each stage screen shows (click one to switch),
  // and a quick message to the performers. Designing layouts and the message presets are the
  // Stage page's.
  import Monitor from "lucide-svelte/icons/monitor";
  import { stageState } from "../../state/stage.svelte";
  import { shell } from "../../state/shell.svelte";
  import { useLive } from "../../live";
  import Button from "../../ui/Button.svelte";
  import { BUILT_IN_LAYOUT, MAX_MESSAGE } from "../stage/stage";

  const live = useLive();
  const stageScreens = live.stageScreens;
  const stageMessage = live.stageMessage;

  let picked = $state<string | null>(null);
  let target = $derived($stageScreens.find((s) => s.screen === picked) ?? $stageScreens[0] ?? null);
  let layouts = $derived(stageState.stageLayoutDocs);
  let text = $state("");

  async function send() {
    if (await stageState.stageMessage(text)) text = text.trim();
  }
</script>

<div class="stage" data-testid="compact-stage">
  {#if target}
    <div class="row">
      {#if $stageScreens.length > 1}
        <label class="pick">
          <span>Stage layout on</span>
          <select value={target.screen} onchange={(e) => (picked = e.currentTarget.value)} aria-label="Stage screen">
            {#each $stageScreens as s (s.screen)}<option value={s.screen}>{s.name || "Stage"}</option>{/each}
          </select>
        </label>
      {:else}
        <span class="lbl"><Monitor size={13} />Stage layout on {target.name || "Stage"}</span>
      {/if}
    </div>
    <div class="layouts" role="group" aria-label="Stage layouts">
      {#each layouts as l (l.id)}
        {@const on = target.layout === l.id}
        <button class="lay" class:on aria-pressed={on} onclick={() => stageState.setStageLayout(target.screen, l.id)}>
          <span class="lname">{l.name || "Layout"}</span>
          {#if on}<span class="cur">Showing</span>{/if}
        </button>
      {:else}
        <button class="lay on" aria-pressed="true" disabled>{BUILT_IN_LAYOUT} (built-in)</button>
      {/each}
    </div>
  {:else}
    <p class="none">No stage screens. <button class="link" onclick={() => shell.go("screens")}>Configure screens</button></p>
  {/if}

  <div class="msg">
    <span class="lbl">Message to the performers{#if $stageMessage}<span class="onair" title={$stageMessage.text}>ON STAGE</span>{/if}</span>
    <input
      type="text"
      bind:value={text}
      maxlength={MAX_MESSAGE}
      placeholder={$stageMessage?.text ?? "Message"}
      onkeydown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") void send();
      }}
      aria-label="Stage message"
      data-testid="compact-stage-message"
    />
    <div class="btns">
      <Button size="sm" variant="primary" disabled={!text.trim()} onclick={() => void send()}>Show</Button>
      <Button size="sm" disabled={!$stageMessage} onclick={() => void stageState.stageMessageClear()}>Clear</Button>
    </div>
  </div>
  <button class="link" onclick={() => shell.go("stage")}>Open the Stage page</button>
</div>

<style>
  .stage {
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: stretch;
  }
  .lbl {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-size: 12px;
  }
  .pick {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-size: 12px;
  }
  .layouts {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .lay {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 8px 10px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-raised);
    color: var(--text);
    font-size: 12px;
    text-align: left;
  }
  .lay:hover:not(:disabled) {
    background: var(--surface-overlay);
  }
  .lay.on {
    border-color: var(--accent-ring);
  }
  .cur {
    color: var(--text-meta);
  }
  .msg {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 4px;
  }
  .btns {
    display: flex;
    gap: 6px;
  }
  .onair {
    margin-left: auto;
    padding: 0 5px;
    border-radius: 3px;
    background: var(--live);
    color: var(--live-ink);
    font-size: 10px;
    font-weight: 700;
  }
  .none {
    margin: 0;
    color: var(--text-label);
    font-size: 12px;
  }
  .link {
    align-self: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-link);
    font-size: 12px;
  }
  .link:hover {
    color: var(--accent-link-hover);
    text-decoration: underline;
  }
</style>
