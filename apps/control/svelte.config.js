import { vitePreprocess } from "@sveltejs/vite-plugin-svelte";

// Our components use runes; Svelte detects that per file (lucide-svelte ships legacy-syntax
// components, so runes mode is not forced globally).
export default {
  preprocess: vitePreprocess(),
};
