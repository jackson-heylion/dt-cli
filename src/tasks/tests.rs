use super::*;
use crate::{
    credentials::{CredentialStore, Credentials},
    login::Browser,
    output::Result,
    profile::{self, Environment},
};
use std::{collections::BTreeMap, time::Duration};
struct NoEffects;
impl CredentialStore for NoEffects {
    fn read_integrity_key(&self, _: &str) -> Result<Option<Vec<u8>>> {
        panic!("metadata key access")
    }
    fn write_integrity_key(&self, _: &str, _: &[u8]) -> Result<()> {
        panic!("metadata key access")
    }
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
fn runtime(root: &std::path::Path) -> Runtime {
    Runtime {
        root: root.into(),
        environments: BTreeMap::from([(
            "fixture".into(),
            Environment {
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
#[test]
fn chinese_tasks_and_operation_discovery_find_real_capabilities() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    for (query, id) in [
        ("待办", "inbox.brief"),
        ("整理待办", "inbox.brief"),
        ("抄送", "inbox.brief"),
        ("门店配置", "order-config.compare"),
        ("点赞", "like.send"),
    ] {
        let value = planning::list(&rt, Some(query), None).unwrap();
        assert_eq!(value["tasks"][0]["taskId"], id);
        assert_eq!(value["tasks"][0]["availability"]["state"], "blocked");
    }
    let value = crate::catalog::Catalog::shared().discover("待办", 5);
    assert_eq!(value["operations"][0]["operationId"], "iam.workflow.list");
    assert!(
        planning::list(&rt, Some("not-a-task"), None).unwrap()["tasks"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn task_parameters_reject_missing_duplicate_unknown_and_invalid_dates() {
    let brief = definitions::get("inbox.brief").unwrap();
    for raw in [
        "{\"kinds\":[\"todo\",\"todo\"]}",
        "{\"kinds\":[\"approval\"]}",
        "{\"maxPages\":21}",
        "{\"maxItems\":1001}",
        "{\"startedFrom\":\"2026-02-30\"}",
        "{\"startedFrom\":\"2026-10-07\",\"startedTo\":\"2026-10-06\"}",
        "{\"subjectId\":\"2\"}",
        "{\"title\":\"x\",\"title\":\"y\"}",
    ] {
        assert!(brief.validate(raw).is_err(), "{raw}");
    }
    assert!(brief.validate("{}").is_ok());
    let compare = definitions::get("order-config.compare").unwrap();
    assert!(
        compare
            .validate("{\"itemCode\":\"1\",\"shopCodes\":[\"A\",\"B\"]}")
            .is_ok()
    );
    assert!(
        compare
            .validate("{\"itemCode\":\"1\",\"itemName\":\"x\",\"shopCodes\":[\"A\",\"B\"]}")
            .is_err()
    );
    assert!(
        compare
            .validate("{\"itemCode\":\"1\",\"shopCodes\":[\"A\",\"A\"]}")
            .is_err()
    );
    assert!(
        definitions::get("like.send")
            .unwrap()
            .validate("{}")
            .is_err()
    );
}
#[test]
fn inbox_preview_resolves_unique_profile_without_credentials_or_writes() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    profile::save(
        dir.path(),
        "me",
        &profile::Profile {
            environment: "fixture".into(),
            subject_id: "1".into(),
            authorization_id: "auth".into(),
            authorization_expires_at: "2030-01-01T00:00:00Z".into(),
            access_expires_at: "2030-01-01T00:00:00Z".into(),
            checked_at: "2026-10-06T00:00:00Z".into(),
            credential_key: "fixture/1/auth".into(),
        },
    )
    .unwrap();
    let value = planning::plan(
        &rt,
        "inbox.brief",
        None,
        None,
        "{\"kinds\":[\"todo\",\"cc\"]}",
    )
    .unwrap();
    assert_eq!(value["result"]["binding"]["profile"], "me");
    assert_eq!(value["result"]["effect"], "read");
    assert_eq!(value["result"]["recordWrites"], 0);
    assert!(!dir.path().join("context").exists());
    assert!(!dir.path().join("workpacks").exists());
    let shown = planning::show(&rt, "inbox.brief", None, None).unwrap();
    assert_eq!(shown["availability"]["state"], "unknown");
    assert_eq!(shown["availability"]["permissionVerified"], false);
}

#[tokio::test]
async fn missing_run_status_is_local_and_does_not_create_records_or_read_keys() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    let failure = super::execution::status(&rt, &format!("-{}", "a".repeat(42)))
        .await
        .unwrap_err();
    assert_eq!(failure.code, "RUN_NOT_FOUND");
    assert!(!dir.path().join("workpacks").exists());
}
