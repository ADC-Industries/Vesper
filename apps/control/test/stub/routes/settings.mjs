// Settings ▸ Network & IDN ▸ Control surfaces: `GET` / `PUT /remote-compat` (the Remote API
// compatibility layer). It starts on at port 50001, listening, so the card shows its live
// state; a PUT changes it in memory (port 80 answers "can't be opened", to see the problem).

export default function settingsRoutes({ json }) {
  let status = { enabled: true, port: 50001, listening: true, listeningPort: 50001 };
  const apply = (s) => {
    if (!s.enabled) return { enabled: false, port: s.port, listening: false };
    if (s.port < 1024) return { enabled: true, port: s.port, listening: false, problem: `port ${s.port} can't be opened: Permission denied (os error 13)` };
    return { enabled: true, port: s.port, listening: true, listeningPort: s.port };
  };
  return (req, res, { path }) => {
    if (path !== "/remote-compat") return false;
    if (req.method === "GET") {
      json(res, 200, status);
      return true;
    }
    if (req.method !== "PUT") return false;
    let body = "";
    req.on("data", (d) => (body += d));
    req.on("end", () => {
      let v = {};
      try {
        v = body ? JSON.parse(body) : {};
      } catch {
        /* empty */
      }
      status = apply({ enabled: v.enabled ?? status.enabled, port: v.port ?? status.port });
      json(res, 200, status);
    });
    return true;
  };
}
