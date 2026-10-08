use dt_cli::credentials::{CredentialStore, Credentials, FileStore};
use sha2::{Digest, Sha256};
fn credentials() -> Credentials {
    Credentials {
        access_token: "synthetic-access".into(),
        refresh_token: "synthetic-refresh".into(),
        version: 1,
        access_expires_at: None,
        authorization_expires_at: None,
        generation: 0,
        refresh_state: "ready".into(),
        refresh_request_id: None,
    }
}
fn path(root: &std::path::Path, key: &str) -> std::path::PathBuf {
    root.join("credentials")
        .join(format!("{:x}.json", Sha256::digest(key.as_bytes())))
}
#[test]
fn file_store_round_trip_rotation_reopen_and_idempotent_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let store = FileStore::new(root.path());
    let key = "governed/issuer/stg/hrmp/employee/authorization";
    assert!(store.read(key).unwrap().is_none());
    let mut value = credentials();
    store.write(key, &value).unwrap();
    value.access_token = "synthetic-rotated".into();
    value.generation = 1;
    value.refresh_request_id = Some("synthetic-nonce".into());
    store.write(key, &value).unwrap();
    let reopened = FileStore::new(root.path()).read(key).unwrap().unwrap();
    assert_eq!(reopened.access_token, value.access_token);
    assert_eq!(reopened.generation, 1);
    assert_eq!(reopened.refresh_request_id, value.refresh_request_id);
    assert!(store.read("another-authorization").unwrap().is_none());
    store.delete(key).unwrap();
    store.delete(key).unwrap();
    assert!(store.read(key).unwrap().is_none());
}
#[test]
fn integrity_key_is_separate_from_credentials_and_persists_across_upgrade() {
    let root = tempfile::tempdir().unwrap();
    let store = FileStore::new(root.path());
    store.write("oauth", &credentials()).unwrap();
    let bytes = vec![0x93u8; 32];
    store
        .write_integrity_key("journal-integrity", &bytes)
        .unwrap();
    let reopened = FileStore::new(root.path());
    assert_eq!(
        reopened
            .read_integrity_key("journal-integrity")
            .unwrap()
            .unwrap(),
        bytes
    );
    assert_eq!(
        reopened.read("oauth").unwrap().unwrap().access_token,
        "synthetic-access"
    );
    assert!(reopened.read("journal-integrity").is_err());
}
#[test]
fn malformed_record_is_distinguished_and_retained() {
    let root = tempfile::tempdir().unwrap();
    let store = FileStore::new(root.path());
    store.write("malformed", &credentials()).unwrap();
    let path = path(root.path(), "malformed");
    std::fs::write(&path, "synthetic-malformed").unwrap();
    let failure = store.read("malformed").err().unwrap();
    assert_eq!(failure.code, "CREDENTIAL_DECODE_FAILED");
    assert_eq!(
        std::fs::read_to_string(path).unwrap(),
        "synthetic-malformed"
    );
}
#[cfg(unix)]
#[test]
fn broad_permissions_and_symlinks_are_refused_without_touching_target() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let store = FileStore::new(root.path());
    store.write("private", &credentials()).unwrap();
    let file = path(root.path(), "private");
    assert_eq!(
        std::fs::metadata(file.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        store.read("private").err().unwrap().code,
        "CREDENTIAL_STORE_UNAVAILABLE"
    );
    assert!(store.write("private", &credentials()).is_err());
    assert!(store.delete("private").is_err());
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let retained = std::fs::read(&file).unwrap();
    symlink(&file, path(root.path(), "link")).unwrap();
    assert!(store.read("link").is_err());
    assert!(store.write("link", &credentials()).is_err());
    assert!(store.delete("link").is_err());
    assert_eq!(std::fs::read(file).unwrap(), retained);
}
