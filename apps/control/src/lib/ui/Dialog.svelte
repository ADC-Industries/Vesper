<script lang="ts">
  // A modal dialog: title bar with a close button, content, optional footer (buttons on the
  // right). Takes the focus when it opens, keeps Tab inside, gives the focus back on close.
  // Esc is handled by the main view's key handler (it closes every dialog); `onclose` also runs
  // on Esc here when the dialog is shown outside the main view.
  import type { Snippet } from "svelte";
  import X from "lucide-svelte/icons/x";

  let {
    title,
    onclose,
    width = 560,
    bare = false,
    escape = false,
    children,
    footer,
  }: {
    title: string;
    onclose: () => void;
    width?: number;
    /** No padding: the content lays itself out (and scrolls itself). */
    bare?: boolean;
    /** Close on Esc here (for dialogs outside the main view's key handling). */
    escape?: boolean;
    children: Snippet;
    footer?: Snippet;
  } = $props();

  const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  function attach(node: HTMLElement) {
    const prev = document.activeElement as HTMLElement | null;
    node.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape" && escape) {
        e.stopPropagation();
        onclose();
        return;
      }
      if (e.key !== "Tab") return;
      const els = [...node.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.offsetParent !== null);
      if (!els.length) return;
      const first = els[0]!;
      const last = els[els.length - 1]!;
      if (e.shiftKey && (document.activeElement === first || document.activeElement === node)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    };
    node.addEventListener("keydown", key);
    return () => {
      node.removeEventListener("keydown", key);
      prev?.focus?.();
    };
  }
</script>

<div class="ui-backdrop" role="presentation" onclick={onclose}></div>
<div
  class="ui-dialog"
  role="dialog"
  aria-modal="true"
  aria-label={title}
  tabindex="-1"
  style:width={`min(${width}px, calc(100vw - 32px))`}
  {@attach attach}
>
  <header>
    <h2>{title}</h2>
    <button type="button" class="close" onclick={onclose} aria-label="Close (Esc)"><X size={16} strokeWidth={1.8} /></button>
  </header>
  <div class="content" class:bare>
    {@render children()}
  </div>
  {#if footer}
    <footer>{@render footer()}</footer>
  {/if}
</div>

<style>
  .ui-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(5, 6, 8, 0.6);
    z-index: 60;
  }
  .ui-dialog {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    max-height: calc(100vh - 80px);
    display: flex;
    flex-direction: column;
    background: var(--surface-panel);
    border: 1px solid var(--line-strong);
    border-radius: var(--r-dialog);
    box-shadow: var(--shadow-dialog);
    z-index: 61;
    outline: none;
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    min-height: 46px;
    padding: 0 10px 0 18px;
    border-bottom: 1px solid var(--line);
  }
  h2 {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: var(--fs-title);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    padding: 0;
    border: 0;
    border-radius: var(--r);
    background: transparent;
    color: var(--text-label);
  }
  .close:hover {
    background: var(--surface-overlay);
    color: var(--text);
  }
  .content {
    padding: 16px 18px 18px;
    overflow-y: auto;
  }
  .content.bare {
    padding: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--sp-4);
    padding: 12px 18px;
    border-top: 1px solid var(--line);
    background: var(--surface-sunk);
  }
</style>
