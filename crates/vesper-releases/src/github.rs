//! Vesper's releases on GitHub (`docs/PACKAGING.md`, "Releases").
//!
//! One repository, one release line per product told apart by the tag's prefix
//! (`server-v0.2.0`, `oriel-v0.2.0`, `launcher-v0.2.0`, `idn-node-v0.3.8-beta18`); a GitHub
//! pre-release is a beta. Each release has its packages, a minisign signature beside each
//! (`<file>.minisig`) and `manifest.json` (per asset its signed product and platform), made by
//! `scripts/release.sh`.
//!
//! [`GitHub`] lists the releases with GitHub's unauthenticated API (one request per check, with
//! `If-None-Match`, so an unchanged list costs nothing against the rate limit), remembers a rate
//! limit until it ends instead of asking again, and treats a network failure as "offline".
//! [`GitHub::latest`] picks the newest release of a product with a package for a platform in a
//! channel; [`GitHub::download`] fetches the package and its signature into a staging folder
//! (with size limits) and verifies them against the trusted keys: the signature must hold and
//! its trusted comment must name the product, the release's version and the platform, or the
//! package is refused and removed. The manifest only says where to look: nothing in it is
//! trusted.
//!
//! Blocking (ureq): call it from a blocking thread.

use crate::package::{self, Verified, compare_versions};
use crate::sign::{Hasher, PublicKey};
use protocol::{CatalogueProduct, ReleaseChannel, UpdatePackage};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The repository releases come from unless configured otherwise.
pub const DEFAULT_REPOSITORY: &str = "ADC-Industries/Vesper";
/// GitHub's API.
pub const DEFAULT_API: &str = "https://api.github.com";
/// The largest package downloaded (the server's Windows installer is a few hundred MB).
pub const MAX_PACKAGE: u64 = 2 << 30;
/// The largest signature file.
pub const MAX_SIGNATURE: u64 = 4096;
/// The largest manifest.
const MAX_MANIFEST: u64 = 256 << 10;
/// The largest release list.
const MAX_LISTING: u64 = 16 << 20;
/// How long a rate limit without a reset time is waited out.
const RATE_LIMIT_WAIT: Duration = Duration::from_secs(3600);
/// The manifest's file name in a release.
pub const MANIFEST: &str = "manifest.json";

/// A release line: a tag prefix and the products its releases carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    /// The tag's prefix (`server` for `server-v0.2.0`).
    pub prefix: &'static str,
    /// The products its packages are signed as.
    pub products: &'static [&'static str],
}

/// Vesper's release lines.
pub const LINES: &[Line] = &[
    Line {
        prefix: "server",
        products: &["vesper"],
    },
    Line {
        prefix: "launcher",
        products: &["vesper-launcher", "vesper-launcher-tray"],
    },
    Line {
        prefix: "idn-node",
        products: &["vesper-idn-node"],
    },
    Line {
        prefix: "oriel",
        products: &["vesper-oriel"],
    },
];

/// The release line carrying `product`.
pub fn line_of(product: &str) -> Option<&'static Line> {
    LINES.iter().find(|l| l.products.contains(&product))
}

/// A product's name for people.
pub fn product_name(product: &str) -> String {
    match product {
        "vesper" => "Vesper".into(),
        "vesper-launcher" => "Vesper Launcher".into(),
        "vesper-launcher-tray" => "Vesper Launcher tray".into(),
        "vesper-idn-node" => "IDN Node (bundled)".into(),
        "vesper-oriel" => "Vesper Oriel".into(),
        p => p.to_string(),
    }
}

/// A tag's line and version (`server-v0.2.0` → `server`, `0.2.0`); `None` for other tags.
pub fn parse_tag(tag: &str) -> Option<(&'static Line, String)> {
    LINES.iter().find_map(|l| {
        let v = tag.strip_prefix(l.prefix)?.strip_prefix("-v")?;
        (v.starts_with(|c: char| c.is_ascii_digit()) && package::valid_word(v))
            .then(|| (l, v.to_string()))
    })
}

