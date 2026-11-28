//! The GitHub checker against a fake GitHub (synthetic releases, a throwaway signing key).

use protocol::ReleaseChannel;
use vesper_releases::fake::FakeGitHub;
use vesper_releases::github::{self, CheckError, GitHub};
use vesper_releases::sign::{self, PublicKey};

const LINUX: &str = "linux-x86_64";
const WINDOWS: &str = "windows-x86_64";

fn client(fake: &FakeGitHub) -> GitHub {
    GitHub::new(&fake.api(), "example/vesper", "vesper-test").unwrap()
}

fn keys(fake: &FakeGitHub) -> Vec<PublicKey> {
    sign::parse_keys(&[fake.public_key()]).unwrap()
}

/// Two server releases (a stable and a newer beta), an Oriel release and a tag of nothing of
/// ours.
fn site() -> FakeGitHub {
    let fake = FakeGitHub::start();
    fake.release(
        "server-v0.2.0",
        "0.2.0",
        false,
        &[
            (
                "vesper",
                LINUX,
                "vesper_0.2.0-2_amd64.deb",
                b"server 0.2.0 linux",
            ),
            (
                "vesper",
                WINDOWS,
                "vesper-server-0.2.0-setup.exe",
                b"server 0.2.0 win",
            ),
        ],
    );
    fake.release(
        "server-v0.3.0-beta1",
        "0.3.0-beta1",
        true,
        &[(
            "vesper",
            LINUX,
            "vesper_0.3.0-beta1-2_amd64.deb",
            b"server 0.3.0-beta1",
        )],
    );
    fake.release(
        "oriel-v0.2.0",
        "0.2.0",
        false,
        &[(
            "vesper-oriel",
            WINDOWS,
            "vesper-oriel_0.2.0_x64-setup.exe",
            b"oriel 0.2.0",
        )],
    );
    fake.raw_release("nightly", false, false, vec![]);
    fake.raw_release("server-v9.9.9", false, true, vec![]); // a draft
    fake
}

#[test]
fn releases_are_listed_by_line_without_drafts_or_other_tags() {
    let fake = site();
    let gh = client(&fake);
    let rs = gh.releases().unwrap();
    let tags: Vec<&str> = rs.iter().map(|r| r.tag.as_str()).collect();
    assert_eq!(
        tags,
        ["oriel-v0.2.0", "server-v0.3.0-beta1", "server-v0.2.0"]
    );
    assert!(rs[1].prerelease);
    assert_eq!(rs[1].line.prefix, "server");
    assert_eq!(rs[1].version, "0.3.0-beta1");
}

#[test]
fn the_channel_and_the_platform_pick_the_package() {
    let fake = site();
    let gh = client(&fake);
    let rs = gh.releases().unwrap();
    let c = gh
        .latest(&rs, "vesper", LINUX, ReleaseChannel::Stable)
        .unwrap()
        .unwrap();
    assert_eq!(
        (c.tag.as_str(), c.file.name.as_str()),
        ("server-v0.2.0", "vesper_0.2.0-2_amd64.deb")
    );
    let c = gh
        .latest(&rs, "vesper", LINUX, ReleaseChannel::Beta)
        .unwrap()
        .unwrap();
    assert_eq!(c.version, "0.3.0-beta1");
    // The beta has no Windows package: Windows stays on the stable one.
    let c = gh
        .latest(&rs, "vesper", WINDOWS, ReleaseChannel::Beta)
        .unwrap()
        .unwrap();
    assert_eq!(c.file.name, "vesper-server-0.2.0-setup.exe");
    assert!(
        gh.latest(&rs, "vesper-oriel", LINUX, ReleaseChannel::Beta)
            .unwrap()
            .is_none()
    );
    assert!(
        gh.latest(&rs, "vesper-idn-node", LINUX, ReleaseChannel::Beta)
            .unwrap()
            .is_none()
    );
    assert!(gh.find(&rs, "vesper", "0.2.0", WINDOWS).unwrap().is_some());
    assert!(gh.find(&rs, "vesper", "0.1.0", WINDOWS).unwrap().is_none());
    // Each manifest was downloaded once.
    let manifests = fake
        .requests()
        .iter()
        .filter(|r| r.path.ends_with("/manifest.json"))
        .count();
    assert_eq!(manifests, 3);
}

