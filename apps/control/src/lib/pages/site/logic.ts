// The Site page's logic (plain TypeScript, unit tested): device health and service states in
// words and colours,
// the toolbar's summary, the Site Manager running without its admin panel, the export form's checks, the import choices and the request they
// make, restore states, membership states and time stamps.

import type {
  DeviceHealth,
  DeviceService,
  ImportCollection,
  ImportPreview,
  ImportRequest,
  MembershipState,
  RestoreState,
  SiteAccess,
  SiteDevice,
  SiteEvent,
  SiteMembership,
  SiteStatus,
} from "$protocol";
import type { Status } from "../../ui/types";

export type SiteTab = "devices" | "join" | "displays" | "codex" | "backups" | "updates" | "security" | "signage";

export const HEALTH: Record<DeviceHealth, { label: string; status: Status }> = {
  online: { label: "Online", status: "ok" },
  needsAttention: { label: "Needs attention", status: "warn" },
  offline: { label: "Offline", status: "error" },
  restarting: { label: "Restarting…", status: "info" },
  updating: { label: "Updating…", status: "info" },
  removed: { label: "Removed", status: "off" },
};

/** A device's status line: its health, with the reason or "last seen". */
export function healthText(d: Pick<SiteDevice, "health" | "reason" | "lastSeen">, now = Date.now()): string {
  const base = HEALTH[d.health]?.label ?? d.health;
  if (d.health === "needsAttention" && d.reason) return `${base}: ${d.reason}`;
  if (d.health === "offline" && d.lastSeen) return `${base} · last seen ${formatWhen(d.lastSeen, now)}`;
  return base;
}

/** The device's name as Oriel shows it ("Codex (built-in)" for the Codex's own membership). */
export function deviceName(d: Pick<SiteDevice, "name" | "builtIn">): string {
  return d.builtIn ? "Codex (built-in)" : d.name;
}

const KIND: Record<string, string> = { app: "App", output: "Output", input: "Input" };

/** What a device is, under its name: its kind, an IDN Node, a site admin, relaying pairings. */
export function deviceKindLine(d: Pick<SiteDevice, "kind" | "node" | "admin" | "builtIn" | "relay">): string {
  if (d.builtIn) return "Part of Vesper on this server";
  return [KIND[d.kind] ?? d.kind, d.node && "IDN Node", d.admin && "site admin", d.relay && "Relays pairings"].filter((x) => x).join(" · ");
}

/** A Vesper box, by Vesper's rule (`vesper-site` `is_vesper_box`): enrolled as an app, with the
 * name a Vesper server joins with, `<its name> (Vesper)`. Only these may relay pairings (M-6). */
export function isVesperBox(d: Pick<SiteDevice, "kind" | "name">): boolean {
  const suffix = " (Vesper)";
  return d.kind === "app" && d.name.endsWith(suffix) && d.name.slice(0, -suffix.length).trim() !== "";
}

/** Site ▸ Devices offers Relay on/off (M-6, H08-4): for a Vesper box, or to take a relay mark
 * off any machine that has one; never the Codex's own membership or a removed machine. */
export function relayToggle(d: Pick<SiteDevice, "kind" | "name" | "builtIn" | "health" | "relay">): boolean {
  return !d.builtIn && d.health !== "removed" && (!!d.relay || isVesperBox(d));
}

/** A service's state as IDN's Node reports it, in words and colours (an unknown one as it is). */
export const SERVICE_STATE: Record<string, { label: string; status: Status }> = {
  running: { label: "Running", status: "ok" },
  starting: { label: "Starting…", status: "info" },
  restarting: { label: "Restarting…", status: "info" },
  failed: { label: "Failed", status: "error" },
  off: { label: "Off", status: "off" },
  unset: { label: "Not set up", status: "off" },
  unsupported: { label: "Not on this device", status: "off" },
  no_session: { label: "Waiting for a log-in", status: "warn" },
};

export function serviceState(state: string): { label: string; status: Status } {
  return SERVICE_STATE[state] ?? { label: state || "Unknown", status: "off" };
}

/** Whether "Restart" makes sense for a service: not one the device doesn't have, nor one
 *  restarting already (its own state, or the device's `restarting` list). */
export function canRestartService(s: Pick<DeviceService, "id" | "state">, restarting: readonly string[] = []): boolean {
  return s.state !== "unsupported" && s.state !== "restarting" && !restarting.includes(s.id);
}

