// Site ▸ Updates (G10): the words and numbers the tab shows for each machine's Launcher and the
// update channel's packages. Pure functions (tested in updates.test.ts).

import type { Status } from "../../ui/types";
import type { LauncherAction, LauncherEntry, LauncherProduct, MachineHealth, ProductState, UpdateOutcome, UpdatePackage } from "$protocol";

const STATE: Record<ProductState, string> = {
  running: "Running",
  stopped: "Stopped",
  starting: "Starting",
  updating: "Updating",
  rollingBack: "Rolling back",
  failed: "Not running",
};

/** A product's state in words. */
export function stateLabel(s: ProductState): string {
  return STATE[s] ?? s;
}

/** The state's status dot: green running, amber busy, red not running, grey stopped. */
export function stateTone(s: ProductState): Status {
  if (s === "running") return "ok";
  if (s === "starting" || s === "updating" || s === "rollingBack") return "warn";
  if (s === "failed") return "error";
  return "off";
}

/** The last update in words ("Updated 1.0.0 → 1.1.0", "1.2.0 didn't start: back on 1.1.0"). */
export function outcomeText(o: UpdateOutcome): string {
  const from = o.from ?? "nothing";
  if (o.result === "installed") return o.message === "rolled back by hand" ? `Rolled back ${from} → ${o.to}` : `Updated ${from} → ${o.to}`;
  if (o.result === "rolledBack") return `${o.to} didn't come up healthy: back on ${o.from ?? "the previous version"}`;
  return `${o.to} failed${o.message ? `: ${o.message}` : ""}`;
}

/** The newer version in the channel for a machine's product, if any. */
export function updateFor(e: LauncherEntry, product: string): string | null {
  return e.updates.find((u) => u.product === product)?.version ?? null;
}

/** A queued action for a product, in words ("Update to 1.1.0 queued"). */
export function pendingFor(e: LauncherEntry, product: string): string | null {
  const a = e.pending.find((p: LauncherAction) => p.product === product);
  if (!a) return null;
  return a.type === "update" ? `Update to ${a.version} queued` : "Roll back queued";
}

/** A product can be acted on: its machine is online and nothing is queued or running for it. */
export function canAct(e: LauncherEntry, p: LauncherProduct): boolean {
  return e.online && !pendingFor(e, p.product) && p.state !== "updating" && p.state !== "rollingBack";
}

/** Bytes for people (1 decimal from MB up). */
export function bytes(n: number | undefined | null): string {
  if (n == null) return "–";
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}

/** The machine's numbers in one line ("CPU 12 % of 8 · memory 5.1 GB free of 16 GB · disk …"). */
export function healthLine(h: MachineHealth): string {
  const parts: string[] = [];
  parts.push(h.cpuPercent != null ? `CPU ${Math.round(h.cpuPercent)} % of ${h.cpus}` : `${h.cpus} CPUs`);
  if (h.memoryTotal != null) parts.push(`memory ${bytes(h.memoryAvailable)} free of ${bytes(h.memoryTotal)}`);
  if (h.diskTotal != null) parts.push(`disk ${bytes(h.diskFree)} free of ${bytes(h.diskTotal)}`);
  if (h.gpus.length) parts.push(`GPU ${h.gpus.join(", ")}`);
  return parts.join(" · ");
}

/** "just now", "2 min ago", "3 h ago". */
export function seenAgo(seconds: number): string {
  if (seconds < 45) return "just now";
  if (seconds < 3600) return `${Math.round(seconds / 60)} min ago`;
  if (seconds < 86400) return `${Math.round(seconds / 3600)} h ago`;
  return `${Math.round(seconds / 86400)} d ago`;
}

/** Platform names for people. */
export function platformLabel(p: string): string {
  const [os, arch] = p.split("-");
  const o = os === "linux" ? "Linux" : os === "windows" ? "Windows" : os === "macos" ? "macOS" : (os ?? p);
  return arch ? `${o} (${arch})` : o;
}

/** Machines needing attention: offline, or a product not running or whose update failed. */
export function attentionCount(entries: LauncherEntry[]): number {
  return entries.filter((e) => !e.online || e.report.products.some((p) => p.state === "failed" || p.lastUpdate?.result === "failed")).length;
}

/** Packages grouped by product, newest first within each (as the server lists them). */
export function byProduct(packages: UpdatePackage[]): [string, UpdatePackage[]][] {
  const m = new Map<string, UpdatePackage[]>();
  for (const p of packages) m.set(p.product, [...(m.get(p.product) ?? []), p]);
  return [...m.entries()];
}

const ROLES: Record<string, string> = {
  presentation: "presentation server",
  display: "display box",
  siteManager: "Site Manager",
  codex: "Codex",
  mixer: "mixer",
  switcher: "switcher",
  intercom: "intercom station",
};

/** A machine's role for people. */
export function roleLabel(r: string): string {
  return ROLES[r] ?? r;
}

const PRODUCTS: Record<string, string> = { vesper: "Vesper", oriel: "Vesper Oriel", launcher: "Vesper Launcher" };

/** A package's product for people. */
export function productName(p: string): string {
  return PRODUCTS[p] ?? p;
}
