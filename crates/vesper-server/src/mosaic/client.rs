//! The client for one box's IDN display service (`https://HOST:29223/api/v1/`), as the clean
//! room documented it (2026-11-25, IDN v0.3.4-Beta11; `docs/MOSAIC-PLAN.md`). Every detail of
//! the service's API is here.
//!
//! - Every request carries the box's site token (`Authorization: Bearer idn1.…`, engine
//!   `display`); every change also `X-IDN-Display: 1`. Errors are `{"ok": false, "error"}`.
//! - `GET /api/v1/state`: the service's own `error` (a box without a desktop session: "no desktop
//!   on this computer") and the screens (id, name, number, size, feed, `routed_to`, error,
//!   `status.state`, test pattern, `settings`, `stats.sync`).
//! - Video walls (`settings.wall`): `{rows, cols, row, col, bezel_mm: {h, v}, screen_mm?: {w, h},
//!   group?}`; every tile of a wall gets the same geometry with its own row and column, the same
//!   group and the same feed, and then shows each picture within one refresh of the others
//!   (`stats.sync {group, mode, delay_ms, late}` per screen).
//! - `POST /api/v1/feed`, `settings`, `identify`, `test_pattern`; `GET /api/v1/thumbnail`.
//!
//! **The box's certificate is checked** (IDN v0.3.4-Beta12): HTTPS goes through
//! `Authority::lan_client_config(fp)` of the site this server hosts, which trusts only the site
//! CA and only the box's own LAN certificate (its machine fingerprint), at whatever address it
//! was reached: another machine of the site with a valid certificate of its own can't answer
//! for it, and a box that offers only its self-signed certificate isn't talked to. Plain
//! `http://` is used only for a loopback URL an admin (or a test) gives.
//!
//! Blocking: call from `spawn_blocking`.

use idn_open_protocol::web::httpc;
use protocol::{
    MosaicFallback, MosaicFit, MosaicScreenSettings, MosaicScreenStatus, MosaicSound, MosaicTile,
};
use serde_json::{Value, json};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

/// The display service's port for the site (`https`).
pub const PORT: u16 = 29223;
/// How long a request may take.
const TIMEOUT: Duration = Duration::from_secs(4);

/// The service's base URL for a box's address: a full URL as it is (`http://…` in tests), an
/// address with a port over HTTPS, else HTTPS on [`PORT`].
pub fn base_url(address: &str) -> String {
    let a = address.trim().trim_end_matches('/');
    if a.contains("://") {
        return a.to_string();
    }
    let has_port = if let Some(rest) = a.strip_prefix('[') {
        rest.split_once(']')
            .is_some_and(|(_, p)| p.starts_with(':'))
    } else {
        a.matches(':').count() == 1
    };
    if has_port {
        format!("https://{a}")
    } else if a.contains(':') {
        format!("https://[{a}]:{PORT}")
    } else {
        format!("https://{a}:{PORT}")
    }
}

/// One screen as the service's `state` gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenState {
    pub number: u32,
    pub name: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Its feed (empty: none).
    pub feed: String,
    pub status: MosaicScreenStatus,
    /// Its error, or the status's reason.
    pub problem: Option<String>,
    pub test_pattern: bool,
    /// Its settings as the box has them (`None`: it didn't say).
    pub settings: Option<Value>,
    /// Its frame sync (`stats.sync`).
    pub sync: Option<SyncStats>,
}

/// A screen's `stats.sync`.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncStats {
    pub group: String,
    pub mode: String,
    pub delay_ms: f32,
    pub late: u64,
}

/// What a box's `state` gives.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoxState {
    /// The service's own error (empty: none), e.g. "no desktop on this computer".
    pub error: Option<String>,
    pub screens: Vec<ScreenState>,
}

/// A read that failed.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadError {
    /// The box refused the token (`401`: bad, expired or another engine's): a new one is needed.
    pub token_refused: bool,
    pub message: String,
}

/// One box's display service.
pub struct Client<'a> {
    pub base: &'a str,
    pub token: &'a str,
    /// How its HTTPS certificate is checked: only that box's (`lan_client_config`). `None`: an
    /// `https://` base isn't used.
    pub tls: Option<&'a Arc<rustls::ClientConfig>>,
}

