<script lang="ts">
  // The Oriel mark from the owner's canvas: a compass star in the violet link colour on the
  // raised surface.
  let { size = 22 }: { size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 64 64" aria-hidden="true">
  <rect width="64" height="64" rx="14" fill="var(--surface-raised, #22252b)" />
  <g
    transform="translate(14 14) scale(1.5)"
    fill="none"
    stroke="var(--accent-link, #c9c0ff)"
    stroke-width="1.8"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d="M12 2v6M12 16v6M2 12h6M16 12h6M5 5l3.5 3.5M15.5 15.5L19 19M19 5l-3.5 3.5M8.5 15.5L5 19" />
  </g>
</svg>