#[test]
fn an_unchanged_list_is_asked_for_with_its_etag() {
    let fake = site();
    let gh = client(&fake);
    gh.releases().unwrap();
    let again = gh.releases().unwrap();
    assert_eq!(again.len(), 3);
    let l = fake.listings();
    assert_eq!(l.len(), 2);
    assert_eq!(l[0].if_none_match, None);
    assert_eq!(l[1].status, 304);
    assert!(l[1].if_none_match.is_some());
    // A new release changes the ETag: the new list comes back.
    fake.release("oriel-v0.3.0", "0.3.0", false, &[]);
    assert_eq!(gh.releases().unwrap().len(), 4);
    assert_eq!(fake.listings()[2].status, 200);
}

#[test]
fn a_rate_limit_is_waited_out_without_asking_again() {
    let fake = site();
    let gh = client(&fake);
    let reset = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    fake.rate_limit(Some(reset));
    let e = gh.releases().unwrap_err();
    assert_eq!(e, CheckError::RateLimited { until: reset });
    assert!(e.to_string().contains("rate limit"), "{e}");
    // Until the reset, nothing is sent.
    fake.rate_limit(None);
    assert_eq!(
        gh.releases().unwrap_err(),
        CheckError::RateLimited { until: reset }
    );
    assert_eq!(fake.listings().len(), 1);
}

#[test]
fn offline_is_told_apart() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let gh = GitHub::new(&format!("http://127.0.0.1:{port}"), "example/vesper", "t").unwrap();
    let e = gh.releases().unwrap_err();
    assert!(matches!(e, CheckError::Offline(_)), "{e:?}");
    assert!(GitHub::new("http://x", "not a repository", "t").is_err());
}

