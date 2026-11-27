<script lang="ts">
  // "Someone else changed this": the operator picks the server's text or their own (Reflow, the
  // Edit view's in-place text editor and the Library's song Edit). A panel over the editor in
  // the redesign's look (like the Site page's confirmations: dark red border, a bold question,
  // the two texts in wells, the buttons on the right).
  import TriangleAlert from "lucide-svelte/icons/triangle-alert";
  import Button from "../ui/Button.svelte";

  let {
    title,
    message,
    theirs,
    mine,
    onTheirs,
    onMine,
  }: { title: string; message: string; theirs: string; mine: string; onTheirs: () => void; onMine: () => void } = $props();
</script>

<div class="conflict" role="alertdialog" aria-label="Edit conflict">
  <div class="head">
    <span class="icon" aria-hidden="true"><TriangleAlert size={16} strokeWidth={1.8} /></span>
    <span class="words">
      <span class="title">{title}</span>
      <span class="msg">{message}</span>
    </span>
  </div>
  <div class="cols">
    <div class="col">
      <span class="section-label">Theirs · on the server now</span>
      <pre>{theirs || "(empty)"}</pre>
    </div>
    <div class="col">
      <span class="section-label">Yours</span>
      <pre class="mine">{mine || "(empty)"}</pre>
    </div>
  </div>
  <div class="buttons">
    <Button onclick={onTheirs}>Use theirs</Button>
    <Button variant="primary" onclick={onMine}>Keep mine</Button>
  </div>
</div>

<style>
  .conflict {
    position: absolute;
    top: 48px;
    left: 50%;
    transform: translateX(-50%);
    width: min(640px, calc(100% - 40px));
    z-index: 40;
    display: flex;
    flex-direction: column;
    gap: var(--sp-6);
    padding: 14px 16px;
    border: 1px solid var(--danger-bg);
    border-radius: var(--r-card);
    background: var(--surface-panel);
    box-shadow: var(--shadow-dialog);
  }
  .head {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-5);
  }
  .icon {
    flex: none;
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border-radius: 50%;
    background: var(--danger-soft);
    color: var(--danger-text);
  }
  .words {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .title {
    font-size: var(--fs-md);
    font-weight: 700;
    color: var(--text);
  }
  .msg {
    color: var(--text-2);
    line-height: 1.45;
  }
  .cols {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--sp-6);
  }
  .col {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    min-width: 0;
  }
  pre {
    margin: 0;
    max-height: 160px;
    overflow: auto;
    padding: 10px 12px;
    border: 1px solid var(--line);
    border-radius: var(--r);
    background: var(--surface-well);
    color: var(--text);
    font-family: var(--font);
    font-size: var(--fs);
    line-height: 1.5;
    white-space: pre-wrap;
    text-align: center;
    user-select: text;
  }
  pre.mine {
    border-color: var(--accent-ring);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: var(--sp-4);
  }
</style>
