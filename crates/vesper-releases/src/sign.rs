//! Package signatures: Ed25519 in [minisign](https://jedisct1.github.io/minisign/)'s format, so
//! the owner signs releases with the stock `minisign` tool and keeps the private key offline
//! (`docs/LAUNCHER-PLAN.md`).
//!
//! Only minisign's prehashed signatures (`ED`, its default) are accepted: the signature is over
//! the package's BLAKE2b-512 hash, so a package is verified while it streams to disk. The
//! *trusted comment* is signed too (the global signature), and carries the package's product,
//! version and platform ([`crate::package::PackageMeta`]).
//!
//! ```text
//! untrusted comment: <anything>
//! base64( "ED" | key id (8 bytes) | Ed25519 signature of BLAKE2b-512(file) (64 bytes) )
//! trusted comment: vesper-package product=vesper version=0.2.0 platform=linux-x86_64
//! base64( Ed25519 signature of (the signature above | the trusted comment) (64 bytes) )
//! ```

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use blake2::Digest;
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use std::fmt;

/// The owner's release public keys (minisign `.pub` text or its base64 line), compiled in.
/// Two keys, both minisign (only their public halves are in the repository, `packaging/keys/`):
/// - `vesper-automation.pub` (A05D75D53CB6819A): signs every release; the release automation on
///   the build machine holds its private key (owner's decision, 2026-11-26);
/// - `vesper-release.pub` (FB82D64871794689): the owner's own key, generated on their machine and
///   kept offline; it signs only a release that replaces the automation key (if it is ever lost
///   or exposed).
///
/// More keys come from the admin-only key folder (the Launcher's `Config::key_dir`) and, on
/// the Vesper server, `--update-key`.
pub const RELEASE_KEYS: &[&str] = &[
    include_str!("../../../packaging/keys/vesper-automation.pub"),
    include_str!("../../../packaging/keys/vesper-release.pub"),
];

/// Why a key or signature was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignError(pub String);

impl fmt::Display for SignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SignError {}

fn bad(m: impl Into<String>) -> SignError {
    SignError(m.into())
}

/// A minisign public key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublicKey {
    /// Its key ID.
    pub key_id: [u8; 8],
    key: VerifyingKey,
}

/// A key ID as minisign prints it (16 upper-case hex digits).
pub fn key_id_hex(id: &[u8; 8]) -> String {
    format!("{:016X}", u64::from_le_bytes(*id))
}

/// The base64 line of a minisign file: the given line, or the one after the untrusted comment.
fn data_line(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
}

impl PublicKey {
    /// Reads a minisign `.pub` file (or just its base64 line).
    pub fn parse(text: &str) -> Result<Self, SignError> {
        let line = data_line(text).ok_or_else(|| bad("the public key is empty"))?;
        let b = B64
            .decode(line)
            .map_err(|_| bad("the public key isn't minisign's base64"))?;
        if b.len() != 42 || &b[..2] != b"Ed" {
            return Err(bad("this isn't a minisign Ed25519 public key"));
        }
        let key_id = b[2..10].try_into().expect("8 bytes");
        let key = VerifyingKey::from_bytes(b[10..42].try_into().expect("32 bytes"))
            .map_err(|_| bad("the public key isn't a valid Ed25519 key"))?;
        Ok(PublicKey { key_id, key })
    }

    /// Its key ID as minisign prints it.
    pub fn key_id_hex(&self) -> String {
        key_id_hex(&self.key_id)
    }

    /// As a minisign `.pub` file.
    pub fn to_text(&self) -> String {
        let mut b = b"Ed".to_vec();
        b.extend_from_slice(&self.key_id);
        b.extend_from_slice(self.key.as_bytes());
        format!(
            "untrusted comment: minisign public key {}\n{}\n",
            self.key_id_hex(),
            B64.encode(b)
        )
    }
}

/// A minisign signature file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageSignature {
    /// The key it was made with.
    pub key_id: [u8; 8],
    signature: [u8; 64],
    /// The signed trusted comment.
    pub trusted_comment: String,
    global: [u8; 64],
}

