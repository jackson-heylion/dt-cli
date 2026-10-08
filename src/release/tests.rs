use super::*;
const HOST: (&str, &str) = ("Darwin", "arm64");
struct FakeVerifier {
    broken: bool,
}
impl Verifier for FakeVerifier {
    fn probe(&self, binary: &Path) -> Result<Value> {
        if self.broken {
            return Err(failure("PACKAGE_VERIFICATION_FAILED", "fixture failure"));
        }
        let bytes = fs::read(binary).unwrap();
        let details: Value = serde_json::from_slice(&bytes).unwrap();
        Ok(json!({"ok":true,"data":details}))
    }
}
fn verifier() -> FakeVerifier {
    FakeVerifier { broken: false }
}
fn package(ver: &str, kind: &str) -> Package {
    let info = json!({"cliVersion":ver,"buildCommit":"b".repeat(40),"buildTarget":"aarch64-apple-darwin","localDevelopment":false,"catalogVersion":"1.2.0","catalogDigest":"a".repeat(64),"sourceVersion":"fixture"});
    let bytes = serde_json::to_vec(&info).unwrap();
    let manifest = Manifest {
        manifest_schema_version: 1,
        release_type: kind.into(),
        version: ver.into(),
        os: HOST.0.into(),
        architecture: HOST.1.into(),
        binary: "dt-cli".into(),
        sha256: sha(&bytes),
        build_commit: "b".repeat(40),
        build_target: "aarch64-apple-darwin".into(),
        local_development: false,
        catalog_version: "1.2.0".into(),
        catalog_digest: "a".repeat(64),
        source_version: "fixture".into(),
        profile_format: 1,
        credential_format: 1,
        minimum_installer_schema: 1,
        minimum_launcher_schema: 1,
        native_probe: if kind == "release" {
            "performed"
        } else {
            "not-performed"
        }
        .into(),
    };
    Package {
        manifest,
        bytes,
        archive_digest: "c".repeat(64),
    }
}
fn options(install: bool, candidate: bool) -> Options<'static> {
    Options {
        install,
        check: false,
        rollback: false,
        prune: false,
        candidate,
        expected: Some("cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"),
        signer: None,
    }
}
fn fixture() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("安装 目录");
    (dir, root)
}
#[test]
fn install_upgrade_rollback_use_immutable_versions_and_preserve_unrelated_data() {
    let (dir, root) = fixture();
    let profile = dir.path().join("profile.json");
    fs::write(&profile, b"original").unwrap();
    let first = package("0.3.0", "candidate");
    let key = first.manifest.sha256.clone();
    apply(&root, Some(first), &options(true, true), HOST, &verifier()).unwrap();
    let launcher = fs::read(root.join("bin/dt-cli")).unwrap();
    apply(
        &root,
        Some(package("0.4.0", "candidate")),
        &options(false, true),
        HOST,
        &verifier(),
    )
    .unwrap();
    let selected = Selection::read(&root).unwrap();
    assert_eq!(selected.previous.as_deref(), Some(key.as_str()));
    assert!(root.join("versions").join(key).join("dt-cli").exists());
    assert_eq!(fs::read(root.join("bin/dt-cli")).unwrap(), launcher);
    let mut rollback = options(false, false);
    rollback.rollback = true;
    rollback.expected = None;
    apply(&root, None, &rollback, HOST, &verifier()).unwrap();
    assert_eq!(status(&root, HOST).unwrap()["current"]["version"], "0.3.0");
    apply(&root, None, &rollback, HOST, &verifier()).unwrap();
    assert_eq!(status(&root, HOST).unwrap()["current"]["version"], "0.4.0");
    assert_eq!(fs::read(profile).unwrap(), b"original");
}
#[test]
fn bad_probe_preserves_current_and_rollback() {
    let (_dir, root) = fixture();
    apply(
        &root,
        Some(package("0.3.0", "release")),
        &options(true, false),
        HOST,
        &verifier(),
    )
    .unwrap();
    apply(
        &root,
        Some(package("0.4.0", "release")),
        &options(false, false),
        HOST,
        &verifier(),
    )
    .unwrap();
    let before = fs::read(root.join("active")).unwrap();
    let mut broken = verifier();
    broken.broken = true;
    assert!(
        apply(
            &root,
            Some(package("0.5.0", "release")),
            &options(false, false),
            HOST,
            &broken
        )
        .is_err()
    );
    assert_eq!(fs::read(root.join("active")).unwrap(), before);
}
#[test]
fn release_never_accepts_candidates_downgrades_or_format_changes() {
    let (_dir, root) = fixture();
    apply(
        &root,
        Some(package("0.4.0", "release")),
        &options(true, false),
        HOST,
        &verifier(),
    )
    .unwrap();
    let before = fs::read(root.join("active")).unwrap();
    for pkg in [package("0.5.0", "candidate"), package("0.3.0", "release")] {
        assert!(apply(&root, Some(pkg), &options(false, false), HOST, &verifier()).is_err());
    }
    let mut next = package("0.5.0", "release");
    next.manifest.credential_format = 2;
    assert!(next.manifest.validate(HOST).is_err());
    assert_eq!(fs::read(root.join("active")).unwrap(), before);
}
#[test]
fn repeated_upgrade_and_checks_do_not_destroy_rollback() {
    let (_dir, root) = fixture();
    apply(
        &root,
        Some(package("0.3.0", "release")),
        &options(true, false),
        HOST,
        &verifier(),
    )
    .unwrap();
    apply(
        &root,
        Some(package("0.4.0", "release")),
        &options(false, false),
        HOST,
        &verifier(),
    )
    .unwrap();
    let before = fs::read(root.join("active")).unwrap();
    assert_eq!(
        apply(
            &root,
            Some(package("0.4.0", "release")),
            &options(false, false),
            HOST,
            &verifier()
        )
        .unwrap()["changed"],
        false
    );
    let mut check = options(false, false);
    check.check = true;
    assert_eq!(
        apply(
            &root,
            Some(package("0.5.0", "release")),
            &check,
            HOST,
            &verifier()
        )
        .unwrap()["updateAvailable"],
        true
    );
    assert_eq!(fs::read(root.join("active")).unwrap(), before);
    let absent = root.join("absent");
    apply(&absent, None, &check, HOST, &verifier()).unwrap();
    assert!(!absent.exists());
}
#[test]
fn concurrent_upgrade_and_foreign_installation_are_refused() {
    let (_dir, root) = fixture();
    let held = lock(&root).unwrap();
    let error = apply(
        &root,
        Some(package("0.3.0", "candidate")),
        &options(true, true),
        HOST,
        &verifier(),
    )
    .unwrap_err();
    assert_eq!(error.code, "UPGRADE_BUSY");
    drop(held);
    fs::create_dir(&root).unwrap();
    fs::write(root.join("personal.txt"), b"keep").unwrap();
    assert!(
        apply(
            &root,
            Some(package("0.3.0", "candidate")),
            &options(true, true),
            HOST,
            &verifier()
        )
        .is_err()
    );
    assert_eq!(fs::read(root.join("personal.txt")).unwrap(), b"keep");
    assert!(!root.join("trust.json").exists());
}
#[test]
fn candidate_and_internal_release_require_explicit_archive_digest() {
    let (_dir, root) = fixture();
    let mut opts = options(true, true);
    opts.expected = None;
    assert!(
        apply(
            &root,
            Some(package("0.3.0", "candidate")),
            &opts,
            HOST,
            &verifier()
        )
        .is_err()
    );
    let (_dir, root) = fixture();
    let mut opts = options(true, false);
    opts.expected = None;
    assert!(
        apply(
            &root,
            Some(package("0.3.0", "release")),
            &opts,
            HOST,
            &verifier()
        )
        .is_err()
    );
}
fn zip(root: &Path, pkg: &Package, extra: Option<&str>) -> PathBuf {
    let path = root.join("fixture.zip");
    let mut writer = zip::ZipWriter::new(File::create(&path).unwrap());
    let opts = zip::write::SimpleFileOptions::default();
    writer.start_file("manifest.json", opts).unwrap();
    writer
        .write_all(&serde_json::to_vec(&pkg.manifest).unwrap())
        .unwrap();
    writer.start_file(&pkg.manifest.binary, opts).unwrap();
    writer.write_all(&pkg.bytes).unwrap();
    if let Some(name) = extra {
        writer.start_file(name, opts).unwrap();
        writer.write_all(b"unsafe").unwrap();
    }
    writer.finish().unwrap();
    path
}
#[test]
fn archive_hash_platform_and_path_traversal_are_rejected_before_installation() {
    let (dir, root) = fixture();
    let pkg = package("0.3.0", "candidate");
    let file = zip(dir.path(), &pkg, None);
    assert!(Package::read(&file, Some(&"0".repeat(64)), HOST).is_err());
    assert!(Package::read(&file, None, ("Windows", "x86_64")).is_err());
    let file = zip(dir.path(), &pkg, Some("../escaped"));
    assert!(Package::read(&file, None, HOST).is_err());
    assert!(!root.exists());
    assert!(!dir.path().join("escaped").exists());
}
#[test]
fn modified_provenance_and_binary_payload_cannot_be_installed() {
    let (dir, _root) = fixture();
    let mut pkg = package("0.3.0", "candidate");
    pkg.bytes.push(0);
    let file = zip(dir.path(), &pkg, None);
    assert!(Package::read(&file, None, HOST).is_err());
    let mut pkg = package("0.3.0", "candidate");
    pkg.manifest.catalog_digest = "d".repeat(64);
    assert!(
        pkg.manifest
            .matches_probe(
                &verifier()
                    .probe(&{
                        let path = dir.path().join("program");
                        fs::write(&path, &pkg.bytes).unwrap();
                        path
                    })
                    .unwrap()
            )
            .is_err()
    );
}
#[test]
fn internal_release_upgrades_legacy_pinned_installation_and_rolls_back_without_signer() {
    let (_dir, root) = fixture();
    apply(
        &root,
        Some(package("0.3.0", "release")),
        &options(true, false),
        HOST,
        &verifier(),
    )
    .unwrap();
    assert!(trust_at(&root).unwrap().identity.is_none());
    let mut legacy = trust_at(&root).unwrap();
    legacy.identity = Some(Identity {
        signer: "OLD-TEAM".into(),
        identifier: Some("old.cli".into()),
    });
    atomic_write(
        &root.join("trust.json"),
        &serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    let mut upgrade = options(false, false);
    upgrade.signer = Some("IGNORED-COMPATIBILITY-ARGUMENT");
    apply(
        &root,
        Some(package("0.4.0", "release")),
        &upgrade,
        HOST,
        &verifier(),
    )
    .unwrap();
    let mut rollback = options(false, false);
    rollback.rollback = true;
    rollback.expected = None;
    apply(&root, None, &rollback, HOST, &verifier()).unwrap();
    assert_eq!(status(&root, HOST).unwrap()["current"]["version"], "0.3.0");
}
#[test]
fn windows_internal_release_installs_without_authenticode_identity() {
    let (_dir, root) = fixture();
    let mut pkg = package("0.3.0", "release");
    let mut info: Value = serde_json::from_slice(&pkg.bytes).unwrap();
    info["buildTarget"] = json!("x86_64-pc-windows-msvc");
    pkg.bytes = serde_json::to_vec(&info).unwrap();
    pkg.manifest.os = "Windows".into();
    pkg.manifest.architecture = "x86_64".into();
    pkg.manifest.binary = "dt-cli.exe".into();
    pkg.manifest.build_target = "x86_64-pc-windows-msvc".into();
    pkg.manifest.sha256 = sha(&pkg.bytes);
    apply(
        &root,
        Some(pkg),
        &options(true, false),
        ("Windows", "x86_64"),
        &verifier(),
    )
    .unwrap();
    assert!(root.join("bin/dt-cli.exe").is_file());
    assert!(trust_at(&root).unwrap().identity.is_none());
}
#[test]
fn failed_first_install_is_retryable_without_partial_installation() {
    let (_dir, root) = fixture();
    let mut broken = verifier();
    broken.broken = true;
    assert!(
        apply(
            &root,
            Some(package("0.3.0", "candidate")),
            &options(true, true),
            HOST,
            &broken
        )
        .is_err()
    );
    assert!(!root.exists());
    apply(
        &root,
        Some(package("0.3.0", "candidate")),
        &options(true, true),
        HOST,
        &verifier(),
    )
    .unwrap();
    assert_eq!(status(&root, HOST).unwrap()["current"]["version"], "0.3.0");
}
#[cfg(unix)]
#[test]
fn version_symlink_refusal_preserves_current_and_foreign_files() {
    let (dir, root) = fixture();
    apply(
        &root,
        Some(package("0.3.0", "candidate")),
        &options(true, true),
        HOST,
        &verifier(),
    )
    .unwrap();
    let before = fs::read(root.join("active")).unwrap();
    let next = package("0.4.0", "candidate");
    let foreign = dir.path().join("foreign");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("personal.txt"), b"keep").unwrap();
    std::os::unix::fs::symlink(&foreign, root.join("versions").join(&next.manifest.sha256))
        .unwrap();
    assert!(apply(&root, Some(next), &options(false, true), HOST, &verifier()).is_err());
    assert_eq!(fs::read(root.join("active")).unwrap(), before);
    assert_eq!(fs::read(foreign.join("personal.txt")).unwrap(), b"keep");
}
#[test]
fn pruning_keeps_current_previous_and_foreign_files() {
    let (_dir, root) = fixture();
    let first = package("0.3.0", "candidate");
    let first_key = first.manifest.sha256.clone();
    apply(&root, Some(first), &options(true, true), HOST, &verifier()).unwrap();
    let second = package("0.4.0", "candidate");
    let second_key = second.manifest.sha256.clone();
    apply(
        &root,
        Some(second),
        &options(false, true),
        HOST,
        &verifier(),
    )
    .unwrap();
    apply(
        &root,
        Some(package("0.5.0", "candidate")),
        &options(false, true),
        HOST,
        &verifier(),
    )
    .unwrap();
    let before = fs::read(root.join("active")).unwrap();
    let mut opts = options(false, false);
    opts.prune = true;
    opts.expected = None;
    opts.check = true;
    assert_eq!(
        apply(&root, None, &opts, HOST, &verifier()).unwrap()["eligible"],
        json!([first_key])
    );
    assert!(root.join("versions").join(&first_key).exists());
    fs::write(
        root.join("versions").join(&first_key).join("personal.txt"),
        b"keep",
    )
    .unwrap();
    opts.check = false;
    assert_eq!(
        apply(&root, None, &opts, HOST, &verifier()).unwrap()["removed"],
        json!([])
    );
    fs::remove_file(root.join("versions").join(&first_key).join("personal.txt")).unwrap();
    assert_eq!(
        apply(&root, None, &opts, HOST, &verifier()).unwrap()["removed"],
        json!([first_key])
    );
    assert!(!root.join("versions").join(first_key).exists());
    assert!(root.join("versions").join(second_key).exists());
    assert_eq!(fs::read(root.join("active")).unwrap(), before);
}
