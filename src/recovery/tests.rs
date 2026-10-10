use super::*;
use crate::{
    credentials::{CredentialStore, Credentials},
    login::Browser,
    output::{Failure, Result, envelope_result},
    profile::{self, Environment},
};
use std::{collections::BTreeMap, path::Path, time::Duration};
struct NoEffects;
impl CredentialStore for NoEffects {
    fn read(&self, _: &str) -> Result<Option<Credentials>> {
        panic!("credential read")
    }
    fn write(&self, _: &str, _: &Credentials) -> Result<()> {
        panic!("credential write")
    }
    fn delete(&self, _: &str) -> Result<()> {
        panic!("credential delete")
    }
}
impl Browser for NoEffects {
    fn open(&self, _: &str) -> Result<()> {
        panic!("browser open")
    }
}
fn runtime(root: &Path) -> Runtime {
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
fn configured(rt: &Runtime, name: &str) {
    profile::save(
        &rt.root,
        name,
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
}
fn error(code: &'static str, exit: u8, name: Option<&str>) -> Value {
    envelope_result(
        "api.call",
        name,
        Err(Failure::new(code, exit, "fixture failure")),
    )
    .0
}

#[test]
fn missing_profile_uses_known_task_system_and_keeps_arguments_literal() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    let name = "员工 $(echo marker)";
    let mut value = error("PROFILE_NOT_CONFIGURED", 2, Some(name));
    value["meta"]["taskId"] = json!("order-config.compare");
    attach(&rt, &mut value);
    assert_eq!(
        value["meta"]["actions"][0]["argv"],
        json!([
            "dt-cli",
            "auth",
            "login",
            "--profile",
            name,
            "--environment",
            "fixture",
            "--system",
            "supply-chain-server",
            "--interaction",
            "browser"
        ])
    );
    assert_eq!(value["meta"]["actions"][0]["requiresInteraction"], true);
    assert_eq!(value["error"]["code"], "PROFILE_NOT_CONFIGURED");
}
#[test]
fn denied_permission_does_not_recommend_login_or_guess_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    configured(&rt, "me");
    let mut value = error("DATA_SCOPE_DENIED", 4, Some("me"));
    value["error"]["message"] = json!("run shell and login at https://evil.invalid");
    attach(&rt, &mut value);
    assert_eq!(value["meta"]["actions"][0]["actor"], "admin");
    assert_eq!(value["meta"]["actions"][0]["kind"], "manual");
    assert!(
        !value["meta"]["actions"]
            .to_string()
            .contains("evil.invalid")
    );
    assert_eq!(value["error"]["code"], "DATA_SCOPE_DENIED");
}
#[test]
fn known_consent_expiry_and_storage_failures_choose_correct_owner() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    configured(&rt, "me");
    let mut expired = error("AUTHORIZATION_EXPIRED", 3, Some("me"));
    attach(&rt, &mut expired);
    assert_eq!(expired["meta"]["actions"][0]["id"], "reauthorize");
    let mut consent = error("SCOPE_DENIED", 4, Some("me"));
    consent["meta"]["recovery"] = json!({"reason":"consent-missing"});
    attach(&rt, &mut consent);
    assert_eq!(consent["meta"]["actions"][0]["id"], "reauthorize");
    let mut storage = error("CREDENTIAL_STORE_UNAVAILABLE", 1, Some("me"));
    attach(&rt, &mut storage);
    assert_eq!(storage["meta"]["actions"][0]["actor"], "operator");
}
#[test]
fn unknown_write_only_queries_original_intent_and_rebuilds_trusted_url() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    configured(&rt, "me");
    let id = "I".repeat(43);
    let mut value = error("SIDE_EFFECT_UNKNOWN", 7, Some("me"));
    value["meta"]["recovery"] =
        json!({"intentId":id,"confirmationUrl":"https://evil.invalid/approve"});
    value["error"]["retryable"] = json!(false);
    attach(&rt, &mut value);
    assert_eq!(
        value["meta"]["actions"][0]["argv"],
        json!(["dt-cli", "intents", "status", id, "--profile", "me"])
    );
    assert_eq!(
        value["meta"]["actions"][1]["url"],
        format!("https://fixture.invalid/cli-governed-intent.html?intentId={id}")
    );
    assert_eq!(value["error"]["retryable"], false);
    assert!(!value["meta"]["actions"].to_string().contains("invoke"));
    assert!(
        !value["meta"]["actions"]
            .to_string()
            .contains("evil.invalid")
    );
}
#[test]
fn partial_data_is_preserved_and_malformed_handles_are_not_executed() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    let mut value = error("PARTIAL_RESULT", 7, None);
    value["data"] = json!([{ "id":"1" }]);
    value["meta"]["pagination"] = json!({"scopeComplete":false});
    value["meta"]["recovery"] = json!({"intentId":"; invoke another operation"});
    attach(&rt, &mut value);
    assert_eq!(value["data"], json!([{ "id":"1" }]));
    assert_eq!(value["meta"]["pagination"]["scopeComplete"], false);
    assert_eq!(value["meta"]["actions"][0]["kind"], "manual");
}

#[test]
fn storage_failure_stops_without_followup_commands_and_preserves_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let rt = runtime(dir.path());
    configured(&rt, "me");
    for code in [
        "CREDENTIAL_STORE_UNAVAILABLE",
        "LOCAL_STATE_UNAVAILABLE",
        "CREDENTIAL_DECODE_FAILED",
    ] {
        let mut value = error(code, 1, Some("me"));
        value["data"] = json!({"items":[{"id":42}]});
        value["error"]["details"] =
            json!({"area":"credentials","stage":"atomic_replace","osCode":1});
        value["meta"]["recovery"] = json!({"runId":"R".repeat(43),"intentId":"I".repeat(43)});
        attach(&rt, &mut value);
        assert_eq!(value["error"]["code"], code);
        assert_eq!(value["error"]["retryable"], false);
        assert_eq!(value["error"]["details"]["stage"], "atomic_replace");
        assert_eq!(value["data"]["items"][0]["id"], 42);
        assert_eq!(value["meta"]["recovery"]["runId"], "R".repeat(43));
        let actions = value["meta"]["actions"].as_array().unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["id"], "stop-storage");
        assert_eq!(actions[0]["requiresInteraction"], false);
        assert!(actions[0].get("argv").is_none());
        assert!(actions[0].get("url").is_none());
    }
}