#[test]
fn a_good_package_downloads_and_verifies() {
    let fake = site();
    let gh = client(&fake);
    let dir = tempfile::tempdir().unwrap();
    let rs = gh.releases().unwrap();
    let c = gh
        .latest(&rs, "vesper", WINDOWS, ReleaseChannel::Stable)
        .unwrap()
        .unwrap();
    let d = gh.download(&c, dir.path(), &keys(&fake)).unwrap();
    assert_eq!(std::fs::read(&d.path).unwrap(), b"server 0.2.0 win");
    assert_eq!(d.verified.meta.product, "vesper");
    assert_eq!(d.verified.meta.platform, WINDOWS);
    assert_eq!(d.verified.key_id, fake.signer.public().key_id_hex());
    // Not trusted: the compiled-in keys only.
    let release = sign::parse_keys(sign::RELEASE_KEYS).unwrap();
    let e = gh.download(&c, dir.path(), &release).unwrap_err();
    assert!(
        matches!(&e, CheckError::Refused(m) if m.contains("isn't one of")),
        "{e:?}"
    );
    // A refused download leaves nothing behind but the good one.
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_tampered_package_or_a_relabelled_signature_is_refused() {
    let fake = site();
    let gh = client(&fake);
    let dir = tempfile::tempdir().unwrap();
    // The package changed after signing (same size, so only the signature catches it).
    fake.release(
        "oriel-v0.3.0",
        "0.3.0",
        false,
        &[("vesper-oriel", LINUX, "o.deb", b"oriel good")],
    );
    fake.replace_file("oriel-v0.3.0", "o.deb", b"oriel evil".to_vec());
    let rs = gh.releases().unwrap();
    let mut c = gh
        .latest(&rs, "vesper-oriel", LINUX, ReleaseChannel::Stable)
        .unwrap()
        .unwrap();
    c.sha256 = None; // past the manifest's (untrusted) SHA-256: the signature still refuses it
    let e = gh.download(&c, dir.path(), &keys(&fake)).unwrap_err();
    assert!(
        matches!(&e, CheckError::Refused(m) if m.contains("doesn't match")),
        "{e:?}"
    );

    // A package signed for another platform (or version), offered as this one.
    let bytes = b"server 0.4.0 linux";
    fake.raw_release(
        "server-v0.4.0",
        false,
        false,
        vec![
            ("v.deb".into(), bytes.to_vec()),
            (
                "v.deb.minisig".into(),
                fake.sign(bytes, "vesper", "0.4.0", WINDOWS).into_bytes(),
            ),
            (
                "manifest.json".into(),
                br#"{"assets":[{"product":"vesper","platform":"linux-x86_64","file":"v.deb"}]}"#
                    .to_vec(),
            ),
        ],
    );
    let rs = gh.releases().unwrap();
    let c = gh
        .latest(&rs, "vesper", LINUX, ReleaseChannel::Stable)
        .unwrap()
        .unwrap();
    assert_eq!(c.version, "0.4.0");
    let e = gh.download(&c, dir.path(), &keys(&fake)).unwrap_err();
    assert!(
        matches!(&e, CheckError::Refused(m) if m.contains("signed as vesper 0.4.0 for windows")),
        "{e:?}"
    );

    // A signature from someone else's key.
    let stranger = sign::TestSigner::generate();
    let bytes = b"server 0.5.0 linux";
    let comment = "vesper-package product=vesper version=0.5.0 platform=linux-x86_64";
    fake.raw_release(
        "server-v0.5.0",
        false,
        false,
        vec![
            ("v5.deb".into(), bytes.to_vec()),
            (
                "v5.deb.minisig".into(),
                stranger
                    .sign(&sign::Hasher::bytes(bytes), comment)
                    .into_bytes(),
            ),
            (
                "manifest.json".into(),
                br#"{"assets":[{"product":"vesper","platform":"linux-x86_64","file":"v5.deb"}]}"#
                    .to_vec(),
            ),
        ],
    );
    let rs = gh.releases().unwrap();
    let c = gh.find(&rs, "vesper", "0.5.0", LINUX).unwrap().unwrap();
    let e = gh.download(&c, dir.path(), &keys(&fake)).unwrap_err();
    assert!(matches!(&e, CheckError::Refused(_)), "{e:?}");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn a_missing_signature_or_a_larger_file_is_refused() {
    let fake = site();
    let gh = client(&fake);
    let dir = tempfile::tempdir().unwrap();
    // No .minisig beside the package: nothing to offer.
    fake.raw_release(
        "server-v0.6.0",
        false,
        false,
        vec![
            ("v6.deb".into(), b"x".to_vec()),
            (
                "manifest.json".into(),
                br#"{"assets":[{"product":"vesper","platform":"linux-x86_64","file":"v6.deb"}]}"#
                    .to_vec(),
            ),
        ],
    );
    let rs = gh.releases().unwrap();
    assert!(gh.find(&rs, "vesper", "0.6.0", LINUX).unwrap().is_none());
    // The file grew after GitHub listed its size.
    let c = gh.find(&rs, "vesper", "0.2.0", LINUX).unwrap().unwrap();
    fake.replace_file("server-v0.2.0", "vesper_0.2.0-2_amd64.deb", vec![0; 4096]);
    let e = gh.download(&c, dir.path(), &keys(&fake)).unwrap_err();
    assert!(
        matches!(&e, CheckError::Refused(m) if m.contains("larger")),
        "{e:?}"
    );
}

#[test]
fn the_catalogue_lists_the_products_by_channel_and_platform() {
    let fake = site();
    let gh = client(&fake);
    let rs = gh.releases().unwrap();
    let cat = github::catalogue(&gh, &rs).unwrap();
    let names: Vec<&str> = cat.iter().map(|c| c.product.as_str()).collect();
    assert_eq!(names, ["vesper", "vesper-oriel"]);
    assert_eq!(cat[0].stable.as_deref(), Some("0.2.0"));
    assert_eq!(cat[0].beta.as_deref(), Some("0.3.0-beta1"));
    assert_eq!(cat[0].platforms, [LINUX, WINDOWS]);
    assert_eq!(cat[1].platforms, [WINDOWS]);
}