fn text(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Client<'_> {
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Vec<u8>), String> {
        let url = format!("{}/api/v1/{path}", self.base);
        let mut headers = vec![(
            "Authorization".to_string(),
            format!("Bearer {}", self.token),
        )];
        let bytes = body.map(|b| b.to_string().into_bytes());
        if body.is_some() {
            headers.push(("X-IDN-Display".into(), "1".into()));
            headers.push(("Content-Type".into(), "application/json".into()));
        }
        let r = if url.starts_with("https://") {
            let tls = self
                .tls
                .ok_or("the box's certificate can't be checked (no site authority here)")?;
            https(tls, method, &url, &headers, bytes.as_deref())
        } else if loopback_http(&url) {
            httpc::request(method, &url, &headers, bytes.as_deref(), false, TIMEOUT)
        } else {
            return Err(
                "only https:// reaches a display box (http:// only on this machine)".into(),
            );
        };
        r.map_err(|e| {
            if e.get_ref()
                .is_some_and(|x| x.downcast_ref::<rustls::Error>().is_some())
            {
                format!("the box's certificate isn't its own site certificate: not talked to ({e})")
            } else {
                format!("the display service didn't answer ({e})")
            }
        })
    }

    /// Its answer's error text (`{"ok": false, "error"}`), else the status.
    fn refused(status: u16, body: &[u8]) -> String {
        let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        match v["error"].as_str() {
            Some(e) => format!("the box refused it: {e}"),
            None => format!("the box answered {status}"),
        }
    }

    /// `GET /api/v1/state`: its own error and its screens.
    pub fn state(&self) -> Result<BoxState, ReadError> {
        let failed = |message: String| ReadError {
            token_refused: false,
            message,
        };
        let (status, body) = self.call("GET", "state", None).map_err(failed)?;
        if status != 200 {
            return Err(ReadError {
                token_refused: status == 401,
                message: Self::refused(status, &body),
            });
        }
        let v: Value = serde_json::from_slice(&body).map_err(|e| failed(text(e)))?;
        Ok(box_state(&v))
    }

    /// A change (`POST /api/v1/{cmd}`).
    pub fn post(&self, cmd: &str, body: &Value) -> Result<(), String> {
        let (status, answer) = self.call("POST", cmd, Some(body))?;
        let v: Value = serde_json::from_slice(&answer).unwrap_or(Value::Null);
        if status == 200 && v["ok"] != false {
            Ok(())
        } else {
            Err(Self::refused(status, &answer))
        }
    }

    /// Shows `source` on screen `number` (`""`: back to the desktop).
    pub fn feed(&self, number: u32, source: &str) -> Result<(), String> {
        self.post("feed", &json!({"screen": number, "source": source}))
    }

    /// `GET /api/v1/thumbnail`: a JPEG of what screen `number` shows (`None`: it shows nothing).
    pub fn thumbnail(&self, number: u32, width: u32) -> Result<Option<Vec<u8>>, String> {
        let (status, body) = self.call(
            "GET",
            &format!("thumbnail?screen={number}&width={}", width.clamp(16, 1920)),
            None,
        )?;
        match status {
            200 => Ok(Some(body)),
            404 => Ok(None),
            s => Err(Self::refused(s, &body)),
        }
    }
}

/// Whether `url` is plain HTTP to this machine (`127.0.0.1`, `::1`, `localhost`).
fn loopback_http(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let host = host_port(rest.split('/').next().unwrap_or("")).0;
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// `host[:port]` or `[v6][:port]`: the host (without brackets) and the port, if given.
fn host_port(hp: &str) -> (&str, Option<u16>) {
    if let Some(rest) = hp.strip_prefix('[') {
        let (h, after) = rest.split_once(']').unwrap_or((rest, ""));
        return (h, after.strip_prefix(':').and_then(|p| p.parse().ok()));
    }
    match hp.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h, p.parse().ok()),
        _ => (hp, None),
    }
}

/// The longest answer read.
const MAX_ANSWER: usize = 16 << 20;

/// One HTTPS request (HTTP/1.1, one per connection) with `tls` deciding which certificate
/// passes: `(status, body)`.
fn https(
    tls: &Arc<rustls::ClientConfig>,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    body: Option<&[u8]>,
) -> io::Result<(u16, Vec<u8>)> {
    let bad = |m: &str| io::Error::new(io::ErrorKind::InvalidData, m.to_string());
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| bad("not https"))?;
    let (hp, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = host_port(hp);
    let addr = (host, port.unwrap_or(443))
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| bad("no address"))?;
    let tcp = TcpStream::connect_timeout(&addr, TIMEOUT)?;
    tcp.set_read_timeout(Some(TIMEOUT))?;
    tcp.set_write_timeout(Some(TIMEOUT))?;
    // The name only goes out as SNI: the certificate is checked against the machine, not it.
    let name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|_| bad("not a host name"))?;
    let conn = rustls::ClientConnection::new(tls.clone(), name).map_err(io::Error::other)?;
    let mut s = rustls::StreamOwned::new(conn, tcp);
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {hp}\r\nConnection: close\r\n");
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!(
        "Content-Length: {}\r\n\r\n",
        body.map_or(0, <[u8]>::len)
    ));
    let mut out = req.into_bytes();
    out.extend_from_slice(body.unwrap_or_default());
    s.write_all(&out)?;
    s.flush()?;
    let mut raw = Vec::new();
    let mut buf = [0u8; 16 << 10];
    loop {
        match s.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&buf[..n]);
                if raw.len() > MAX_ANSWER {
                    return Err(bad("the answer is too long"));
                }
                if answer_complete(&raw) {
                    break;
                }
            }
            // A peer closing without TLS's close_notify: what came is the answer.
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof && !raw.is_empty() => break,
            Err(e) => return Err(e),
        }
    }
    parse_answer(&raw).ok_or_else(|| bad("not an HTTP answer"))
}

