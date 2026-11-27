<script lang="ts">
  // The Reflow page (its toolbar row is in ReflowEditor, with the save status).
  import type { Connection } from "../../server";
  import ReflowEditor from "./ReflowEditor.svelte";

  let { conn }: { conn: Connection } = $props();
</script>

<ReflowEditor {conn} />