impl PackageSignature {
    /// Reads a minisign `.minisig` file.
    pub fn parse(text: &str) -> Result<Self, SignError> {
        let mut lines = text.lines().map(|l| l.trim_end_matches('\r'));
        let first = lines.next().ok_or_else(|| bad("the signature is empty"))?;
        if !first.starts_with("untrusted comment:") {
            return Err(bad(
                "this isn't a minisign signature file (no untrusted comment)",
            ));
        }
        let sig_line = lines
            .next()
            .ok_or_else(|| bad("the signature has no signature line"))?;
        let b = B64
            .decode(sig_line.trim())
            .map_err(|_| bad("the signature line isn't base64"))?;
        if b.len() != 74 {
            return Err(bad("the signature line has the wrong length"));
        }
        match &b[..2] {
            b"ED" => {}
            b"Ed" => {
                return Err(bad(
                    "legacy (not prehashed) minisign signatures aren't accepted: sign without -l",
                ));
            }
            _ => return Err(bad("this isn't an Ed25519 minisign signature")),
        }
        let comment = lines
            .next()
            .and_then(|l| l.strip_prefix("trusted comment: "))
            .ok_or_else(|| bad("the signature has no trusted comment"))?;
        let global = lines
            .next()
            .ok_or_else(|| bad("the signature has no global signature"))?;
        let global = B64
            .decode(global.trim())
            .map_err(|_| bad("the global signature isn't base64"))?;
        let global: [u8; 64] = global
            .try_into()
            .map_err(|_| bad("the global signature has the wrong length"))?;
        Ok(PackageSignature {
            key_id: b[2..10].try_into().expect("8 bytes"),
            signature: b[10..74].try_into().expect("64 bytes"),
            trusted_comment: comment.to_string(),
            global,
        })
    }

    /// As a minisign `.minisig` file.
    pub fn to_text(&self) -> String {
        let mut b = b"ED".to_vec();
        b.extend_from_slice(&self.key_id);
        b.extend_from_slice(&self.signature);
        format!(
            "untrusted comment: signature from minisign secret key\n{}\ntrusted comment: {}\n{}\n",
            B64.encode(b),
            self.trusted_comment,
            B64.encode(self.global)
        )
    }

    /// The key ID as minisign prints it.
    pub fn key_id_hex(&self) -> String {
        key_id_hex(&self.key_id)
    }
}

/// A package's digests, computed while it is read or streamed.
#[derive(Clone)]
pub struct Hasher {
    blake2b: blake2::Blake2b512,
    sha256: sha2::Sha256,
    size: u64,
}

/// What [`Hasher`] computed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Digests {
    /// BLAKE2b-512, what the signature is over.
    pub blake2b: [u8; 64],
    /// SHA-256 (hex), the package's ID in the channel.
    pub sha256: String,
    /// Bytes.
    pub size: u64,
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl Hasher {
    /// A new hasher.
    pub fn new() -> Self {
        Hasher {
            blake2b: blake2::Blake2b512::new(),
            sha256: <sha2::Sha256 as sha2::Digest>::new(),
            size: 0,
        }
    }

    /// Adds bytes.
    pub fn update(&mut self, b: &[u8]) {
        self.blake2b.update(b);
        sha2::Digest::update(&mut self.sha256, b);
        self.size += b.len() as u64;
    }

    /// The digests.
    pub fn finish(self) -> Digests {
        let sha = sha2::Digest::finalize(self.sha256);
        Digests {
            blake2b: self.blake2b.finalize().into(),
            sha256: sha.iter().map(|b| format!("{b:02x}")).collect(),
            size: self.size,
        }
    }

    /// The digests of a whole file.
    pub fn file(path: &std::path::Path) -> std::io::Result<Digests> {
        use std::io::Read;
        let mut f = std::fs::File::open(path)?;
        let mut h = Hasher::new();
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                return Ok(h.finish());
            }
            h.update(&buf[..n]);
        }
    }

    /// The digests of bytes.
    pub fn bytes(b: &[u8]) -> Digests {
        let mut h = Hasher::new();
        h.update(b);
        h.finish()
    }
}

/// Checks `sig` over a package with the BLAKE2b-512 hash `blake2b` against `keys`: the key it
/// names must be one of them, and both the signature and the trusted comment's global signature
/// must hold. The key that signed it.
pub fn verify<'k>(
    keys: &'k [PublicKey],
    sig: &PackageSignature,
    blake2b: &[u8; 64],
) -> Result<&'k PublicKey, SignError> {
    if keys.is_empty() {
        return Err(bad(
            "no update signing key is configured, so no package can be trusted",
        ));
    }
    let key = keys
        .iter()
        .find(|k| k.key_id == sig.key_id)
        .ok_or_else(|| {
            bad(format!(
                "signed with key {}, which isn't one of this machine's update keys",
                sig.key_id_hex()
            ))
        })?;
    let s = ed25519_dalek::Signature::from_bytes(&sig.signature);
    key.key.verify_strict(blake2b, &s).map_err(|_| {
        bad("the signature doesn't match the package (it was changed, or isn't the signed file)")
    })?;
    let mut global = sig.signature.to_vec();
    global.extend_from_slice(sig.trusted_comment.as_bytes());
    let g = ed25519_dalek::Signature::from_bytes(&sig.global);
    key.key
        .verify_strict(&global, &g)
        .map_err(|_| bad("the signature's trusted comment was changed"))?;
    Ok(key)
}

/// Parses keys (each a minisign `.pub` text or base64 line); bad ones are errors.
pub fn parse_keys<S: AsRef<str>>(texts: &[S]) -> Result<Vec<PublicKey>, SignError> {
    texts.iter().map(|t| PublicKey::parse(t.as_ref())).collect()
}