/** "4 devices · 3 online · 1 needs attention" (removed ones left out). */
export function deviceSummary(devices: Pick<SiteDevice, "health">[]): string {
  const list = devices.filter((d) => d.health !== "removed");
  const online = list.filter((d) => d.health !== "offline").length;
  const attention = list.filter((d) => d.health === "needsAttention").length;
  const n = list.length;
  return `${n} device${n === 1 ? "" : "s"} · ${online} online${attention ? ` · ${attention} need${attention === 1 ? "s" : ""} attention` : ""}`;
}

export const JOIN_MODES = [
  { value: "code_or_approval" as const, name: "Site code or approval", desc: "A device that types the site code joins at once; one without a code waits for approval here." },
  { value: "code_and_approval" as const, name: "Site code, then approval", desc: "A device types the site code, then waits for approval here." },
  { value: "approval" as const, name: "Approval only", desc: "Devices ask to join; you compare the code both screens show and approve." },
];

export const ACCESS: Record<SiteAccess, { name: string; desc: string }> = {
  open: { name: "Open", desc: "Every paired computer may approve devices, remove them and change the site's settings." },
  pin: { name: "Admin PIN", desc: "Admin actions need the site's PIN; Oriel locks again after 30 minutes without one." },
  users: { name: "Users", desc: "Admin actions need a site user with the admin role." },
};

/** "482913" → "482 913". */
export function spacedCode(code: string | undefined | null): string {
  const c = (code ?? "").replace(/\s+/g, "");
  return c.length === 6 ? `${c.slice(0, 3)} ${c.slice(3)}` : c;
}

/** "9:41" for seconds left. */
export function mmss(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** "Today 06:00", "Yesterday 02:00", "Fri 21 Nov 02:00" (local time), "–" for nothing. */
export function formatWhen(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "–";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "–";
  const d = new Date(t);
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(new Date(now)) - day(d)) / 86_400_000);
  if (diff === 0) return `Today ${time}`;
  if (diff === 1) return `Yesterday ${time}`;
  if (diff === -1) return `Tomorrow ${time}`;
  const year = d.getFullYear() !== new Date(now).getFullYear() ? { year: "numeric" as const } : {};
  return `${d.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", ...year })} ${time}`;
}

/** "2 min ago", "3 h ago", "4 days ago". */
export function ago(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86_400) return `${Math.floor(s / 3600)} h ago`;
  const d = Math.floor(s / 86_400);
  return `${d} day${d === 1 ? "" : "s"} ago`;
}

/** A line of "Recently": "10:41 Chapel Box joined the site". */
export function eventLine(e: Pick<SiteEvent, "at" | "text">): string {
  const t = Date.parse(e.at);
  const time = Number.isNaN(t) ? "" : new Date(t).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  return `${time} ${e.text}`.trim();
}

/** How many events "Recently" keeps (the first load asks for as many). */
export const RECENT_EVENTS = 50;

/**
 * "Recently" after a `siteEvent`: the new event first (the list is newest first), at most `max`.
 * An event the list already holds (a refetch that raced it) isn't added twice.
 */
export function withSiteEvent(events: SiteEvent[], e: SiteEvent, max = RECENT_EVENTS): SiteEvent[] {
  const same = (x: SiteEvent) => x.name === e.name && x.at === e.at && x.text === e.text;
  if (events.some(same)) return events;
  return [e, ...events].slice(0, max);
}

/** Events worth listing under Join requests (joins, codes, pairings, devices coming and going). */
export function isJoinEvent(e: Pick<SiteEvent, "name">): boolean {
  return /^(join_|device_|key_request)/.test(e.name);
}

// ---------------------------------------------------------------- export and import

export const MIN_PASSPHRASE = 8;

/** Why a passphrase (typed twice) can't be used yet, or null. */
export function passphraseProblem(a: string, b: string | null = null): string | null {
  if (a.length < MIN_PASSPHRASE) return `At least ${MIN_PASSPHRASE} characters`;
  if (b !== null && a !== b) return "The two passphrases don't match";
  return null;
}

export type ImportChoice = "add" | "skip" | "keep" | "replace";

export const COMPARISON: Record<ImportCollection["comparison"], string> = {
  new: "New",
  same: "Same as here",
  newer: "Newer than here",
  older: "Older than here",
  inTrash: "In the collection trash",
};

/** What can be done with one collection of the file (the first is the default). */
export function importOptions(c: ImportCollection): ImportChoice[] {
  if (c.tooNew) return ["skip"];
  if (c.comparison === "new") return ["add", "skip"];
  if (c.comparison === "inTrash") return ["skip"];
  // A collection that is here: kept unless replaced (never the one this box shows).
  return c.active ? ["keep"] : ["keep", "replace"];
}