/// Whether a signed version is a release's (`0.3.8~beta18` is `idn-node-v0.3.8-beta18`'s: a tag
/// can't hold `~`).
pub fn same_version(signed: &str, release: &str) -> bool {
    let n = |v: &str| v.to_ascii_lowercase().replace('~', "-");
    n(signed) == n(release)
}

/// A release's asset.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Asset {
    /// Its file name.
    pub name: String,
    /// Its size as GitHub lists it.
    pub size: u64,
    /// Where to download it.
    #[serde(rename = "browser_download_url")]
    pub url: String,
}

/// A release of one of Vesper's lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Its tag.
    pub tag: String,
    /// Its line.
    pub line: &'static Line,
    /// Its version (from the tag).
    pub version: String,
    /// A pre-release (the beta channel).
    pub prerelease: bool,
    /// Its files.
    pub assets: Vec<Asset>,
}

impl Release {
    /// Whether `channel` takes it.
    pub fn in_channel(&self, channel: ReleaseChannel) -> bool {
        !self.prerelease || channel == ReleaseChannel::Beta
    }

    /// The asset named `name`.
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

#[derive(Deserialize)]
struct RawRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

/// A release's `manifest.json` (`scripts/release.sh`). Untrusted: it only says which file is for
/// which product and platform; the signatures decide.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    /// Its assets.
    pub assets: Vec<ManifestAsset>,
}

/// An asset in a manifest.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ManifestAsset {
    /// The product it is signed as.
    pub product: String,
    /// Its platform.
    pub platform: String,
    /// Its file name.
    pub file: String,
    /// Its SHA-256 (hex).
    #[serde(default)]
    pub sha256: Option<String>,
}

/// Why a check or a download didn't work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckError {
    /// GitHub couldn't be reached (no internet, DNS, a timeout).
    Offline(String),
    /// GitHub's rate limit for this address: no request before then (seconds since the epoch).
    RateLimited {
        /// When it ends.
        until: u64,
    },
    /// GitHub answered with an error.
    Http(u16, String),
    /// A package or its signature was refused (and removed).
    Refused(String),
    /// Something else (a bad answer, the disk).
    Other(String),
}

impl std::fmt::Display for CheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckError::Offline(e) => write!(f, "GitHub can't be reached (offline?): {e}"),
            CheckError::RateLimited { until } => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs());
                let mins = until.saturating_sub(now).div_ceil(60);
                write!(
                    f,
                    "GitHub's rate limit for this network was reached: checking again in {mins} min"
                )
            }
            CheckError::Http(s, m) => write!(f, "GitHub answered {s}: {m}"),
            CheckError::Refused(m) => write!(f, "refused: {m}"),
            CheckError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for CheckError {}

/// A package to download: a release's asset for a product and platform, and its signature.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    /// Its release's tag.
    pub tag: String,
    /// The release's version.
    pub version: String,
    /// A pre-release.
    pub prerelease: bool,
    /// The product.
    pub product: String,
    /// The platform.
    pub platform: String,
    /// The package.
    pub file: Asset,
    /// Its `.minisig`.
    pub signature: Asset,
    /// Its SHA-256 as the manifest says (checked when there).
    pub sha256: Option<String>,
}

/// A downloaded package whose signature held for the product, version and platform asked for.
#[derive(Clone, Debug)]
pub struct Downloaded {
    /// The file, in the staging folder (the caller moves or removes it).
    pub path: PathBuf,
    /// Its name in the release.
    pub file_name: String,
    /// The signature's text.
    pub signature: String,
    /// What the signature says.
    pub verified: Verified,
}

