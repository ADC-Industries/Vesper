//! What a signed package is: its product, version and platform, read from the signature's
//! trusted comment (so they are signed too), and the platform names.
//!
//! The owner signs with
//! `minisign -S -m <file> -t "vesper-package product=vesper version=0.2.0 platform=linux-x86_64"`.

use crate::sign::{self, Digests, PackageSignature, PublicKey, SignError};

/// The trusted comment's first word.
pub const COMMENT_TAG: &str = "vesper-package";

/// This machine's platform name (`linux-x86_64`, `windows-x86_64`, `linux-aarch64`, …).
pub fn this_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// A package's signed description.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageMeta {
    /// The product (`vesper`, `oriel`, …).
    pub product: String,
    /// Its version.
    pub version: String,
    /// The platform it installs on.
    pub platform: String,
}

impl PackageMeta {
    /// Reads a trusted comment (`vesper-package product=… version=… platform=…`).
    pub fn from_comment(comment: &str) -> Result<Self, SignError> {
        let mut words = comment.split_whitespace();
        if words.next() != Some(COMMENT_TAG) {
            return Err(SignError(format!(
                "the signature's trusted comment doesn't start with \"{COMMENT_TAG}\": \
                 sign with -t \"{COMMENT_TAG} product=… version=… platform=…\""
            )));
        }
        let (mut product, mut version, mut platform) = (None, None, None);
        for w in words {
            if let Some((k, v)) = w.split_once('=') {
                let v = Some(v.to_string());
                match k {
                    "product" => product = v,
                    "version" => version = v,
                    "platform" => platform = v,
                    _ => {}
                }
            }
        }
        let need = |v: Option<String>, what: &str| {
            v.filter(|v| valid_word(v)).ok_or_else(|| {
                SignError(format!(
                    "the signature's trusted comment has no valid {what}="
                ))
            })
        };
        Ok(PackageMeta {
            product: need(product, "product")?,
            version: need(version, "version")?,
            platform: need(platform, "platform")?,
        })
    }

    /// The trusted comment for this description.
    pub fn comment(&self) -> String {
        format!(
            "{COMMENT_TAG} product={} version={} platform={}",
            self.product, self.version, self.platform
        )
    }
}

/// A product, version or platform: safe as a folder name.
pub fn valid_word(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+' | '~'))
}

/// A package whose signature held.
#[derive(Clone, Debug)]
pub struct Verified {
    /// What it is.
    pub meta: PackageMeta,
    /// Its digests.
    pub digests: Digests,
    /// The key that signed it.
    pub key_id: String,
    /// The signature's trusted comment.
    pub trusted_comment: String,
}

/// Verifies a package with digests `digests` and signature text `sig_text` against `keys`.
pub fn verify(
    keys: &[PublicKey],
    digests: &Digests,
    sig_text: &str,
) -> Result<Verified, SignError> {
    let sig = PackageSignature::parse(sig_text)?;
    let key = sign::verify(keys, &sig, &digests.blake2b)?;
    let meta = PackageMeta::from_comment(&sig.trusted_comment)?;
    Ok(Verified {
        meta,
        digests: digests.clone(),
        key_id: key.key_id_hex(),
        trusted_comment: sig.trusted_comment,
    })
}

/// Orders versions: dot-separated numbers compare as numbers (`0.10.0` > `0.9.2`). After
/// them, a pre-release (`-rc1`, `-beta.2`, `~beta1`) comes before the release, and a package
/// revision (`-1`, `-10`: a Debian revision, only digits; `+…`) after it: `1.0.0-rc1` <
/// `1.0.0` < `1.0.0-1`. Two suffixes of the same kind compare part by part, runs of digits
/// as numbers (`0.2.0-10` > `0.2.0-9`, `-rc10` > `-rc9`).
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    /// Pre-release < release < revision.
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    enum Kind {
        Pre,
        Release,
        Revision,
    }
    let split = |v: &str| -> (Vec<u64>, Kind, String) {
        let Some(at) = v.find(['-', '+', '~']) else {
            return (numbers(v), Kind::Release, String::new());
        };
        let (main, rest) = (&v[..at], &v[at + 1..]);
        let kind = match v.as_bytes()[at] {
            b'~' => Kind::Pre,
            b'+' => Kind::Revision,
            _ if !rest.is_empty() && rest.bytes().all(|c| c.is_ascii_digit() || c == b'.') => {
                Kind::Revision
            }
            _ => Kind::Pre,
        };
        (numbers(main), kind, rest.to_string())
    };
    let ((an, ak, ar), (bn, bk, br)) = (split(a), split(b));
    an.cmp(&bn)
        .then_with(|| ak.cmp(&bk))
        .then_with(|| natural(&ar, &br))
}

