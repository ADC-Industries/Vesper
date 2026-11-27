import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

// Parts of Oriel loaded on first use (`{#await x()}` on a dynamic import) must say so when their
// chunk can't be loaded (a browser tab open across a server upgrade), not render nothing (D11-3).
function svelteFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? svelteFiles(p) : n.endsWith(".svelte") ? [p] : [];
  });
}

describe("lazy-loaded chunks (D11-3)", () => {
  it("every {#await} of a lazy import has a {:catch}", () => {
    const missing: string[] = [];
    let seen = 0;
    for (const f of svelteFiles(join(__dirname, ".."))) {
      const src = readFileSync(f, "utf8");
      const lazy = [...src.matchAll(/const (\w+) = \(\) => import\(/g)].map((m) => m[1]!);
      for (const name of lazy) {
        for (const m of src.matchAll(new RegExp(`\\{#await ${name}\\(\\)`, "g"))) {
          seen++;
          const end = src.indexOf("{/await}", m.index);
          if (!src.slice(m.index, end).includes("{:catch")) missing.push(`${f}: ${name}`);
        }
      }
    }
    expect(seen).toBeGreaterThan(4);
    expect(missing).toEqual([]);
  });
});