/// A client of one repository's releases.
pub struct GitHub {
    api: String,
    repository: String,
    agent: ureq::Agent,
    /// The last list and its ETag.
    listing: Mutex<Option<(String, Vec<Release>)>>,
    /// Manifests by tag.
    manifests: Mutex<HashMap<String, Manifest>>,
    /// No request before then (a rate limit), seconds since the epoch.
    blocked_until: Mutex<u64>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A repository name: `owner/name`, GitHub's characters only.
pub fn valid_repository(r: &str) -> bool {
    let mut parts = r.split('/');
    let ok = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.len() <= 100
                && p != "."
                && p != ".."
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

impl GitHub {
    /// A client of `repository` (`owner/name`) at GitHub's API `api` ([`DEFAULT_API`]; tests
    /// pass a local server). `user_agent` names the program (GitHub needs one).
    pub fn new(api: &str, repository: &str, user_agent: &str) -> Result<Self, CheckError> {
        if !valid_repository(repository) {
            return Err(CheckError::Other(format!(
                "{repository:?} isn't a GitHub repository (owner/name)"
            )));
        }
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .http_status_as_error(false)
            .user_agent(user_agent)
            .build()
            .into();
        Ok(GitHub {
            api: api.trim_end_matches('/').to_string(),
            repository: repository.to_string(),
            agent,
            listing: Mutex::new(None),
            manifests: Mutex::new(HashMap::new()),
            blocked_until: Mutex::new(0),
        })
    }

    /// The repository.
    pub fn repository(&self) -> &str {
        &self.repository
    }

    fn get(
        &self,
        url: &str,
        etag: Option<&str>,
    ) -> Result<ureq::http::Response<ureq::Body>, CheckError> {
        let until = *lock(&self.blocked_until);
        if until > unix_now() {
            return Err(CheckError::RateLimited { until });
        }
        let mut req = self.agent.get(url);
        if url.starts_with(&self.api) {
            req = req
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28");
        }
        if let Some(e) = etag {
            req = req.header("If-None-Match", e);
        }
        let r = req.call().map_err(|e| match e {
            ureq::Error::Io(e) => CheckError::Offline(e.to_string()),
            ureq::Error::HostNotFound => CheckError::Offline("host not found".into()),
            ureq::Error::ConnectionFailed => CheckError::Offline("connection failed".into()),
            ureq::Error::Timeout(t) => CheckError::Offline(format!("timed out ({t})")),
            e => CheckError::Other(e.to_string()),
        })?;
        let status = r.status().as_u16();
        let header = |n: &str| {
            r.headers()
                .get(n)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let limited = status == 429
            || (status == 403
                && (header("x-ratelimit-remaining").as_deref() == Some("0")
                    || header("retry-after").is_some()));
        if limited {
            let now = unix_now();
            let until = header("retry-after")
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|s| now + s)
                .or_else(|| header("x-ratelimit-reset").and_then(|s| s.trim().parse::<u64>().ok()))
                .filter(|t| *t > now)
                .unwrap_or(now + RATE_LIMIT_WAIT.as_secs());
            *lock(&self.blocked_until) = until;
            return Err(CheckError::RateLimited { until });
        }
        Ok(r)
    }

    fn fail(mut r: ureq::http::Response<ureq::Body>) -> CheckError {
        let status = r.status().as_u16();
        let body = r
            .body_mut()
            .with_config()
            .limit(4096)
            .read_to_string()
            .unwrap_or_default();
        let message = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["message"].as_str().map(str::to_string))
            .unwrap_or_else(|| body.trim().chars().take(200).collect());
        CheckError::Http(status, message)
    }

    /// The repository's releases of Vesper's lines (drafts and other tags left out), newest
    /// version first per line. Asks GitHub with the last list's ETag: unchanged, the list kept
    /// is returned.
    pub fn releases(&self) -> Result<Vec<Release>, CheckError> {
        let url = format!(
            "{}/repos/{}/releases?per_page=100",
            self.api, self.repository
        );
        let etag = lock(&self.listing).as_ref().map(|(e, _)| e.clone());
        let mut r = self.get(&url, etag.as_deref())?;
        match r.status().as_u16() {
            304 => {
                if let Some((_, l)) = lock(&self.listing).as_ref() {
                    return Ok(l.clone());
                }
                Err(CheckError::Other(
                    "GitHub answered 304 to a first request".into(),
                ))
            }
            200 => {
                let etag = r
                    .headers()
                    .get("etag")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                let body = r
                    .body_mut()
                    .with_config()
                    .limit(MAX_LISTING)
                    .read_to_string()
                    .map_err(|e| CheckError::Offline(e.to_string()))?;
                let raw: Vec<RawRelease> = serde_json::from_str(&body).map_err(|e| {
                    CheckError::Other(format!("GitHub's release list can't be read: {e}"))
                })?;
                let mut list: Vec<Release> = raw
                    .into_iter()
                    .filter(|r| !r.draft)
                    .filter_map(|r| {
                        let (line, version) = parse_tag(&r.tag_name)?;
                        Some(Release {
                            tag: r.tag_name,
                            line,
                            version,
                            prerelease: r.prerelease,
                            assets: r.assets,
                        })
                    })
                    .collect();
                list.sort_by(|a, b| {
                    a.line
                        .prefix
                        .cmp(b.line.prefix)
                        .then_with(|| compare_versions(&b.version, &a.version))
                });
                *lock(&self.listing) = etag.map(|e| (e, list.clone()));
                Ok(list)
            }
            _ => Err(Self::fail(r)),
        }
    }

    /// A release's manifest (kept per tag).
    pub fn manifest(&self, r: &Release) -> Result<Manifest, CheckError> {
        if let Some(m) = lock(&self.manifests).get(&r.tag) {
            return Ok(m.clone());
        }
        let a = r
            .asset(MANIFEST)
            .ok_or_else(|| CheckError::Other(format!("the release {} has no {MANIFEST}", r.tag)))?;
        let mut resp = self.get(&a.url, None)?;
        if resp.status().as_u16() != 200 {
            return Err(Self::fail(resp));
        }
        let body = resp
            .body_mut()
            .with_config()
            .limit(MAX_MANIFEST)
            .read_to_string()
            .map_err(|e| CheckError::Offline(e.to_string()))?;
        let m: Manifest = serde_json::from_str(&body).map_err(|e| {
            CheckError::Other(format!(
                "the release {}'s {MANIFEST} can't be read: {e}",
                r.tag
            ))
        })?;
        lock(&self.manifests).insert(r.tag.clone(), m.clone());
        Ok(m)
    }

    /// `r`'s package for `product` on `platform`, with its signature (`None`: it has none).
    pub fn candidate(
        &self,
        r: &Release,
        product: &str,
        platform: &str,
    ) -> Result<Option<Candidate>, CheckError> {
        let m = self.manifest(r)?;
        Ok(m.assets
            .iter()
            .filter(|a| a.product == product && a.platform == platform)
            .find_map(|a| {
                Some(Candidate {
                    tag: r.tag.clone(),
                    version: r.version.clone(),
                    prerelease: r.prerelease,
                    product: product.to_string(),
                    platform: platform.to_string(),
                    file: r.asset(&a.file)?.clone(),
                    signature: r.asset(&format!("{}.minisig", a.file))?.clone(),
                    sha256: a.sha256.clone(),
                })
            }))
    }

    /// The newest release of `product` in `channel` with a package for `platform`.
    pub fn latest(
        &self,
        releases: &[Release],
        product: &str,
        platform: &str,
        channel: ReleaseChannel,
    ) -> Result<Option<Candidate>, CheckError> {
        let Some(line) = line_of(product) else {
            return Ok(None);
        };
        let mut rs: Vec<&Release> = releases
            .iter()
            .filter(|r| r.line == line && r.in_channel(channel))
            .collect();
        rs.sort_by(|a, b| compare_versions(&b.version, &a.version));
        for r in rs {
            if let Some(c) = self.candidate(r, product, platform)? {
                return Ok(Some(c));
            }
        }
        Ok(None)
    }

    /// `product` `version` for `platform` (any channel).
    pub fn find(
        &self,
        releases: &[Release],
        product: &str,
        version: &str,
        platform: &str,
    ) -> Result<Option<Candidate>, CheckError> {
        let Some(line) = line_of(product) else {
            return Ok(None);
        };
        for r in releases
            .iter()
            .filter(|r| r.line == line && same_version(version, &r.version))
        {
            if let Some(c) = self.candidate(r, product, platform)? {
                return Ok(Some(c));
            }
        }
        Ok(None)
    }

    /// Downloads `url` into `to`, at most `max` bytes, hashing it.
    fn fetch_to(
        &self,
        url: &str,
        to: &mut dyn Write,
        max: u64,
    ) -> Result<crate::sign::Digests, CheckError> {
        let resp = self.get(url, None)?;
        if resp.status().as_u16() != 200 {
            return Err(Self::fail(resp));
        }
        let mut reader = resp.into_body().into_reader();
        let mut h = Hasher::new();
        let mut buf = vec![0u8; 256 * 1024];
        let mut total = 0u64;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| CheckError::Offline(e.to_string()))?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            total += n as u64;
            if total > max {
                return Err(CheckError::Refused(format!(
                    "{url} is larger than the {max} bytes expected"
                )));
            }
            to.write_all(&buf[..n])
                .map_err(|e| CheckError::Other(format!("can't write the download: {e}")))?;
        }
        Ok(h.finish())
    }

    /// Downloads `c`'s package and signature into `staging` and verifies them against `keys`:
    /// the signature must hold, and its trusted comment must name `c`'s product, version and
    /// platform. Refused (and removed): a wrong or missing signature, a mismatch, a file larger
    /// than listed or than [`MAX_PACKAGE`], a SHA-256 other than the manifest's.
    pub fn download(
        &self,
        c: &Candidate,
        staging: &Path,
        keys: &[PublicKey],
    ) -> Result<Downloaded, CheckError> {
        std::fs::create_dir_all(staging)
            .map_err(|e| CheckError::Other(format!("can't make {}: {e}", staging.display())))?;
        let mut sig = Vec::new();
        self.fetch_to(&c.signature.url, &mut sig, MAX_SIGNATURE)?;
        let sig = String::from_utf8(sig)
            .map_err(|_| CheckError::Refused(format!("{} isn't text", c.signature.name)))?;
        let mut rnd = [0u8; 8];
        getrandom::fill(&mut rnd).map_err(|e| CheckError::Other(e.to_string()))?;
        let id: String = rnd.iter().map(|b| format!("{b:02x}")).collect();
        let path = staging.join(format!(".github-{id}.part"));
        let r = (|| {
            let mut f = std::fs::File::create(&path)
                .map_err(|e| CheckError::Other(format!("can't write {}: {e}", path.display())))?;
            let max = c.file.size.min(MAX_PACKAGE);
            let d = self.fetch_to(&c.file.url, &mut f, max)?;
            f.sync_all()
                .map_err(|e| CheckError::Other(format!("can't write the download: {e}")))?;
            if d.size != c.file.size {
                return Err(CheckError::Refused(format!(
                    "{} is {} bytes, GitHub lists {}",
                    c.file.name, d.size, c.file.size
                )));
            }
            if let Some(want) = &c.sha256
                && !want.eq_ignore_ascii_case(&d.sha256)
            {
                return Err(CheckError::Refused(format!(
                    "{}'s SHA-256 isn't the one in the release's manifest",
                    c.file.name
                )));
            }
            let v = package::verify(keys, &d, &sig)
                .map_err(|e| CheckError::Refused(format!("{}: {e}", c.file.name)))?;
            check_meta(&v, c)?;
            Ok(v)
        })();
        match r {
            Ok(verified) => Ok(Downloaded {
                path,
                file_name: c.file.name.clone(),
                signature: sig,
                verified,
            }),
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                Err(e)
            }
        }
    }
}

