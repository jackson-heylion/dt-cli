use super::*;
use crate::{
    credentials::{CredentialStore, Credentials},
    login::Browser,
};
use std::{collections::BTreeMap, time::Duration};

struct NoEffects;
impl CredentialStore for NoEffects {
    fn read(&self, _: &str) -> Result<Option<Credentials>> {
        panic!("credential access")
    }
    fn write(&self, _: &str, _: &Credentials) -> Result<()> {
        panic!("credential access")
    }
    fn delete(&self, _: &str) -> Result<()> {
        panic!("credential access")
    }
}
impl Browser for NoEffects {
    fn open(&self, _: &str) -> Result<()> {
        panic!("browser access")
    }
}

fn runtime(root: &Path) -> Runtime {
    Runtime {
        root: root.into(),
        environments: BTreeMap::from([(
            "fixture".into(),
            profile::Environment {
                api_origin: "https://fixture.invalid".into(),
                portal_origin: "https://fixture.invalid".into(),
                recovery_url: "https://fixture.invalid/cli-authorizations.html".into(),
                launch_path: "/cli/launch".into(),
            },
        )]),
        store: Box::new(NoEffects),
        browser: Box::new(NoEffects),
        interactive: false,
        aggregate_budget: Duration::from_secs(30),
    }
}
fn personal(rt: &Runtime, name: &str, subject: &str) {
    profile::save(
        &rt.root,
        name,
        &profile::Profile {
            environment: "fixture".into(),
            subject_id: subject.into(),
            authorization_id: format!("auth-{name}"),
            authorization_expires_at: "2030-01-01T00:00:00Z".into(),
            access_expires_at: "2030-01-01T00:00:00Z".into(),
            checked_at: "2026-10-06T00:00:00Z".into(),
            credential_key: format!("fixture/{subject}/auth-{name}"),
        },
    )
    .unwrap();
}
fn governed(rt: &Runtime, name: &str, system: &str) {
    use sha2::{Digest, Sha256};
    fs::create_dir_all(rt.root.join("governed")).unwrap();
    let issuer = "https://fixture.invalid";
    let key = format!(
        "governed/{:x}/fixture/{system}/1/auth-{name}/dt-cli",
        Sha256::digest(issuer)
    );
    fs::write(rt.root.join("governed").join(format!("{name}.profile.json")), json!({"provider":"governed","issuer":issuer,
        "environment":"fixture","systemId":system,"subjectId":"1","authorizationId":format!("auth-{name}"),
        "authorizationExpiresAt":"2030-01-01T00:00:00Z","accessExpiresAt":"2030-01-01T00:00:00Z","credentialKey":key}).to_string()).unwrap();
}

#[test]
fn offline_list_keeps_valid_entries_when_another_profile_is_corrupt() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    personal(&rt, "me", "1");
    governed(&rt, "supply", "supply-chain-server");
    fs::write(dir.path().join("broken.json"), "{").unwrap();
    let result = list(&rt, false).unwrap();
    assert_eq!(result["profiles"].as_array().unwrap().len(), 3);
    assert_eq!(result["profiles"][0]["state"], "invalid");
    assert!(!result.to_string().contains("credentialKey"));
    assert!(!result.to_string().contains("authorizationId"));
    assert!(!result.to_string().contains("subjectId"));
    let detailed = list(&rt, true).unwrap();
    assert!(detailed.to_string().contains("authorizationId"));
    assert_eq!(result["onlineVerified"], false);
}

#[test]
fn unique_selection_and_explicit_choice_do_not_guess_between_accounts() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    assert_eq!(
        resolve(&rt, None, "personal", None, false)
            .unwrap_err()
            .code,
        "PROFILE_NOT_CONFIGURED"
    );
    personal(&rt, "one", "1");
    assert_eq!(
        resolve(&rt, None, "personal", None, false).unwrap().profile,
        "one"
    );
    personal(&rt, "two", "2");
    assert_eq!(
        resolve(&rt, None, "personal", None, false)
            .unwrap_err()
            .code,
        "PROFILE_REQUIRED"
    );
    assert_eq!(
        resolve(&rt, Some("two"), "personal", None, false)
            .unwrap()
            .subject_id,
        "2"
    );
    select(&rt, "one").unwrap();
    assert_eq!(
        resolve(&rt, None, "personal", None, false)
            .unwrap()
            .subject_id,
        "1"
    );
    assert_eq!(
        resolve(&rt, Some("two"), "personal", None, false)
            .unwrap()
            .subject_id,
        "2"
    );
}