/// The answer's head (status, `Content-Length`, chunked) and where its body starts.
fn answer_head(raw: &[u8]) -> Option<(u16, Option<usize>, bool, usize)> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..end]).ok()?;
    let mut lines = head.split("\r\n");
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let mut length = None;
    let mut chunked = false;
    for l in lines {
        let (k, v) = l.split_once(':')?;
        let v = v.trim();
        if k.eq_ignore_ascii_case("content-length") {
            length = v.parse().ok();
        } else if k.eq_ignore_ascii_case("transfer-encoding") {
            chunked = v.to_ascii_lowercase().contains("chunked");
        }
    }
    Some((status, length, chunked, end + 4))
}

/// The whole answer is in (by its length; chunked and unsized ones end with the connection).
fn answer_complete(raw: &[u8]) -> bool {
    matches!(answer_head(raw), Some((_, Some(n), false, at)) if raw.len() >= at + n)
}

fn parse_answer(raw: &[u8]) -> Option<(u16, Vec<u8>)> {
    let (status, length, chunked, at) = answer_head(raw)?;
    let rest = &raw[at..];
    let body = if chunked {
        let mut out = Vec::new();
        let mut r = rest;
        loop {
            let nl = r.windows(2).position(|w| w == b"\r\n")?;
            let size = std::str::from_utf8(&r[..nl]).ok()?;
            let size = usize::from_str_radix(size.split(';').next()?.trim(), 16).ok()?;
            r = &r[nl + 2..];
            if size == 0 {
                break;
            }
            out.extend_from_slice(r.get(..size)?);
            r = r.get(size + 2..)?;
        }
        out
    } else {
        match length {
            Some(n) => rest.get(..n)?.to_vec(),
            None => rest.to_vec(),
        }
    };
    Some((status, body))
}

fn box_state(v: &Value) -> BoxState {
    let mut screens: Vec<ScreenState> = v["screens"]
        .as_array()
        .map(|a| a.iter().filter_map(screen).collect())
        .unwrap_or_default();
    screens.sort_by_key(|s| s.number);
    BoxState {
        error: v["error"]
            .as_str()
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(str::to_string),
        screens,
    }
}

fn screen(v: &Value) -> Option<ScreenState> {
    let number = v["number"].as_u64()? as u32;
    let u = |k: &str| v[k].as_u64().map(|n| n as u32);
    let st = &v["status"];
    let error = v["error"].as_str().filter(|e| !e.is_empty());
    let status = if error.is_some() {
        MosaicScreenStatus::Error
    } else {
        match st["state"].as_str().unwrap_or("idle") {
            "playing" => MosaicScreenStatus::Showing,
            "connecting" => MosaicScreenStatus::Connecting,
            "lost" => MosaicScreenStatus::Lost,
            "fallback" => MosaicScreenStatus::Fallback,
            _ => MosaicScreenStatus::Idle,
        }
    };
    Some(ScreenState {
        number,
        name: v["name"].as_str().unwrap_or("").to_string(),
        width: u("width"),
        height: u("height"),
        feed: v["feed"].as_str().unwrap_or("").to_string(),
        status,
        problem: error
            .or_else(|| st["reason"].as_str().filter(|r| !r.is_empty()))
            .map(str::to_string),
        test_pattern: v["test_pattern"].as_bool().unwrap_or(false),
        settings: v.get("settings").filter(|s| s.is_object()).cloned(),
        sync: v["stats"]
            .get("sync")
            .filter(|s| s.is_object())
            .map(|y| SyncStats {
                group: y["group"].as_str().unwrap_or("").to_string(),
                mode: y["mode"].as_str().unwrap_or("").to_string(),
                delay_ms: y["delay_ms"].as_f64().unwrap_or(0.0) as f32,
                late: y["late"].as_u64().unwrap_or(0),
            }),
    })
}