/// The trusted comment must name what was asked for.
fn check_meta(v: &Verified, c: &Candidate) -> Result<(), CheckError> {
    let m = &v.meta;
    if m.product != c.product || m.platform != c.platform || !same_version(&m.version, &c.version) {
        return Err(CheckError::Refused(format!(
            "{} is signed as {} {} for {}, but the release {} offers it as {} {} for {}",
            c.file.name, m.product, m.version, m.platform, c.tag, c.product, c.version, c.platform
        )));
    }
    Ok(())
}

/// The products the releases offer: per product, the newest stable version, the newest beta
/// when newer, and the platforms of the newest release (from its manifest).
pub fn catalogue(gh: &GitHub, releases: &[Release]) -> Result<Vec<CatalogueProduct>, CheckError> {
    let mut out: BTreeMap<String, CatalogueProduct> = BTreeMap::new();
    for line in LINES {
        let mut rs: Vec<&Release> = releases.iter().filter(|r| r.line == line).collect();
        rs.sort_by(|a, b| compare_versions(&b.version, &a.version));
        let stable = rs.iter().find(|r| !r.prerelease);
        let beta = rs
            .iter()
            .find(|r| r.prerelease)
            .filter(|b| stable.is_none_or(|s| compare_versions(&b.version, &s.version).is_gt()));
        for product in line.products {
            let mut platforms = Vec::new();
            for r in [stable, beta].into_iter().flatten() {
                for a in gh.manifest(r)?.assets {
                    if a.product == *product && !platforms.contains(&a.platform) {
                        platforms.push(a.platform);
                    }
                }
            }
            if platforms.is_empty() {
                continue;
            }
            platforms.sort();
            out.insert(
                product.to_string(),
                CatalogueProduct {
                    product: product.to_string(),
                    name: product_name(product),
                    stable: stable.map(|r| r.version.clone()),
                    beta: beta.map(|r| r.version.clone()),
                    platforms,
                    sources: vec!["github".into()],
                },
            );
        }
    }
    Ok(out.into_values().collect())
}