/// A secret key for signing: **throwaway keys for tests and local experiments only**. The
/// owner's release key is generated and kept offline with `minisign -G`; it never comes near
/// this code or the repository.
pub struct TestSigner {
    key: SigningKey,
    /// Its key ID.
    pub key_id: [u8; 8],
}

impl TestSigner {
    /// A new random key.
    pub fn generate() -> Self {
        let mut seed = [0u8; 32];
        let mut id = [0u8; 8];
        getrandom::fill(&mut seed).expect("the operating system's random source failed");
        getrandom::fill(&mut id).expect("the operating system's random source failed");
        TestSigner {
            key: SigningKey::from_bytes(&seed),
            key_id: id,
        }
    }

    /// Its public key.
    pub fn public(&self) -> PublicKey {
        PublicKey {
            key_id: self.key_id,
            key: self.key.verifying_key(),
        }
    }

    /// A minisign signature (`.minisig` text) of a package with these digests.
    pub fn sign(&self, digests: &Digests, trusted_comment: &str) -> String {
        let signature = self.key.sign(&digests.blake2b).to_bytes();
        let mut global = signature.to_vec();
        global.extend_from_slice(trusted_comment.as_bytes());
        PackageSignature {
            key_id: self.key_id,
            signature,
            trusted_comment: trusted_comment.to_string(),
            global: self.key.sign(&global).to_bytes(),
        }
        .to_text()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_release_keys_are_compiled_in() {
        let keys = parse_keys(RELEASE_KEYS).expect("the release keys parse");
        let ids: Vec<String> = keys.iter().map(|k| k.key_id_hex()).collect();
        assert_eq!(ids, ["A05D75D53CB6819A", "FB82D64871794689"]);
    }

    const COMMENT: &str = "vesper-package product=vesper version=0.2.0 platform=linux-x86_64";

    #[test]
    fn a_good_signature_verifies() {
        let signer = TestSigner::generate();
        let keys = parse_keys(&[signer.public().to_text()]).unwrap();
        let d = Hasher::bytes(b"package bytes");
        let sig = PackageSignature::parse(&signer.sign(&d, COMMENT)).unwrap();
        let k = verify(&keys, &sig, &d.blake2b).unwrap();
        assert_eq!(k.key_id, signer.key_id);
        assert_eq!(sig.trusted_comment, COMMENT);
        // The text round-trips.
        assert_eq!(PackageSignature::parse(&sig.to_text()).unwrap(), sig);
    }

    #[test]
    fn a_tampered_package_or_comment_is_refused() {
        let signer = TestSigner::generate();
        let keys = vec![signer.public()];
        let d = Hasher::bytes(b"package bytes");
        let text = signer.sign(&d, COMMENT);
        let sig = PackageSignature::parse(&text).unwrap();
        let other = Hasher::bytes(b"package bytez");
        let e = verify(&keys, &sig, &other.blake2b).unwrap_err();
        assert!(e.0.contains("doesn't match"), "{e}");
        // A relabelled package: the trusted comment changed.
        let relabelled = text.replace("version=0.2.0", "version=9.9.9");
        let sig = PackageSignature::parse(&relabelled).unwrap();
        let e = verify(&keys, &sig, &d.blake2b).unwrap_err();
        assert!(e.0.contains("trusted comment"), "{e}");
    }

    #[test]
    fn a_wrong_key_is_refused() {
        let signer = TestSigner::generate();
        let stranger = TestSigner::generate();
        let d = Hasher::bytes(b"package bytes");
        let sig = PackageSignature::parse(&stranger.sign(&d, COMMENT)).unwrap();
        let e = verify(&[signer.public()], &sig, &d.blake2b).unwrap_err();
        assert!(e.0.contains("isn't one of"), "{e}");
        // The same key ID with another key (a forged ID) fails the signature.
        let sig = PackageSignature::parse(
            &TestSigner {
                key: SigningKey::from_bytes(&[7; 32]),
                key_id: signer.key_id,
            }
            .sign(&d, COMMENT),
        )
        .unwrap();
        assert!(verify(&[signer.public()], &sig, &d.blake2b).is_err());
        assert!(verify(&[], &sig, &d.blake2b).is_err());
    }

    #[test]
    fn a_minisign_file_is_read() {
        // A key and signature in minisign's own layout (made here with a throwaway key).
        let signer = TestSigner::generate();
        let pk = signer.public().to_text();
        assert!(pk.starts_with("untrusted comment: minisign public key "));
        assert_eq!(PublicKey::parse(&pk).unwrap(), signer.public());
        assert_eq!(
            PublicKey::parse(pk.lines().nth(1).unwrap()).unwrap(),
            signer.public()
        );
        assert!(PublicKey::parse("not a key").is_err());
        assert!(PackageSignature::parse("untrusted comment: x\nAAAA\n").is_err());
    }
}
