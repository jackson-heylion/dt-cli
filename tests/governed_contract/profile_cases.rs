use super::*;

#[tokio::test]
async fn profiles_are_isolated_by_system_environment_and_authorization() {
    let a = supply();
    let b = Iam::start(
        "hrmp",
        "fixture-b",
        vec![operation("hrmp.like.rank.monthly.read", "1.0.0", true)],
    );
    let h = Harness::new(&[(ENV, &a), ("fixture-b", &b)]);
    h.login("a", ENV, SYSTEM).await;
    h.login("b", "fixture-b", "hrmp").await;
    let keys: Vec<_> = h
        .store
        .keys()
        .into_iter()
        .filter(|k| k.starts_with("governed/"))
        .collect();
    assert_eq!(keys.len(), 2, "{keys:?}");
    let ka = keys
        .iter()
        .find(|k| k.contains("/fixture/supply-chain-server/"))
        .unwrap();
    let kb = keys
        .iter()
        .find(|k| k.contains("/fixture-b/hrmp/"))
        .unwrap();
    assert_ne!(ka.split('/').nth(1), kb.split('/').nth(1), "issuer differs");
    let (found, _) = h.run(&["discover", "--profile", "b"]).await;
    let ids: Vec<_> = found["data"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op["operationId"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(ids, vec!["hrmp.like.rank.monthly.read"]);
    let (a_before, b_before) = (a.count(), b.count());
    let (value, exit) = h
        .run(&[
            "api",
            "call",
            READ,
            "--profile",
            "b",
            "--params",
            READ_PARAMS,
        ])
        .await;
    assert_failure(&value, exit, 2, "PROFILE_SELECTION_MISMATCH");
    std::fs::copy(h.file("a.catalog.json"), h.file("b.catalog.json")).unwrap();
    let (value, exit) = h.run(&["discover", "--profile", "b"]).await;
    assert_failure(&value, exit, 2, "CATALOG_INVALID");
    let (value, exit) = h
        .run(&[
            "api",
            "call",
            "hrmp.like.rank.monthly.read",
            "--profile",
            "b",
            "--params",
            READ_PARAMS,
        ])
        .await;
    assert_failure(&value, exit, 2, "CATALOG_INVALID");
    let mut forged: Value =
        serde_json::from_str(&std::fs::read_to_string(h.file("b.profile.json")).unwrap()).unwrap();
    forged["credentialKey"] = json!(ka);
    std::fs::write(h.file("b.profile.json"), forged.to_string()).unwrap();
    let (value, exit) = h.run(&["auth", "status", "--profile", "b"]).await;
    assert_failure(&value, exit, 2, "PROFILE_INVALID");
    assert_eq!((a.count(), b.count()), (a_before, b_before));
}

// 旧个人流程 profile/命令原样可用；两类 profile 不能混用。
#[tokio::test]
async fn personal_workflow_profiles_and_commands_are_unchanged() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    let now = chrono::Utc::now();
    let old = Credentials {
        access_token: OLD_ACCESS.into(),
        refresh_token: OLD_REFRESH.into(),
        version: 1,
        access_expires_at: Some((now + chrono::Duration::hours(1)).to_rfc3339()),
        authorization_expires_at: Some((now + chrono::Duration::days(6)).to_rfc3339()),
        generation: 0,
        refresh_state: "ready".into(),
        refresh_request_id: None,
    };
    h.store.write(OLD_KEY, &old).unwrap();
    let profile = json!({"environment":ENV,"subjectId":"1","authorizationId":"auth-fixture",
        "authorizationExpiresAt":(now + chrono::Duration::days(6)).to_rfc3339(),
        "accessExpiresAt":(now + chrono::Duration::hours(1)).to_rfc3339(),
        "checkedAt":"2026-09-22T00:00:00Z","credentialKey":OLD_KEY});
    std::fs::write(h.root.path().join("old.json"), profile.to_string()).unwrap();
    let old_record = serde_json::to_string(&h.store.get(OLD_KEY)).unwrap();
    h.login("gov", ENV, SYSTEM).await;
    let (value, exit) = h
        .run(&[
            "api",
            "call",
            READ,
            "--profile",
            "gov",
            "--params",
            READ_PARAMS,
        ])
        .await;
    assert_eq!(exit, 0, "{value}");
    let before = iam.count();
    let (me, exit) = h.run(&["whoami", "--profile", "old"]).await;
    assert_eq!(exit, 0, "{me}");
    assert_eq!(me["data"]["subject"]["id"], "1");
    let last = iam.seen().last().cloned().unwrap();
    assert_eq!(last.path, "/cli/v1/me");
    assert_eq!(
        last.authorization.as_deref(),
        Some(format!("Bearer {OLD_ACCESS}").as_str())
    );
    let (status, exit) = h.run(&["auth", "status", "--profile", "old"]).await;
    assert_eq!(exit, 0, "{status}");
    assert!(status["data"].get("provider").is_none(), "{status}");
    assert_eq!(status["data"]["lastCheckedAt"], "2026-09-22T00:00:00Z");
    let (builtin, exit) = h.run(&["discover"]).await;
    assert_eq!(exit, 0, "{builtin}");
    assert!(builtin["data"]["catalogVersion"].is_string());
    let after_old = iam.count();
    assert_eq!(after_old, before + 1);
    for args in [
        vec!["apps", "list", "--profile", "gov"],
        vec!["workflow", "list", "--kind", "todo", "--profile", "gov"],
        vec!["catalog", "sync", "--profile", "old"],
        vec!["jobs", "status", &"J".repeat(43), "--profile", "old"],
        vec![
            "auth",
            "login",
            "--profile",
            "old",
            "--environment",
            ENV,
            "--system",
            SYSTEM,
        ],
    ] {
        let (value, exit) = h.run(&args).await;
        assert_failure(&value, exit, 2, "PROFILE_PROVIDER_MISMATCH");
    }
    assert_eq!(iam.count(), after_old, "mismatches send nothing");
    assert_eq!(
        iam.with(|f| f.authorize.len()),
        1,
        "no second browser consent"
    );
    assert_eq!(
        serde_json::to_string(&h.store.get(OLD_KEY)).unwrap(),
        old_record
    );
    for seen in iam.seen() {
        if seen.path == "/cli-api/oauth2/exchange" {
            assert!(seen.form()["subject_token"].starts_with("dtcli_g_a_"));
        }
        if seen.path.starts_with("/cli-api/") {
            assert!(
                !seen
                    .authorization
                    .as_deref()
                    .unwrap_or_default()
                    .contains("dtcli_a_"),
                "old token sent to a governed route"
            );
        }
    }
}

// A4（CLI 侧）：过期即单次轮换；显式 401 最多一次恢复；刷新重放撤销授权；未知结果不重发。

#[tokio::test]
async fn governed_setup_uses_existing_browser_consent_and_preserves_provider_binding() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    let (v, e) = h
        .run(&[
            "setup",
            "--profile",
            "supply",
            "--environment",
            ENV,
            "--system",
            SYSTEM,
        ])
        .await;
    assert_eq!(e, 0, "{v}");
    let p: Value =
        serde_json::from_slice(&std::fs::read(h.file("supply.profile.json")).unwrap()).unwrap();
    assert_eq!(p["provider"], "governed");
    assert_eq!(p["systemId"], SYSTEM);
    assert_eq!(h.store.keys().len(), 1);
    assert!(!h.root.path().join("supply.json").exists());
    iam.with(|f| assert_eq!(f.authorize.len(), 1));
}
