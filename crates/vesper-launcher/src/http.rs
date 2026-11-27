//! A minimal blocking HTTP/1.0 client for the LAN: health checks and the site's update channel
//! (plain `http://`, like the rest of the site's API). HTTP/1.0 keeps the answers simple: no
//! chunked bodies, the connection closes after each one.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

/// An answer.
pub struct Response {
    /// Its status code.
    pub status: u16,
    /// Its body.
    pub body: Vec<u8>,
}

struct Target {
    host: String,
    port: u16,
    path: String,
}

fn parse_url(url: &str) -> anyhow::Result<Target> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| anyhow::anyhow!("only http:// addresses are supported: {url}"))?;
    let (hostport, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !h.ends_with(']') || h.starts_with('[') => (
            h.to_string(),
            p.parse()
                .map_err(|_| anyhow::anyhow!("bad port in {url}"))?,
        ),
        _ => (hostport.to_string(), 80),
    };
    Ok(Target {
        host,
        port,
        path: path.to_string(),
    })
}

fn connect(t: &Target, timeout: Duration) -> anyhow::Result<TcpStream> {
    let host = t.host.trim_start_matches('[').trim_end_matches(']');
    let addr = (host, t.port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| anyhow::anyhow!("{} has no address", t.host))?;
    let s = TcpStream::connect_timeout(&addr, timeout)?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(timeout))?;
    Ok(s)
}

fn send(
    s: &mut TcpStream,
    t: &Target,
    method: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> anyhow::Result<()> {
    let mut head = format!(
        "{method} {} HTTP/1.0\r\nHost: {}:{}\r\nUser-Agent: vesper-launcher\r\n",
        t.path, t.host, t.port
    );
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if let Some(b) = body {
        head.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())?;
    if let Some(b) = body {
        s.write_all(b)?;
    }
    s.flush()?;
    Ok(())
}

/// Reads the status line and headers; the reader is left at the body.
fn read_head(r: &mut BufReader<TcpStream>) -> anyhow::Result<u16> {
    let mut line = String::new();
    r.read_line(&mut line)?;
    let status = line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| anyhow::anyhow!("not an HTTP answer"))?;
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 || line == "\r\n" || line == "\n" {
            return Ok(status);
        }
    }
}

/// Sends a request and reads the whole answer.
pub fn request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
    timeout: Duration,
) -> anyhow::Result<Response> {
    let t = parse_url(url)?;
    let mut s = connect(&t, timeout)?;
    send(&mut s, &t, method, headers, body)?;
    let mut r = BufReader::new(s);
    let status = read_head(&mut r)?;
    let mut body = Vec::new();
    r.read_to_end(&mut body)?;
    Ok(Response { status, body })
}

/// Downloads `url` into `dest` (a `200` answer only); its size.
pub fn download(
    url: &str,
    headers: &[(&str, &str)],
    dest: &Path,
    timeout: Duration,
) -> anyhow::Result<u64> {
    let t = parse_url(url)?;
    let mut s = connect(&t, timeout)?;
    send(&mut s, &t, "GET", headers, None)?;
    let mut r = BufReader::new(s);
    let status = read_head(&mut r)?;
    if status != 200 {
        let mut b = Vec::new();
        let _ = r.read_to_end(&mut b);
        anyhow::bail!(
            "{url} answered {status}: {}",
            String::from_utf8_lossy(&b).trim()
        );
    }
    let mut f = std::fs::File::create(dest)?;
    let n = std::io::copy(&mut r, &mut f)?;
    f.sync_all()?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_parsed() {
        let t = parse_url("http://192.0.2.10:5555/api/v1/server").unwrap();
        assert_eq!(
            (t.host.as_str(), t.port, t.path.as_str()),
            ("192.0.2.10", 5555, "/api/v1/server")
        );
        let t = parse_url("http://[::1]:80").unwrap();
        assert_eq!(
            (t.host.as_str(), t.port, t.path.as_str()),
            ("[::1]", 80, "/")
        );
        let t = parse_url("http://box.local/x").unwrap();
        assert_eq!((t.host.as_str(), t.port), ("box.local", 80));
        assert!(parse_url("https://x").is_err());
    }
}
