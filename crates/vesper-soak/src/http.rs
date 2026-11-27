//! A tiny blocking HTTP/1.1 client for the local server (one request per connection, JSON
//! bodies). The server is on this machine, so plain TCP to its address is enough.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// An answer: status and body.
pub struct Resp {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Resp {
    /// The body as JSON (`null` when empty or not JSON).
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// 2xx.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Sends `method path` with an optional JSON body.
pub fn request(addr: SocketAddr, method: &str, path: &str, body: Option<&Value>) -> Result<Resp> {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5))
        .with_context(|| format!("connect {addr}"))?;
    s.set_read_timeout(Some(Duration::from_secs(60)))?;
    s.set_write_timeout(Some(Duration::from_secs(10)))?;
    let payload = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n",
        payload.len()
    );
    if body.is_some() {
        req.push_str("Content-Type: application/json\r\n");
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes())?;
    s.write_all(payload.as_bytes())?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("no HTTP header")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .context("bad status line")?;
    let mut body = raw[split + 4..].to_vec();
    if head.lines().any(|l| {
        l.to_ascii_lowercase()
            .starts_with("transfer-encoding: chunked")
    }) {
        body = dechunk(&body)?;
    }
    Ok(Resp { status, body })
}

fn dechunk(mut b: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = b
            .windows(2)
            .position(|w| w == b"\r\n")
            .context("bad chunk")?;
        let n = usize::from_str_radix(String::from_utf8_lossy(&b[..eol]).trim(), 16)?;
        b = &b[eol + 2..];
        if n == 0 {
            return Ok(out);
        }
        if b.len() < n {
            bail!("short chunk");
        }
        out.extend_from_slice(&b[..n]);
        b = &b[(n + 2).min(b.len())..];
    }
}

/// `GET path` as JSON; an error for a non-2xx answer.
pub fn get(addr: SocketAddr, path: &str) -> Result<Value> {
    let r = request(addr, "GET", path, None)?;
    if !r.ok() {
        bail!(
            "GET {path}: {} {}",
            r.status,
            String::from_utf8_lossy(&r.body)
        );
    }
    Ok(r.json())
}

/// `method path body` as JSON; an error for a non-2xx answer.
pub fn send(addr: SocketAddr, method: &str, path: &str, body: &Value) -> Result<Value> {
    let r = request(addr, method, path, Some(body))?;
    if !r.ok() {
        bail!(
            "{method} {path}: {} {}",
            r.status,
            String::from_utf8_lossy(&r.body)
        );
    }
    Ok(r.json())
}