#[test]
fn stale_or_wrong_default_is_not_replaced_by_a_different_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    personal(&rt, "me", "1");
    governed(&rt, "supply", "supply-chain-server");
    select(&rt, "supply").unwrap();
    assert_eq!(
        resolve(&rt, None, "personal", None, false)
            .unwrap_err()
            .code,
        "PROFILE_SELECTION_MISMATCH"
    );
    select(&rt, "me").unwrap();
    personal(&rt, "me", "2");
    assert_eq!(
        resolve(&rt, None, "personal", None, false)
            .unwrap_err()
            .code,
        "PROFILE_SELECTION_MISMATCH"
    );
}

#[test]
fn same_name_provider_conflicts_and_malformed_defaults_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    personal(&rt, "me", "1");
    governed(&rt, "me", "hrmp");
    assert_eq!(
        binding(&rt, "me").unwrap_err().code,
        "PROFILE_PROVIDER_MISMATCH"
    );
    fs::remove_file(dir.path().join("governed/me.profile.json")).unwrap();
    select(&rt, "me").unwrap();
    fs::write(
        selection_path(dir.path()),
        "{\"schemaVersion\":1,\"schemaVersion\":2}",
    )
    .unwrap();
    assert_eq!(
        resolve(&rt, None, "personal", None, false)
            .unwrap_err()
            .code,
        "LOCAL_STATE_UNAVAILABLE"
    );
    assert_eq!(list(&rt, false).unwrap()["selectionState"], "invalid");
}

#[test]
fn a_write_always_needs_explicit_matching_profile() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    governed(&rt, "hr", "hrmp");
    select(&rt, "hr").unwrap();
    assert_eq!(
        resolve(&rt, None, "governed", Some("hrmp"), true)
            .unwrap_err()
            .code,
        "PROFILE_REQUIRED"
    );
    assert_eq!(
        resolve(&rt, Some("hr"), "governed", Some("other"), true)
            .unwrap_err()
            .code,
        "PROFILE_SELECTION_MISMATCH"
    );
    assert_eq!(
        resolve(&rt, Some("hr"), "governed", Some("hrmp"), true)
            .unwrap()
            .system_id
            .as_deref(),
        Some("hrmp")
    );
}

#[tokio::test]
async fn setup_checks_binding_and_environment_before_interaction() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    personal(&rt, "me", "1");
    assert_eq!(
        setup(&rt, "me", "fixture", Some("hrmp"))
            .await
            .unwrap_err()
            .code,
        "PROFILE_SELECTION_MISMATCH"
    );
    assert_eq!(
        setup(&rt, "new", "prod", None).await.unwrap_err().code,
        "ENVIRONMENT_NOT_CONFIGURED"
    );
    assert_eq!(
        setup(&rt, "me", "fixture", None).await.unwrap_err().code,
        "INTERACTION_REQUIRED"
    );
    assert_eq!(
        setup(&rt, "hr", "fixture", Some("hrmp"))
            .await
            .unwrap_err()
            .code,
        "INTERACTION_REQUIRED"
    );
    assert_eq!(binding(&rt, "me").unwrap().subject_id, "1");
}

#[cfg(unix)]
#[test]
fn selection_rejects_symlinks_and_unsafe_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    personal(&rt, "me", "1");
    select(&rt, "me").unwrap();
    let path = selection_path(dir.path());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        resolve(&rt, None, "personal", None, false)
            .unwrap_err()
            .code,
        "LOCAL_STATE_UNAVAILABLE"
    );
    fs::remove_file(&path).unwrap();
    symlink(dir.path().join("me.json"), &path).unwrap();
    assert_eq!(
        select(&rt, "me").unwrap_err().code,
        "LOCAL_STATE_UNAVAILABLE"
    );
    assert!(path.is_symlink());
}

#[test]
fn seven_accounts_filter_to_business_system_without_a_saved_selection() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    for n in ["one", "two", "three", "four"] {
        personal(&rt, n, "1");
    }
    governed(&rt, "hr", "hrmp");
    governed(&rt, "other", "other-system");
    governed(&rt, "supply", "supply-chain-server");
    let filtered = list_filtered(&rt, false, Some("supply-chain-server"), Some("fixture")).unwrap();
    assert_eq!(filtered["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["profiles"][0]["profile"], "supply");
    assert_eq!(filtered["profiles"][0]["selected"], false);
    assert_eq!(
        resolve(&rt, None, "governed", Some("supply-chain-server"), false)
            .unwrap()
            .profile,
        "supply"
    );
    governed(&rt, "second-supply", "supply-chain-server");
    assert_eq!(
        resolve(&rt, None, "governed", Some("supply-chain-server"), false)
            .unwrap_err()
            .code,
        "PROFILE_REQUIRED"
    );
    assert_eq!(
        list_filtered(&rt, false, Some("supply-chain-server"), Some("missing")).unwrap()["profiles"],
        json!([])
    );
}
