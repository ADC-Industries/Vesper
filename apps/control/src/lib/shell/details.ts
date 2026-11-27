// Machine details in words (G9): this Vesper's (`GET /server/details`: Site ▸ Devices' "This
// Vesper" card and the status bar) and every other machine's, as its IDN Node reports them
// (`SiteDevice.machine`, the device rows), in the same words. Unit tested.

import type { ServerDetails, SiteDeviceMachine } from "$protocol";

/** "3 d 4 h", "2 h 5 min", "12 min". */
export function uptimeText(s: number): string {
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d) return `${d} d ${h} h`;
  return h ? `${h} h ${m} min` : `${m} min`;
}

/** "CPU: Example CPU (16 CPUs)", "CPU: 1 CPU"; null when nothing is known. */
export function cpuLine(cpu: string | null | undefined, cpus: number | null | undefined): string | null {
  const n = cpus ? `${cpus} CPU${cpus === 1 ? "" : "s"}` : null;
  if (cpu) return `CPU: ${cpu}${n ? ` (${n})` : ""}`;
  return n ? `CPU: ${n}` : null;
}

/** Bytes as "31.9 GB" (GiB, as systems show memory). */
export function gbText(bytes: number): string {
  return `${(bytes / 2 ** 30).toFixed(1)} GB`;
}

/** The status bar's short text and its tooltip. */
export function detailsShort(d: ServerDetails): { text: string; title: string } {
  return {
    text: `Vesper ${d.version} · up ${uptimeText(d.uptimeSeconds)}`,
    title: detailsLines(d).join("\n"),
  };
}

/** One line per fact, for the card and the tooltip. */
export function detailsLines(d: ServerDetails): string[] {
  return [
    `${d.name}: Vesper ${d.version} on ${d.os}`,
    cpuLine(d.cpu, d.cpus) ?? "CPU: 1 CPU",
    `GPU: ${d.gpu ?? "not rendering yet"}`,
    `Up for ${uptimeText(d.uptimeSeconds)}`,
    `Runs ${d.products.join(", ")}`,
  ];
}

/** Seconds from `iso` to `now`, or null. */
function since(iso: string | null | undefined, now: number): number | null {
  if (!iso) return null;
  const t = Date.parse(iso);
  return Number.isNaN(t) ? null : Math.max(0, Math.round((now - t) / 1000));
}

/**
 * Another machine's details, one line per fact its Node reported (nothing for what it didn't:
 * never zeros). `online`: the machine is connected now; its free memory and uptime are shown
 * only then (an idle Node reports every 30 s, so free memory is "up to 30 s" old). Its Node's
 * start is told when it came well after the machine's (the Node restarted since).
 */
export function machineLines(
  m: SiteDeviceMachine,
  opts: { online: boolean; nodeStartedAt?: string | null; now?: number },
): string[] {
  const now = opts.now ?? Date.now();
  const out: string[] = [];
  const system = [m.os, m.osVersion].filter((s) => s).join(" ");
  const kernel = m.kernel ? `kernel ${m.kernel}` : "";
  if (system || kernel) out.push(`System: ${[system, kernel].filter((s) => s).join(", ")}`);
  const cpu = cpuLine(m.cpu, m.cpus);
  if (cpu) out.push(cpu);
  if (m.gpus.length) out.push(`GPU${m.gpus.length === 1 ? "" : "s"}: ${m.gpus.join(", ")}`);
  if (m.memoryTotal) {
    out.push(
      opts.online && m.memoryFree != null
        ? `Memory: ${gbText(m.memoryFree)} free of ${gbText(m.memoryTotal)} (updated up to 30 s ago)`
        : `Memory: ${gbText(m.memoryTotal)}`,
    );
  }
  const up = opts.online ? since(m.bootedAt, now) : null;
  if (up != null) {
    const node = since(opts.nodeStartedAt, now);
    // (Its Node started more than 5 minutes after the machine: it restarted since.)
    out.push(node != null && up - node > 300 ? `Up for ${uptimeText(up)} (IDN Node for ${uptimeText(node)})` : `Up for ${uptimeText(up)}`);
  }
  return out;
}
