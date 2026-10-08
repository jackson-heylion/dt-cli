use super::*;
const PARAMS: &str = r#"{"itemCode":"101010009","shopCodes":["2021003","missing"]}"#;
async fn compare(h: &Harness) -> (Value, u8) {
    h.run(&[
        "tasks",
        "run",
        "order-config.compare",
        "--profile",
        "supply",
        "--version",
        "1.0.0",
        "--params",
        PARAMS,
    ])
    .await
}
#[tokio::test]
async fn incomplete_failed_or_permission_denied_jobs_never_produce_comparisons() {
    for (mode, exit, code) in [
        ("failed", 7, "PARTIAL_RESULT"),
        ("expired", 7, "PARTIAL_RESULT"),
        ("denied", 4, "DATA_SCOPE_DENIED"),
    ] {
        let iam = supply();
        let h = Harness::new(&[(ENV, &iam)]);
        h.login("supply", ENV, SYSTEM).await;
        iam.with(|f| {
            f.auto_complete_jobs = true;
            if mode == "failed" {
                f.job_initial_state = Some("failed");
            } else {
                f.overrides.insert(
                    "jobs.result",
                    failure(if mode == "denied" { 403 } else { 409 }, code),
                );
            }
        });
        let (v, e) = compare(&h).await;
        assert_failure(&v, e, exit, code);
        assert_eq!(v["meta"]["complete"], false);
        assert!(v["data"]["result"]["items"].is_null());
        if mode == "denied" {
            assert_eq!(v["meta"]["actions"][0]["actor"], "admin");
            assert_eq!(v["meta"]["actions"][0]["kind"], "manual");
        } else {
            assert_eq!(v["meta"]["actions"][0]["id"], "inspect-run");
        }
        let id = v["data"]["runId"].as_str().unwrap();
        let start = iam.count();
        let (_, e) = h.run(&["tasks", "status", id]).await;
        assert_eq!(e, 0);
        assert!(iam.since(start).iter().all(|r| r.method == "GET"));
        iam.with(|f| {
            assert_eq!(
                f.seen
                    .iter()
                    .filter(|r| r.method == "POST" && r.path == "/cli-api/v1/jobs")
                    .count(),
                1
            )
        });
    }
}
#[tokio::test]
async fn recovery_rejects_changed_local_contract_before_any_http() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    iam.with(|f| f.auto_complete_jobs = true);
    let (v, e) = compare(&h).await;
    assert_eq!(e, 0, "{v}");
    let id = v["data"]["runId"].as_str().unwrap();
    let path = h.file("supply.catalog.json");
    let mut cache: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for op in cache["operations"].as_array_mut().unwrap() {
        if op["operationId"] == BATCH {
            op["contractDigest"] = json!("0".repeat(64));
        }
    }
    std::fs::write(path, cache.to_string()).unwrap();
    let start = iam.count();
    let (v, e) = h.run(&["tasks", "status", id]).await;
    assert_failure(&v, e, 6, "CONTRACT_CHANGED");
    assert_eq!(iam.count(), start);
    assert_eq!(v["meta"]["effect"], "read");
    assert_eq!(v["meta"]["availability"]["state"], "blocked");
    assert_eq!(
        v["meta"]["actions"][0]["argv"],
        json!(["dt-cli", "catalog", "sync", "--profile", "supply"])
    );
    assert_eq!(v["meta"]["recovery"]["runId"], id);
}

#[tokio::test]
async fn outdated_permission_cache_on_original_run_requires_sync_before_status() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    iam.with(|f| f.auto_complete_jobs = true);
    let (v, e) = compare(&h).await;
    assert_eq!(e, 0, "{v}");
    let id = v["data"]["runId"].as_str().unwrap();
    let path = h.file("supply.catalog.json");
    let mut cache: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    for op in cache["operations"].as_array_mut().unwrap() {
        if op["operationId"] == BATCH {
            op["consented"] = json!(false);
        }
    }
    std::fs::write(path, cache.to_string()).unwrap();
    let start = iam.count();
    let (v, e) = h.run(&["tasks", "status", id]).await;
    assert_failure(&v, e, 4, "SCOPE_DENIED");
    assert_eq!(iam.count(), start);
    assert_eq!(v["meta"]["actions"][0]["id"], "sync-contract");
    assert_eq!(v["meta"]["actions"][0]["actor"], "employee");
    assert_eq!(
        v["meta"]["actions"][0]["argv"],
        json!(["dt-cli", "catalog", "sync", "--profile", "supply"])
    );
    assert_eq!(v["meta"]["recovery"]["runId"], id);
}

#[tokio::test]
async fn expired_and_revoked_authorizations_return_bound_employee_recovery() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let path = h.file("supply.profile.json");
    let original = std::fs::read(&path).unwrap();
    let mut p: Value = serde_json::from_slice(&original).unwrap();
    p["authorizationExpiresAt"] = json!("2000-01-01T00:00:00Z");
    std::fs::write(&path, p.to_string()).unwrap();
    let start = iam.count();
    let args = [
        "api",
        "call",
        READ,
        "--profile",
        "supply",
        "--params",
        READ_PARAMS,
    ];
    let (v, e) = h.run(&args).await;
    assert_failure(&v, e, 4, "AUTHORIZATION_EXPIRED");
    assert_eq!(iam.count(), start);
    assert_eq!(v["meta"]["actions"][0]["id"], "reauthorize");
    std::fs::write(path, original).unwrap();
    iam.with(|f| {
        f.overrides.insert(
            "exchange",
            oauth_error(401, "invalid_grant", "AUTHORIZATION_REVOKED"),
        );
    });
    let (v, e) = h.run(&args).await;
    assert_failure(&v, e, 4, "AUTHORIZATION_REVOKED");
    assert_eq!(v["meta"]["actions"][0]["id"], "reauthorize");
    assert_eq!(
        iam.paths_since(start),
        vec!["POST /cli-api/oauth2/exchange"]
    );
}