/** Why a collection can't be imported as it is (or null). */
export function importNote(c: ImportCollection): string | null {
  if (c.tooNew) return "Written by a newer Vesper";
  if (c.comparison === "inTrash") return "Restore it from the collection trash or delete it for good first";
  if (c.active && c.comparison !== "new") return "Shown on this box: assign another collection to replace it";
  return null;
}

export const CHOICE_LABEL: Record<ImportChoice, string> = { add: "Add", skip: "Skip", keep: "Keep this one", replace: "Replace" };

/** The default choice per collection of the file. */
export function defaultChoices(p: Pick<ImportPreview, "collections">): Record<string, ImportChoice> {
  const out: Record<string, ImportChoice> = {};
  for (const c of p.collections) out[c.collection.id] = importOptions(c)[0] ?? "skip";
  return out;
}

/** The apply request for the operator's choices. */
export function importRequest(
  passphrase: string,
  p: Pick<ImportPreview, "collections">,
  choices: Record<string, ImportChoice>,
  opts: { backups: boolean; site: boolean },
): ImportRequest {
  const replace: string[] = [];
  const skip: string[] = [];
  for (const c of p.collections) {
    const id = c.collection.id;
    const pick = choices[id] ?? importOptions(c)[0] ?? "skip";
    if (pick === "replace" && importOptions(c).includes("replace")) replace.push(id);
    if (pick === "skip" && c.comparison === "new") skip.push(id);
  }
  const req: ImportRequest = { passphrase };
  if (replace.length) req.replace = replace;
  if (skip.length) req.skip = skip;
  if (!opts.backups) req.backups = false;
  if (opts.site) req.site = true;
  return req;
}

// ---------------------------------------------------------------- restores and membership

export const RESTORE_STATE: Record<RestoreState, { label: string; status: Status }> = {
  queued: { label: "Queued", status: "info" },
  applying: { label: "Applying…", status: "info" },
  applied: { label: "Applied", status: "ok" },
  failed: { label: "Failed", status: "error" },
  expired: { label: "Expired", status: "off" },
  cancelled: { label: "Cancelled", status: "off" },
  unknown: { label: "Unknown", status: "off" },
};

const MEMBERSHIP: Record<MembershipState, string> = {
  none: "This server runs its own site.",
  starting: "Joining…",
  noSite: "No site answered at that address.",
  waitingForWindow: "Waiting for the site to open its approval window.",
  waitingForApproval: "Waiting for approval at the site's server. Compare this code there:",
  codeNeeded: "The site needs its join code.",
  codeAccepted: "Code accepted: joining…",
  wrongCode: "Wrong code.",
  codeExpired: "The code expired: ask for a new one.",
  enrolled: "In the site",
  denied: "The site refused this server.",
  removed: "The site removed this server.",
  error: "Joining failed.",
};

export function membershipText(m: SiteMembership): string {
  let t = MEMBERSHIP[m.state] ?? m.state;
  if (m.state === "enrolled") t = `In the site at ${m.server ?? "another server"} · ${m.connected ? "connected" : "not connected"}`;
  if (m.state === "wrongCode" && m.triesLeft !== undefined) t += ` ${m.triesLeft} tries left.`;
  if (m.error) t += ` ${m.error}`;
  return t;
}

/** The warning while this server's Site Manager runs without its API (the admin panel), e.g.
 *  its port taken by another program; null otherwise. */
export function apiWarning(s: Pick<SiteStatus, "running" | "apiError"> | null): string | null {
  if (!s?.running || !s.apiError) return null;
  return `Running, but the admin panel isn't available: ${s.apiError}`;
}

/** The site settings were damaged at start (`settingsDamaged`): the server started locked down
 *  until an admin sets the access again (on the server machine, or signed in as a site admin). */
export function damagedWarning(s: Pick<SiteStatus, "settingsDamaged" | "you"> | null): string | null {
  if (!s?.settingsDamaged) return null;
  const fix =
    s.you.role === "admin"
      ? "Set the access again in Security to fix it (keeping Users is enough)."
      : "Set the access again in Security, on the server machine or signed in as a site admin, to fix it.";
  return `The site settings were damaged, so this server started locked down: paired computers can only view. ${fix}`;
}

/** True while a join is under way or done (Leave applies). */
export function inOtherSite(m: SiteMembership): boolean {
  return m.state !== "none" && m.state !== "removed" && m.state !== "denied" && m.state !== "noSite";
}

/** A backup set is this box's: the server's `thisBox`, unless the list came from the Codex box
 *  through the side connection (a show box), where `thisBox` means the Codex box. */
export function isThisBox(s: { thisBox: boolean; set: { machine: string } }, thisId: string, viaCodexBox: boolean): boolean {
  return viaCodexBox ? !!thisId && s.set.machine === thisId : s.thisBox;
}