/// Whether the settings a box has lost what Vesper set (`wanted`: a `settings` command): every
/// field `wanted` sets (fit, rotation, overscan, the sound's card, volume and mute, the
/// fallback's show, delay and logo, the wall tile). Fields the box doesn't report are taken as kept;
/// an absent or `null` wall is none, a `null` sound none; "" and "default" are the same card.
pub fn settings_lost(wanted: &Value, has: &Value) -> bool {
    let num = |a: &Value, b: &Value| match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => (x - y).abs() > 0.01,
        _ => a != b,
    };
    for k in ["fit", "rotation", "overscan"] {
        if let (Some(w), Some(h)) = (wanted.get(k), has.get(k))
            && num(w, h)
        {
            return true;
        }
    }
    if let (Some(wf), Some(hf)) = (wanted.get("fallback"), has.get("fallback"))
        && hf.is_object()
        && ["show", "after_s", "logo"].iter().any(|k| {
            wf.get(k)
                .is_some_and(|w| hf.get(k).is_some_and(|h| num(w, h)))
        })
    {
        return true;
    }
    if let (Some(wa), Some(ha)) = (wanted.get("audio"), has.get("audio")) {
        // A sound card's name, `None`: no sound.
        fn card(a: &Value) -> Option<&str> {
            match a["device"].as_str() {
                None => None,
                Some("") | Some("default") => Some("default"),
                Some(d) => Some(d),
            }
        }
        let reported = ha.is_null() || ha.get("device").is_some();
        if reported && card(wa) != card(ha) {
            return true;
        }
        if card(wa).is_some()
            && ["volume_db", "mute"]
                .iter()
                .any(|k| ha.get(k).is_some_and(|h| num(&wa[k], h)))
        {
            return true;
        }
    }
    let (Some(ww), Some(hw)) = (wanted.get("wall"), has.get("wall")) else {
        return false;
    };
    match (ww.is_object(), hw.is_object()) {
        (false, false) => false,
        (true, true) => {
            ["rows", "cols", "row", "col"]
                .iter()
                .any(|k| num(&ww[k], &hw[k]))
                || ["h", "v"]
                    .iter()
                    .any(|k| num(&ww["bezel_mm"][k], &hw["bezel_mm"][k]))
                || (ww["screen_mm"].is_object()
                    && ["w", "h"]
                        .iter()
                        .any(|k| num(&ww["screen_mm"][k], &hw["screen_mm"][k])))
                || (ww["group"].is_string() && ww["group"] != hw["group"])
        }
        _ => true,
    }
}

/// What a box has after it took a `settings` command (`body`): the fields sent replace its
/// own (`audio`, `fallback` and `wall` whole), the rest stay.
pub fn took_settings(has: &mut Option<Value>, body: &Value) {
    let h = has.get_or_insert_with(|| json!({}));
    if let (Some(h), Some(b)) = (h.as_object_mut(), body.as_object()) {
        for (k, v) in b {
            if k != "screen" {
                h.insert(k.clone(), v.clone());
            }
        }
    }
}

/// The service's `settings` command for a screen's settings (the whole of `audio`, `fallback`
/// and `wall` each time: the service replaces them whole).
pub fn settings_body(number: u32, s: &MosaicScreenSettings) -> Value {
    let fit = match s.fit {
        MosaicFit::Fit => "fit",
        MosaicFit::Fill => "fill",
        MosaicFit::Stretch => "stretch",
    };
    let show = match s.fallback {
        MosaicFallback::Logo => "logo",
        MosaicFallback::Last => "last",
        MosaicFallback::Black => "black",
    };
    let audio = match &s.sound {
        Some(a) => json!({"device": a.device, "volume_db": a.volume_db, "mute": a.mute}),
        None => json!({"device": null}),
    };
    let wall = s.tile.as_ref().map(wall_json);
    json!({
        "screen": number,
        "fit": fit,
        "rotation": s.rotation,
        "overscan": s.overscan,
        "audio": audio,
        "fallback": {"show": show, "after_s": s.fallback_after_seconds},
        "wall": wall,
    })
}