fn numbers(v: &str) -> Vec<u64> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// Compares runs of digits as numbers and everything else as text, in order.
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    fn runs(s: &str) -> Vec<(bool, &str)> {
        let mut v = Vec::new();
        let mut start = 0;
        let b = s.as_bytes();
        for i in 1..=b.len() {
            if i == b.len() || b[i].is_ascii_digit() != b[start].is_ascii_digit() {
                v.push((b[start].is_ascii_digit(), &s[start..i]));
                start = i;
            }
        }
        v
    }
    let (ra, rb) = (runs(a), runs(b));
    for (x, y) in ra.iter().zip(rb.iter()) {
        let o = match (x, y) {
            ((true, x), (true, y)) => {
                let (x, y) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                x.len().cmp(&y.len()).then_with(|| x.cmp(y))
            }
            ((_, x), (_, y)) => x.cmp(y),
        };
        if o != std::cmp::Ordering::Equal {
            return o;
        }
    }
    ra.len().cmp(&rb.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn the_trusted_comment_names_the_package() {
        let m = PackageMeta::from_comment(
            "vesper-package product=vesper version=0.2.0 platform=linux-x86_64",
        )
        .unwrap();
        assert_eq!(m.product, "vesper");
        assert_eq!(m.version, "0.2.0");
        assert_eq!(m.platform, "linux-x86_64");
        assert_eq!(PackageMeta::from_comment(&m.comment()).unwrap(), m);
        // minisign's default comment, a path in a name, a missing field.
        assert!(PackageMeta::from_comment("timestamp:1 file:x.deb").is_err());
        assert!(
            PackageMeta::from_comment("vesper-package product=../x version=1 platform=p").is_err()
        );
        assert!(PackageMeta::from_comment("vesper-package product=vesper version=1").is_err());
    }

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(compare_versions("0.10.0", "0.9.2"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0", "1.0.0-rc1"), Ordering::Greater);
        assert_eq!(compare_versions("1.0", "1.0.0"), Ordering::Less);
        assert_eq!(compare_versions("2.1.3", "2.1.3"), Ordering::Equal);
    }

    /// Debian revisions (`0.2.0-1`, the .deb packages) and numbered pre-releases compare
    /// by number, not as text: an older package is never offered as an update.
    #[test]
    fn revisions_and_pre_releases_compare_by_number() {
        for (newer, older) in [
            ("0.2.0-10", "0.2.0-9"),
            ("0.2.0-1", "0.2.0"),
            ("0.2.0", "0.2.0-rc1"),
            ("0.2.0-1", "0.2.0-rc1"),
            ("1.0.0-rc10", "1.0.0-rc9"),
            ("1.0.0-rc.2", "1.0.0-rc.1"),
            ("1.0.0-rc1", "1.0.0-beta2"),
            ("0.3.4-Beta10", "0.3.4-Beta9"),
            ("0.3.4", "0.3.4-Beta10"),
            ("1.0.0", "1.0.0~beta1"),
            ("1.0.0+2", "1.0.0+1"),
            ("0.2.1-1", "0.2.0-10"),
        ] {
            assert_eq!(
                compare_versions(newer, older),
                Ordering::Greater,
                "{newer} > {older}"
            );
            assert_eq!(
                compare_versions(older, newer),
                Ordering::Less,
                "{older} < {newer}"
            );
        }
        assert_eq!(compare_versions("0.2.0-01", "0.2.0-1"), Ordering::Equal);
    }
}
