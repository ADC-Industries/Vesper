// Typed REST client for the site (crates/protocol/README.md, "Site"): status and settings, the
// session (unlock, sign in), devices, join requests and codes, the approval window, Oriel
// pairings waiting for approval, the site's API keys, users, audit, events and this server's
// membership of another site. Plain TypeScript over a RestClient; errors stay `ApiRequestError`s.

import type {
  ApproveJoin,
  DenyPairing,
  IdentifyDevice,
  JoinCodeInfo,
  JoinRequestInfo,
  JoinSite,
  LogSnapshot,
  PairingRequest,
  PutSiteUser,
  RestartDevice,
  SetDeviceRelay,
  SiteApiKey,
  SiteAuditEntry,
  SiteDevice,
  SiteEvent,
  SiteMembership,
  SiteSession,
  SiteSettings,
  SiteSignIn,
  SiteStatus,
  SiteUser,
} from "$protocol";
import type { RestClient } from "./rest";

const enc = encodeURIComponent;

export class SiteClient {
  constructor(readonly rest: RestClient) {}

  status = () => this.rest.get<SiteStatus>("/site");
  settings = () => this.rest.get<SiteSettings>("/site/settings");
  /** Changes the join mode or the access level (admins; `If-Match` with the settings' revision).
   *  The server takes whole settings (`enabled`, `joinMode`, `access`), so the fields not being
   *  changed are read first. */
  saveSettings = async (changes: Partial<Pick<SiteSettings, "joinMode" | "access" | "pin">>, revision: string | null) => {
    const cur = await this.settings();
    const body = { enabled: cur.enabled, joinMode: cur.joinMode, access: cur.access, ...changes };
    return this.rest.request<SiteSettings>("PUT", "/site/settings", body, undefined, { ifMatch: revision });
  };

  session = () => this.rest.get<SiteSession>("/site/session");
  signIn = (body: SiteSignIn) => this.rest.request<SiteSession>("POST", "/site/session", body);
  signOut = () => this.rest.request<void>("DELETE", "/site/session");
  unlock = (pin: string) => this.rest.request<SiteSession>("POST", "/site/unlock", { pin });
  lock = () => this.rest.request<void>("DELETE", "/site/unlock");

  devices = () => this.rest.get<SiteDevice[]>("/site/devices");
  removeDevice = (fp: string) => this.rest.request<void>("DELETE", `/site/devices/${enc(fp)}`);
  allowDevice = (fp: string) => this.rest.request<void>("POST", `/site/devices/${enc(fp)}/allow`);
  identify = (fp: string, body: IdentifyDevice = {}) => this.rest.request<void>("POST", `/site/devices/${enc(fp)}/identify`, body);
  restart = (fp: string, body: RestartDevice) => this.rest.request<void>("POST", `/site/devices/${enc(fp)}/restart`, body);
  /** Marks the machine a pairing relay, or not (admins). */
  setRelay = (fp: string, body: SetDeviceRelay) => this.rest.request<void>("POST", `/site/devices/${enc(fp)}/relay`, body);
  /** Asks the machine for a new log snapshot. */
  takeLogs = (fp: string) => this.rest.request<void>("POST", `/site/devices/${enc(fp)}/logs`);
  logs = (fp: string) => this.rest.get<LogSnapshot[]>(`/site/devices/${enc(fp)}/logs`);
  /** A log file as text. */
  async logFile(fp: string, snapshot: string, file: string, auth: { token: () => string | null }): Promise<string> {
    const headers: Record<string, string> = { Accept: "text/plain" };
    const t = auth.token();
    if (t) headers.Authorization = `Bearer ${t}`;
    const res = await fetch(this.rest.url(`/site/devices/${enc(fp)}/logs/${enc(snapshot)}/${enc(file)}`), { headers });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    return res.text();
  }

  joinRequests = () => this.rest.get<JoinRequestInfo[]>("/site/join-requests");
  approveJoin = (id: number, body: ApproveJoin = {}) => this.rest.request<void>("POST", `/site/join-requests/${id}/approve`, body);
  denyJoin = (id: number) => this.rest.request<void>("POST", `/site/join-requests/${id}/deny`);

  /** Opens the approval window (`seconds`, 600 by default on the server). */
  openWindow = (seconds?: number) => this.rest.request<void>("POST", "/site/pairing", seconds ? { seconds } : {});
  closeWindow = () => this.rest.request<void>("DELETE", "/site/pairing");

  /** The site code (`404` while there is none). */
  joinCode = () => this.rest.get<JoinCodeInfo>("/site/join-code");
  newJoinCode = () => this.rest.request<JoinCodeInfo>("POST", "/site/join-code");

  pairings = () => this.rest.get<PairingRequest[]>("/site/pairings");
  approvePairing = (id: string) => this.rest.request<void>("POST", `/site/pairings/${enc(id)}/approve`);
  denyPairing = (id: string, body: DenyPairing = {}) => this.rest.request<void>("POST", `/site/pairings/${enc(id)}/deny`, body);

  apiKeys = () => this.rest.get<SiteApiKey[]>("/site/api-keys");
  revokeKey = (id: string) => this.rest.request<void>("DELETE", `/site/api-keys/${enc(id)}`);

  users = () => this.rest.get<SiteUser[]>("/site/users");
  putUser = (name: string, body: PutSiteUser, revision: string | null = null) =>
    this.rest.request<SiteUser>("PUT", `/site/users/${enc(name)}`, body, undefined, { ifMatch: revision });
  removeUser = (name: string) => this.rest.request<void>("DELETE", `/site/users/${enc(name)}`);

  audit = (n = 200) => this.rest.get<SiteAuditEntry[]>("/site/audit", { n });
  events = (n = 200) => this.rest.get<SiteEvent[]>("/site/events", { n });

  membership = () => this.rest.get<SiteMembership>("/site/membership");
  join = (body: JoinSite) => this.rest.request<SiteMembership>("POST", "/site/join", body);
  leave = () => this.rest.request<void>("POST", "/site/leave");
}