/// Adds the site's update channel's packages to a catalogue: per product the newest version
/// there (as stable) and its platforms; a product in both shows both sources.
pub fn merge_channel(cat: &mut Vec<CatalogueProduct>, packages: &[UpdatePackage]) {
    for p in packages {
        let i = match cat.iter().position(|c| c.product == p.product) {
            Some(i) => i,
            None => {
                cat.push(CatalogueProduct {
                    product: p.product.clone(),
                    name: product_name(&p.product),
                    stable: None,
                    beta: None,
                    platforms: Vec::new(),
                    sources: Vec::new(),
                });
                cat.len() - 1
            }
        };
        let c = &mut cat[i];
        if !c.platforms.contains(&p.platform) {
            c.platforms.push(p.platform.clone());
            c.platforms.sort();
        }
        if !c.sources.iter().any(|s| s == "channel") {
            c.sources.insert(0, "channel".into());
        }
        if c.stable
            .as_deref()
            .is_none_or(|v| compare_versions(&p.version, v).is_gt())
        {
            c.stable = Some(p.version.clone());
        }
        if c.beta
            .as_deref()
            .zip(c.stable.as_deref())
            .is_some_and(|(b, s)| !compare_versions(b, s).is_gt())
        {
            c.beta = None;
        }
    }
    cat.sort_by_key(|c| c.name.to_lowercase());
}

