<script lang="ts">
  import Dialog from "../ui/Dialog.svelte";
  import Logo from "./Logo.svelte";
  import { APP_FULL_NAME } from "../config";
  import { PROTOCOL_VERSION } from "$protocol";
  import { session } from "../state/session.svelte";
  import { shell } from "../state/shell.svelte";
</script>

<Dialog title={`About ${APP_FULL_NAME}`} onclose={() => (shell.showAbout = false)} width={420}>
  <div class="about">
    <Logo size={56} />
    <h3>{APP_FULL_NAME}</h3>
    <p>Open-source presentation software for churches.</p>
    <dl>
      <dt>Protocol</dt><dd>{PROTOCOL_VERSION}</dd>
      <dt>Server</dt><dd>{session.server ? `${session.server.name} (${session.server.version})` : "not connected"}</dd>
    </dl>
  </div>
</Dialog>

<style>
  .about {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    text-align: center;
  }
  h3 {
    margin: 6px 0 0;
    font-size: 18px;
  }
  p {
    margin: 0;
    color: var(--text-2);
  }
  dl {
    display: grid;
    grid-template-columns: auto auto;
    gap: 4px 12px;
    margin: 12px 0 0;
    font-size: 12.5px;
  }
  dt {
    color: var(--text-label);
    text-align: right;
  }
  dd {
    margin: 0;
    text-align: left;
  }
</style>