/// A screen's settings as the box has them (its `state`'s `settings`), in Vesper's terms: what a
/// screen Vesper never set reports, so that a change made in Vesper starts from them. What
/// Vesper doesn't have is left out (a slideshow fallback reads as the logo); a command sent
/// keeps it on the box (`keep_box_extras` in [`crate::mosaic`]).
pub fn read_settings(v: &Value) -> MosaicScreenSettings {
    let d = MosaicScreenSettings::default();
    let u16_of = |x: &Value| x.as_u64().and_then(|n| u16::try_from(n).ok());
    let f32_of = |x: &Value| x.as_f64().filter(|f| f.is_finite()).map(|f| f as f32);
    let fb = &v["fallback"];
    let audio = &v["audio"];
    let wall = &v["wall"];
    MosaicScreenSettings {
        fit: match v["fit"].as_str() {
            Some("fill") => MosaicFit::Fill,
            Some("stretch") => MosaicFit::Stretch,
            _ => MosaicFit::Fit,
        },
        rotation: u16_of(&v["rotation"])
            .filter(|r| [0, 90, 180, 270].contains(r))
            .unwrap_or(d.rotation),
        overscan: f32_of(&v["overscan"])
            .map(|o| o.clamp(0.0, 20.0))
            .unwrap_or(d.overscan),
        sound: audio["device"].as_str().map(|dev| MosaicSound {
            device: if dev.is_empty() { "default" } else { dev }.to_string(),
            volume_db: f32_of(&audio["volume_db"])
                .map(|x| x.clamp(-60.0, 12.0))
                .unwrap_or(0.0),
            mute: audio["mute"].as_bool().unwrap_or(false),
        }),
        fallback: match fb["show"].as_str() {
            Some("last") => MosaicFallback::Last,
            Some("black") => MosaicFallback::Black,
            _ => MosaicFallback::Logo,
        },
        fallback_after_seconds: fb["after_s"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .map(|n| n.min(3600))
            .unwrap_or(d.fallback_after_seconds),
        tile: wall.is_object().then(|| MosaicTile {
            rows: u16_of(&wall["rows"]).unwrap_or(1),
            columns: u16_of(&wall["cols"]).unwrap_or(1),
            row: u16_of(&wall["row"]).unwrap_or(0),
            column: u16_of(&wall["col"]).unwrap_or(0),
            bezel_mm: [
                f32_of(&wall["bezel_mm"]["h"]).unwrap_or(0.0),
                f32_of(&wall["bezel_mm"]["v"]).unwrap_or(0.0),
            ],
            screen_mm: wall["screen_mm"].is_object().then(|| {
                [
                    f32_of(&wall["screen_mm"]["w"]).unwrap_or(0.0),
                    f32_of(&wall["screen_mm"]["h"]).unwrap_or(0.0),
                ]
            }),
            group: wall["group"].as_str().map(str::to_string),
        }),
    }
}

/// A tile as the service's `settings.wall`.
pub fn wall_json(t: &MosaicTile) -> Value {
    let mut w = json!({
        "rows": t.rows, "cols": t.columns, "row": t.row, "col": t.column,
        "bezel_mm": {"h": t.bezel_mm[0], "v": t.bezel_mm[1]},
    });
    if let Some([sw, sh]) = t.screen_mm {
        w["screen_mm"] = json!({"w": sw, "h": sh});
    }
    if let Some(g) = &t.group {
        w["group"] = json!(g);
    }
    w
}

/// Checks settings against the service's ranges.
pub fn check_settings(s: &MosaicScreenSettings) -> Result<(), String> {
    if ![0, 90, 180, 270].contains(&s.rotation) {
        return Err("rotation: 0, 90, 180 or 270".into());
    }
    if !(0.0..=20.0).contains(&s.overscan) {
        return Err("overscan: 0 to 20 percent".into());
    }
    if s.fallback_after_seconds > 3600 {
        return Err("fallbackAfterSeconds: 0 to 3600".into());
    }
    if let Some(a) = &s.sound
        && !(-60.0..=12.0).contains(&a.volume_db)
    {
        return Err("sound.volumeDb: −60 to 12".into());
    }
    if let Some(t) = &s.tile {
        let ok = (1..=16).contains(&t.rows)
            && (1..=16).contains(&t.columns)
            && t.row < t.rows
            && t.column < t.columns
            && t.bezel_mm.iter().all(|b| (0.0..=10000.0).contains(b))
            && (t.bezel_mm == [0.0, 0.0] || t.screen_mm.is_some_and(|[w, h]| w > 0.0 && h > 0.0))
            && t.group.as_ref().is_none_or(|g| g.len() <= 64);
        if !ok {
            return Err("tile: 1–16 rows and columns, its row and column inside, bezels 0–10000 mm (with the screen's size), group ≤ 64 characters".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_urls() {
        assert_eq!(base_url("192.0.2.40"), "https://192.0.2.40:29223");
        assert_eq!(base_url("lobby.local:4000"), "https://lobby.local:4000");
        assert_eq!(base_url("2001:db8::7"), "https://[2001:db8::7]:29223");
        assert_eq!(base_url("[2001:db8::7]:5"), "https://[2001:db8::7]:5");
        assert_eq!(base_url("http://127.0.0.1:9/"), "http://127.0.0.1:9");
    }

    #[test]
    fn reads_a_screen_of_the_state() {
        let s = screen(&json!({
            "id": "DP-2", "name": "DP-2", "number": 2, "width": 1920, "height": 1080,
            "feed": "BOOTH (Program)", "error": "",
            "status": {"state": "playing", "showing": "feed"}, "test_pattern": false,
        }))
        .unwrap();
        assert_eq!(s.number, 2);
        assert_eq!(s.status, MosaicScreenStatus::Showing);
        assert_eq!(s.feed, "BOOTH (Program)");
        assert_eq!(s.problem, None);
        let lost = screen(&json!({"number": 1, "name": "HDMI-1", "status": {"state": "lost", "reason": "no pictures"}})).unwrap();
        assert_eq!(lost.status, MosaicScreenStatus::Lost);
        assert_eq!(lost.problem.as_deref(), Some("no pictures"));
    }

    #[test]
    fn settings_go_as_the_service_takes_them() {
        let mut s = MosaicScreenSettings::default();
        let b = settings_body(1, &s);
        assert_eq!(b["fallback"], json!({"show": "logo", "after_s": 5}));
        assert_eq!(b["audio"], json!({"device": null}));
        assert_eq!(b["wall"], Value::Null);
        s.rotation = 45;
        assert!(check_settings(&s).is_err());
        s.rotation = 90;
        s.overscan = 25.0;
        assert!(check_settings(&s).is_err());
        s.overscan = 2.5;
        s.tile = Some(protocol::MosaicTile {
            rows: 1,
            columns: 2,
            row: 0,
            column: 1,
            bezel_mm: [12.0, 0.0],
            screen_mm: None,
            group: None,
        });
        assert!(
            check_settings(&s).is_err(),
            "a bezel needs the screen's size"
        );
        s.tile.as_mut().unwrap().screen_mm = Some([1210.0, 680.0]);
        assert!(check_settings(&s).is_ok());
        let b = settings_body(1, &s);
        assert_eq!(b["wall"]["cols"], 2);
        assert_eq!(b["wall"]["bezel_mm"], json!({"h": 12.0, "v": 0.0}));
    }

    #[test]
    fn lost_settings_are_noticed() {
        let mut s = MosaicScreenSettings {
            tile: Some(protocol::MosaicTile {
                rows: 2,
                columns: 2,
                row: 1,
                column: 0,
                bezel_mm: [8.0, 8.0],
                screen_mm: Some([1210.0, 680.0]),
                group: Some("wall:Front".into()),
            }),
            ..Default::default()
        };
        let want = settings_body(3, &s);
        // As the box has it again (`screen` isn't part of its settings; extra fields are its own).
        let mut has = want.clone();
        has.as_object_mut().unwrap().remove("screen");
        has["idle"] = json!("desktop");
        has["wall"]["bezel_mm"] = json!({"h": 8, "v": 8.0});
        assert!(!settings_lost(&want, &has));
        assert!(!settings_lost(&want, &json!({})), "nothing said: kept");
        assert!(
            settings_lost(&want, &json!({"wall": null})),
            "the wall went"
        );
        has["wall"]["row"] = json!(0);
        assert!(settings_lost(&want, &has), "another tile");
        has["wall"]["row"] = json!(1);
        has["wall"]["group"] = json!("wall:BOOTH (Program)");
        assert!(settings_lost(&want, &has), "another group");
        s.tile = None;
        let want = settings_body(3, &s);
        assert!(settings_lost(&want, &has), "a wall it shouldn't have");
        assert!(!settings_lost(
            &want,
            &json!({"fit": "fit", "rotation": 0, "wall": null})
        ));
        assert!(settings_lost(&want, &json!({"fit": "fill"})));
        // Only the tile sent (the box's own settings left alone).
        let only = json!({"screen": 3, "wall": null});
        assert!(!settings_lost(
            &only,
            &json!({"fit": "stretch", "rotation": 90})
        ));
        assert!(settings_lost(
            &only,
            &json!({"fit": "stretch", "wall": {"rows": 1}})
        ));
    }

    /// H01-7: the service's own error (no desktop session) is read with the screens.
    #[test]
    fn reads_the_services_own_error() {
        let st = box_state(
            &json!({"service": "display", "error": "no desktop on this computer", "screens": []}),
        );
        assert_eq!(st.error.as_deref(), Some("no desktop on this computer"));
        assert!(st.screens.is_empty());
        let st = box_state(&json!({"error": "", "screens": [{"number": 2}, {"number": 1}]}));
        assert_eq!(st.error, None);
        assert_eq!(
            st.screens.iter().map(|s| s.number).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    /// H01-5: overscan, the sound and the fallback lost are noticed as well.
    #[test]
    fn every_setting_sent_is_compared() {
        let s = MosaicScreenSettings {
            overscan: 2.0,
            sound: Some(MosaicSound {
                device: "HDMI 1".into(),
                volume_db: -6.0,
                mute: false,
            }),
            fallback: MosaicFallback::Black,
            ..Default::default()
        };
        let want = settings_body(1, &s);
        // As the box has it (with fields of its own).
        let has = json!({"fit": "fit", "rotation": 0, "overscan": 2, "idle": "desktop",
            "audio": {"device": "HDMI 1", "channels": [0, 1], "volume_db": -6, "mute": false},
            "fallback": {"show": "black", "after_s": 5, "logo": ""}, "wall": null});
        assert!(!settings_lost(&want, &has));
        for (k, v) in [
            ("overscan", json!(0)),
            (
                "audio",
                json!({"device": "HDMI 1", "volume_db": -6, "mute": true}),
            ),
            (
                "audio",
                json!({"device": "default", "volume_db": -6, "mute": false}),
            ),
            ("audio", json!(null)),
            ("fallback", json!({"show": "logo", "after_s": 5})),
            ("fallback", json!({"show": "black", "after_s": 30})),
        ] {
            let mut h = has.clone();
            h[k] = v.clone();
            assert!(settings_lost(&want, &h), "{k}: {v}");
        }
        // No sound: a box with none, or the same card spelt "" / "default".
        let none = settings_body(1, &MosaicScreenSettings::default());
        assert!(!settings_lost(&none, &json!({"audio": null})));
        assert!(!settings_lost(
            &none,
            &json!({"audio": {"device": null, "channels": [0, 1]}})
        ));
        assert!(settings_lost(&none, &json!({"audio": {"device": ""}})));
        let mut sys = s.clone();
        sys.sound.as_mut().unwrap().device = "default".into();
        let sys = settings_body(1, &sys);
        assert!(!settings_lost(
            &sys,
            &json!({"audio": {"device": "", "volume_db": -6, "mute": false}})
        ));
        // Nothing said about them: kept.
        assert!(!settings_lost(&want, &json!({"fit": "fit"})));
    }

    /// H01-6: a box's own settings read in Vesper's terms, and sent back as they were.
    #[test]
    fn a_boxs_own_settings_are_read() {
        let has = json!({"fit": "stretch", "rotation": 270, "overscan": 3.5,
            "audio": {"device": "Speakers", "channels": [0, 1], "volume_db": -12, "mute": true},
            "fallback": {"show": "last", "after_s": 20}, "idle": "fallback",
            "wall": {"rows": 1, "cols": 2, "row": 0, "col": 1, "bezel_mm": {"h": 0, "v": 0}, "group": "wall:Side"}});
        let s = read_settings(&has);
        assert_eq!(s.fit, MosaicFit::Stretch);
        assert_eq!(s.rotation, 270);
        assert_eq!(s.overscan, 3.5);
        assert_eq!(
            s.sound,
            Some(MosaicSound {
                device: "Speakers".into(),
                volume_db: -12.0,
                mute: true
            })
        );
        assert_eq!(s.fallback, MosaicFallback::Last);
        assert_eq!(s.fallback_after_seconds, 20);
        assert_eq!(s.tile.as_ref().unwrap().column, 1);
        assert_eq!(s.tile.as_ref().unwrap().group.as_deref(), Some("wall:Side"));
        assert!(check_settings(&s).is_ok());
        assert!(!settings_lost(&settings_body(4, &s), &has));
        // What it has after taking a command: the fields sent, its own kept.
        let mut after = Some(has.clone());
        took_settings(&mut after, &json!({"screen": 4, "wall": null}));
        let after = after.unwrap();
        assert_eq!(after["wall"], Value::Null);
        assert_eq!(after["rotation"], 270);
        assert!(after.get("screen").is_none());
        // Nothing (or nothing known): the defaults.
        assert_eq!(read_settings(&json!({})), MosaicScreenSettings::default());
        assert_eq!(
            read_settings(&json!({"rotation": 45, "audio": {"device": null}})),
            MosaicScreenSettings::default()
        );
    }

    /// A one-connection-at-a-time HTTPS server presenting `cert` (with the CA) for `key`,
    /// answering every request with an empty state. Its address.
    fn tls_box(cert: Vec<u8>, ca: Vec<u8>, key: &[u8]) -> std::net::SocketAddr {
        use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let cfg = Arc::new(
            rustls::ServerConfig::builder_with_provider(provider)
                .with_protocol_versions(&[&rustls::version::TLS13])
                .unwrap()
                .with_no_client_auth()
                .with_single_cert(
                    vec![CertificateDer::from(cert), CertificateDer::from(ca)],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.to_vec())),
                )
                .unwrap(),
        );
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        std::thread::spawn(move || {
            for tcp in l.incoming().flatten() {
                let Ok(conn) = rustls::ServerConnection::new(cfg.clone()) else {
                    continue;
                };
                let _ = tcp.set_read_timeout(Some(Duration::from_secs(5)));
                let mut s = rustls::StreamOwned::new(conn, tcp);
                let mut got = Vec::new();
                let mut buf = [0u8; 4096];
                while !got.windows(4).any(|w| w == b"\r\n\r\n") {
                    match s.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => got.extend_from_slice(&buf[..n]),
                    }
                }
                if got.is_empty() {
                    continue; // the client refused the certificate
                }
                let body =
                    br#"{"service":"display","error":"","screens":[{"number":1,"name":"HDMI-1"}]}"#;
                let _ = s.write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
                );
                let _ = s.write_all(body);
                let _ = s.flush();
                s.conn.send_close_notify();
                let _ = s.flush();
            }
        });
        addr
    }

    /// Batch 15 M3-1: a box is reached only with its own LAN certificate of the site: another
    /// machine of the site answering with its valid certificate is refused, and so is a box
    /// whose certificate is another site's; plain HTTP only to this machine.
    #[test]
    fn only_the_boxs_own_certificate_is_accepted() {
        use idn_open_protocol::trust::pki::{Anchor, Ca, KeyFile};
        let ca = Ca::from_key(KeyFile::generate().unwrap()).unwrap();
        let anchor = Arc::new(Anchor::new(ca.cert_der()).unwrap());
        let lobby = KeyFile::generate().unwrap();
        let side = KeyFile::generate().unwrap();
        let localhost = [std::net::IpAddr::from([127, 0, 0, 1])];
        // SIDE's own certificate, issued by the site's CA, at 127.0.0.1.
        let cert = ca
            .issue_lan(&side.spki(), "SIDE", &["side".into()], &localhost)
            .unwrap();
        let at = tls_box(cert.clone(), ca.cert_der(), side.pkcs8());
        let base = format!("https://{at}");
        let tls_for = |fp: &str| Arc::new(anchor.lan_client_config(fp).unwrap());
        let ask = |tls: &Arc<rustls::ClientConfig>| {
            Client {
                base: &base,
                token: "t",
                tls: Some(tls),
            }
            .state()
        };
        let st = ask(&tls_for(&side.fingerprint())).expect("SIDE's own certificate passes");
        assert_eq!(st.screens[0].name, "HDMI-1");
        // Vesper wants LOBBY there: SIDE's certificate, valid as it is, is refused.
        let e = ask(&tls_for(&lobby.fingerprint())).unwrap_err();
        assert!(e.message.contains("certificate"), "{}", e.message);
        // Another site's CA: refused, even for the right machine.
        let other = Ca::from_key(KeyFile::generate().unwrap()).unwrap();
        let other = Arc::new(Anchor::new(other.cert_der()).unwrap());
        let e = ask(&Arc::new(
            other.lan_client_config(&side.fingerprint()).unwrap(),
        ));
        assert!(e.is_err());
        // No authority to check with: not asked at all.
        let none = Client {
            base: &base,
            token: "t",
            tls: None,
        };
        assert!(
            none.state()
                .unwrap_err()
                .message
                .contains("can't be checked")
        );
        // Plain HTTP goes only to this machine.
        let plain = Client {
            base: "http://192.0.2.40:29222",
            token: "t",
            tls: None,
        };
        assert!(plain.state().unwrap_err().message.contains("only https://"));
        assert!(loopback_http("http://127.0.0.1:9/api/v1/state"));
        assert!(loopback_http("http://[::1]:9/x"));
        assert!(loopback_http("http://localhost:9"));
        assert!(!loopback_http("http://192.0.2.40:9"));
        assert!(!loopback_http("https://127.0.0.1:9"));
    }

    #[test]
    fn reads_http_answers() {
        assert_eq!(
            parse_answer(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi"),
            Some((200, b"hi".to_vec()))
        );
        assert!(answer_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi"
        ));
        assert!(!answer_complete(
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nhi"
        ));
        assert_eq!(
            parse_answer(b"HTTP/1.1 404 Not Found\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"),
            Some((404, b"abcde".to_vec()))
        );
        assert_eq!(host_port("[2001:db8::7]:5"), ("2001:db8::7", Some(5)));
        assert_eq!(host_port("lobby.local:29223"), ("lobby.local", Some(29223)));
        assert_eq!(host_port("2001:db8::7"), ("2001:db8::7", None));
    }

    #[test]
    fn reads_a_tiles_sync() {
        let s = screen(&json!({
            "number": 1, "name": "HDMI-1", "feed": "BOOTH (Program)",
            "status": {"state": "playing"},
            "settings": {"fit": "fit", "wall": {"rows": 1, "cols": 2, "row": 0, "col": 0}},
            "stats": {"fps": 30, "sync": {"group": "wall:Front", "mode": "sync/1", "delay_ms": 41.5, "late": 3}},
        }))
        .unwrap();
        let y = s.sync.unwrap();
        assert_eq!(y.group, "wall:Front");
        assert_eq!(y.delay_ms, 41.5);
        assert_eq!(y.late, 3);
        assert_eq!(s.settings.unwrap()["wall"]["cols"], 2);
    }
}