/// When the next check is due: `interval` after the last attempt, sooner after a failure (5
/// minutes, doubling, never more than `interval`), and never before a rate limit ends.
#[derive(Clone, Debug, Default)]
pub struct Schedule {
    last: Option<std::time::Instant>,
    failures: u32,
}

impl Schedule {
    /// Whether a check is due.
    pub fn due(&self, interval: Duration) -> bool {
        self.last.is_none_or(|t| t.elapsed() >= self.wait(interval))
    }

    /// How long after the last attempt the next one is.
    pub fn wait(&self, interval: Duration) -> Duration {
        if self.failures == 0 {
            interval
        } else {
            let back = Duration::from_secs(300) * 2u32.saturating_pow(self.failures - 1).min(1024);
            back.min(interval)
        }
    }

    /// Records an attempt and whether it worked.
    pub fn attempted(&mut self, ok: bool) {
        self.last = Some(std::time::Instant::now());
        self.failures = if ok {
            0
        } else {
            self.failures.saturating_add(1)
        };
    }

    /// Checks again at once (settings changed, "Check now").
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Consecutive failures.
    pub fn failures(&self) -> u32 {
        self.failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_name_their_line_and_version() {
        let (l, v) = parse_tag("server-v0.2.0").unwrap();
        assert_eq!((l.prefix, v.as_str()), ("server", "0.2.0"));
        let (l, v) = parse_tag("idn-node-v0.3.8-beta18").unwrap();
        assert_eq!((l.prefix, v.as_str()), ("idn-node", "0.3.8-beta18"));
        assert!(parse_tag("v0.2.0").is_none());
        assert!(parse_tag("server-0.2.0").is_none());
        assert!(parse_tag("server-vnext").is_none());
        assert!(parse_tag("mixer-v1.0.0").is_none());
        assert!(same_version("0.3.8~beta18", "0.3.8-beta18"));
        assert!(same_version("0.3.8~Beta18", "0.3.8-beta18"));
        assert!(!same_version("0.3.8", "0.3.8-beta18"));
    }

    #[test]
    fn repositories_are_checked() {
        assert!(valid_repository("ADC-Industries/Vesper"));
        assert!(!valid_repository("ADC-Industries"));
        assert!(!valid_repository("a/b/c"));
        assert!(!valid_repository("../x"));
        assert!(!valid_repository("a/b?x=1"));
    }

    #[test]
    fn failures_back_off_up_to_the_interval() {
        let six = Duration::from_secs(6 * 3600);
        let mut s = Schedule::default();
        assert!(s.due(six));
        s.attempted(false);
        assert_eq!(s.wait(six), Duration::from_secs(300));
        s.attempted(false);
        assert_eq!(s.wait(six), Duration::from_secs(600));
        for _ in 0..20 {
            s.attempted(false);
        }
        assert_eq!(s.wait(six), six);
        s.attempted(true);
        assert_eq!(s.wait(six), six);
        assert!(!s.due(six));
        s.reset();
        assert!(s.due(six));
    }

    #[test]
    fn the_channel_joins_the_catalogue() {
        let pkg = |product: &str, version: &str, platform: &str| UpdatePackage {
            id: format!("{product}{version}{platform}"),
            product: product.into(),
            version: version.into(),
            platform: platform.into(),
            file_name: "f".into(),
            size: 1,
            sha256: String::new(),
            key_id: String::new(),
            trusted_comment: String::new(),
            uploaded_at: String::new(),
            source: None,
        };
        let mut cat = vec![CatalogueProduct {
            product: "vesper".into(),
            name: "Vesper".into(),
            stable: Some("0.2.0".into()),
            beta: Some("0.3.0-beta1".into()),
            platforms: vec!["linux-x86_64".into()],
            sources: vec!["github".into()],
        }];
        merge_channel(
            &mut cat,
            &[
                pkg("vesper", "0.3.0", "windows-x86_64"),
                pkg("vesper-oriel", "0.1.0", "linux-x86_64"),
            ],
        );
        assert_eq!(cat.len(), 2);
        assert_eq!(cat[0].product, "vesper");
        assert_eq!(cat[0].stable.as_deref(), Some("0.3.0"));
        assert_eq!(cat[0].beta, None, "the beta is older than the channel's");
        assert_eq!(cat[0].platforms, ["linux-x86_64", "windows-x86_64"]);
        assert_eq!(cat[0].sources, ["channel", "github"]);
        assert_eq!(cat[1].name, "Vesper Oriel");
        assert_eq!(cat[1].sources, ["channel"]);
    }
}
