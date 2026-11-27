import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { fileURLToPath } from "node:url";

// Where `npm run dev` proxies /api to. Override with VESPER_SERVER=http://host:port.
// TODO: drop the deprecated STAGE_SERVER fallback one release after the rename.
if (!process.env.VESPER_SERVER && process.env.STAGE_SERVER) {
  console.warn("STAGE_SERVER is deprecated and will be removed in the next release; use VESPER_SERVER");
}
const server = process.env.VESPER_SERVER ?? process.env.STAGE_SERVER ?? "http://127.0.0.1:5555";
const bindings = fileURLToPath(new URL("../../crates/protocol/bindings", import.meta.url));

export default defineConfig({
  plugins: [svelte()],
  // Relative asset paths, so the built files work when the server (or Tauri) serves them
  // from any path.
  base: "./",
  resolve: {
    alias: { $protocol: bindings },
  },
  server: {
    port: Number(process.env.PORT ?? 5173),
    strictPort: true,
    fs: { allow: [".", bindings] },
    proxy: {
      "/api": { target: server, ws: true, changeOrigin: false },
    },
  },
  preview: {
    proxy: {
      "/api": { target: server, ws: true, changeOrigin: false },
    },
  },
  build: {
    outDir: "dist",
    target: "es2022",
    chunkSizeWarningLimit: 800,
  },
  test: {
    include: ["src/**/*.test.ts"],
    environment: "node",
  },
});
