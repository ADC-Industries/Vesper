//! A fake GitHub for tests (feature `fake`): a local HTTP server answering the releases API
//! (`/repos/{owner}/{name}/releases`, with ETags and a switchable rate limit) and serving the
//! release files, whose packages are signed by a throwaway key ([`crate::sign::TestSigner`]) that
//! the test passes as an extra trusted key. Synthetic data only.

use crate::package::PackageMeta;
use crate::sign::{Hasher, TestSigner};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

/// A request the fake answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The path (with the query).
    pub path: String,
    /// Its `If-None-Match`.
    pub if_none_match: Option<String>,
    /// The status answered.
    pub status: u16,
}

#[derive(Default)]
struct State {
    releases: Vec<serde_json::Value>,
    files: HashMap<String, Vec<u8>>,
    generation: u64,
    rate_limited: Option<u64>,
    requests: Vec<Request>,
}

/// The fake GitHub.
pub struct FakeGitHub {
    addr: std::net::SocketAddr,
    state: Arc<Mutex<State>>,
    /// The key its packages are signed with.
    pub signer: TestSigner,
}

/// A file of a fake release: its name and bytes.
pub type File = (String, Vec<u8>);

fn lock(m: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Default for FakeGitHub {
    fn default() -> Self {
        Self::start()
    }
}

impl FakeGitHub {
    /// Starts it on a free loopback port.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let state = Arc::new(Mutex::new(State::default()));
        let s2 = state.clone();
        std::thread::spawn(move || {
            for c in listener.incoming().flatten() {
                let s3 = s2.clone();
                std::thread::spawn(move || {
                    let _ = serve(c, &s3);
                });
            }
        });
        FakeGitHub {
            addr,
            state,
            signer: TestSigner::generate(),
        }
    }

    /// Its API's URL (`http://127.0.0.1:<port>`).
    pub fn api(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The signing key's public half (minisign text).
    pub fn public_key(&self) -> String {
        self.signer.public().to_text()
    }

    /// A signature of `bytes` by the fake's key with the trusted comment for `meta`.
    pub fn sign(&self, bytes: &[u8], product: &str, version: &str, platform: &str) -> String {
        let meta = PackageMeta {
            product: product.into(),
            version: version.into(),
            platform: platform.into(),
        };
        self.signer.sign(&Hasher::bytes(bytes), &meta.comment())
    }

    /// Adds a release as `scripts/release.sh` makes it: per package (product, platform, file
    /// name, bytes) the file, its signature and a manifest row.
    pub fn release(
        &self,
        tag: &str,
        version: &str,
        prerelease: bool,
        packages: &[(&str, &str, &str, &[u8])],
    ) {
        let mut files: Vec<File> = Vec::new();
        let mut rows = Vec::new();
        for (product, platform, name, bytes) in packages {
            files.push((name.to_string(), bytes.to_vec()));
            files.push((
                format!("{name}.minisig"),
                self.sign(bytes, product, version, platform).into_bytes(),
            ));
            rows.push(serde_json::json!({
                "product": product, "platform": platform, "file": name,
                "size": bytes.len(), "sha256": Hasher::bytes(bytes).sha256,
            }));
        }
        let manifest = serde_json::json!({
            "format": 1, "version": version, "tag": tag,
            "channel": if prerelease { "beta" } else { "stable" }, "assets": rows,
        });
        files.push((
            "manifest.json".into(),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        ));
        self.raw_release(tag, prerelease, false, files);
    }

    /// Adds a release with exactly these files.
    pub fn raw_release(&self, tag: &str, prerelease: bool, draft: bool, files: Vec<File>) {
        let base = self.api();
        let mut s = lock(&self.state);
        let assets: Vec<serde_json::Value> = files
            .iter()
            .map(|(name, bytes)| {
                serde_json::json!({
                    "name": name, "size": bytes.len(),
                    "browser_download_url": format!("{base}/download/{tag}/{name}"),
                })
            })
            .collect();
        for (name, bytes) in files {
            s.files.insert(format!("/download/{tag}/{name}"), bytes);
        }
        s.releases.insert(
            0,
            serde_json::json!({
                "tag_name": tag, "prerelease": prerelease, "draft": draft, "assets": assets,
                "published_at": "2026-11-27T00:00:00Z",
            }),
        );
        s.generation += 1;
    }

    /// Replaces a release file's bytes (a tampered download).
    pub fn replace_file(&self, tag: &str, name: &str, bytes: Vec<u8>) {
        lock(&self.state)
            .files
            .insert(format!("/download/{tag}/{name}"), bytes);
    }

    /// Answers every request with GitHub's rate-limit error until `reset` (seconds since the
    /// epoch); `None` ends it.
    pub fn rate_limit(&self, reset: Option<u64>) {
        lock(&self.state).rate_limited = reset;
    }

    /// The requests answered so far.
    pub fn requests(&self) -> Vec<Request> {
        lock(&self.state).requests.clone()
    }

    /// The release-list requests answered so far.
    pub fn listings(&self) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|r| r.path.contains("/releases"))
            .collect()
    }
}

fn serve(c: TcpStream, state: &Mutex<State>) -> std::io::Result<()> {
    let mut r = BufReader::new(c.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
    let mut inm = None;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h.trim().is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':')
            && k.trim().eq_ignore_ascii_case("if-none-match")
        {
            inm = Some(v.trim().to_string());
        }
    }
    let mut s = lock(state);
    let (status, headers, body): (u16, Vec<(String, String)>, Vec<u8>) =
        if let Some(reset) = s.rate_limited {
            (
                403,
                vec![
                    ("x-ratelimit-remaining".into(), "0".into()),
                    ("x-ratelimit-reset".into(), reset.to_string()),
                ],
                br#"{"message":"API rate limit exceeded for 192.0.2.1."}"#.to_vec(),
            )
        } else if path.starts_with("/repos/") && path.contains("/releases") {
            let etag = format!("\"gen-{}\"", s.generation);
            if inm.as_deref() == Some(etag.as_str()) {
                (304, vec![("etag".into(), etag)], Vec::new())
            } else {
                (
                    200,
                    vec![
                        ("etag".into(), etag),
                        ("content-type".into(), "application/json".into()),
                    ],
                    serde_json::to_vec(&s.releases).unwrap(),
                )
            }
        } else if let Some(b) = s.files.get(&path) {
            (200, Vec::new(), b.clone())
        } else {
            (404, Vec::new(), br#"{"message":"Not Found"}"#.to_vec())
        };
    s.requests.push(Request {
        path,
        if_none_match: inm,
        status,
    });
    drop(s);
    let mut w = c;
    let reason = match status {
        200 => "OK",
        304 => "Not Modified",
        403 => "Forbidden",
        _ => "Not Found",
    };
    let mut head = format!("HTTP/1.1 {status} {reason}\r\nconnection: close\r\n");
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    if status != 304 {
        head.push_str(&format!("content-length: {}\r\n", body.len()));
    }
    head.push_str("\r\n");
    w.write_all(head.as_bytes())?;
    w.write_all(&body)?;
    w.flush()
}
