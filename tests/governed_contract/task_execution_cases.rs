use super::*;
use std::{fs, path::PathBuf};

fn task_hrmp() -> Iam {
    let iam = hrmp();
    iam.with(|f|f.operations[0]["outputSchema"]=json!({"type":"object","properties":{"intentId":{"type":"string"},"state":{"type":"string"},"sideEffect":{"type":"string"}}}));
    iam
}
fn params_file(h: &Harness) -> PathBuf {
    let path = h.root.path().join("business-parameters.json");
    fs::write(&path, LIKE_PARAMS).unwrap();
    path
}
fn like_args(path: &str) -> Vec<&str> {
    vec![
        "tasks",
        "run",
        "like.send",
        "--profile",
        "hr",
        "--version",
        "1.0.0",
        "--params-file",
        path,
        "--execute",
    ]
}
fn receipt(h: &Harness, id: &str) -> PathBuf {
    h.root.path().join("workpacks").join(format!("{id}.json"))
}

#[tokio::test]
async fn task_like_sends_once_and_resume_status_concurrency_never_dispatch() {
    let iam = task_hrmp();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hr", ENV, "hrmp").await;
    let file = params_file(&h);
    let args = like_args(file.to_str().unwrap());
    let (value, exit) = h.run(&args).await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["data"]["result"]["state"], "succeeded");
    assert_eq!(value["data"]["result"]["sideEffect"], "confirmed");
    assert_eq!(value["meta"]["complete"], true);
    assert_eq!(value["meta"]["availability"]["state"], "ready");
    let id = value["data"]["runId"].as_str().unwrap();
    let bytes = fs::read(receipt(&h, id)).unwrap();
    let text = String::from_utf8(bytes).unwrap();
    for private in [
        "beLikedPersonId",
        "周末主动支援",
        file.to_str().unwrap(),
        "access_token",
        "refresh_token",
    ] {
        assert!(!text.contains(private), "{text}");
    }
    assert_eq!(
        fs::read_dir(h.root.path().join("workpacks"))
            .unwrap()
            .filter(|e| e
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|e| e == "json"))
            .count(),
        1
    );
    let mut resume = args.clone();
    resume.extend(["--run-id", id]);
    let start = iam.count();
    let (a, b) = tokio::join!(h.run(&resume), h.run(&resume));
    assert_eq!(a.1, 0, "{}", a.0);
    assert_eq!(b.1, 0, "{}", b.0);
    let (status, exit) = h.run(&["tasks", "status", id]).await;
    assert_eq!(exit, 0, "{status}");
    assert_eq!(status["data"]["result"]["state"], "succeeded");
    assert_eq!(status["meta"]["availability"]["state"], "unknown");
    assert_eq!(status["meta"]["permissionVerified"], false);
    assert!(
        iam.paths_since(start)
            .iter()
            .all(|p| !p.ends_with("/invoke")
                && !p.ends_with("/authorize")
                && !p.ends_with("/intents"))
    );
    iam.with(|f| assert_eq!(f.writes, 1));
    let mut changed: Value = serde_json::from_str(LIKE_PARAMS).unwrap();
    changed["specificDeeds"] = json!("changed");
    fs::write(&file, changed.to_string()).unwrap();
    let start = iam.count();
    let (value, exit) = h.run(&resume).await;
    assert_failure(&value, exit, 6, "RUN_ARGUMENTS_CHANGED");
    assert_eq!(iam.count(), start);
}

#[tokio::test]
async fn unknown_like_retains_original_id_and_tampering_or_rebinding_stops_before_network() {
    let iam = task_hrmp();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hr", ENV, "hrmp").await;
    iam.with(|f| f.dispatch = Some("drop"));
    let file = params_file(&h);
    let args = like_args(file.to_str().unwrap());
    let (value, exit) = h.run(&args).await;
    assert_failure(&value, exit, 6, "SIDE_EFFECT_UNKNOWN");
    let id = value["data"]["runId"].as_str().unwrap();
    let intent = value["meta"]["recovery"]["intentId"].as_str().unwrap();
    let (status, exit) = h.run(&["tasks", "status", id]).await;
    assert_eq!(exit, 0, "{status}");
    assert_eq!(status["data"]["result"]["intentId"], intent);
    assert_eq!(status["data"]["result"]["state"], "unknown");
    assert_eq!(status["meta"]["complete"], false);
    let mut resume = args.clone();
    resume.extend(["--run-id", id]);
    let (_, exit) = h.run(&resume).await;
    assert_eq!(exit, 0);
    iam.with(|f| assert_eq!(f.writes, 1));
    let path = receipt(&h, id);
    let original = fs::read(&path).unwrap();
    let mut sealed: Value = serde_json::from_slice(&original).unwrap();
    sealed["record"]["remoteId"] = json!("R".repeat(43));
    fs::write(&path, sealed.to_string()).unwrap();
    let start = iam.count();
    let (v, e) = h.run(&["tasks", "status", id]).await;
    assert_failure(&v, e, 1, "LOCAL_STATE_UNAVAILABLE");
    assert_eq!(iam.count(), start);
    fs::write(&path, original).unwrap();
    let profile = h.file("hr.profile.json");
    let mut p: Value = serde_json::from_slice(&fs::read(&profile).unwrap()).unwrap();
    p["authorizationExpiresAt"] = json!("2038-01-01T00:00:00Z");
    fs::write(profile, p.to_string()).unwrap();
    let (v, e) = h.run(&["tasks", "status", id]).await;
    assert_failure(&v, e, 4, "RUN_BINDING_CHANGED");
    assert_eq!(iam.count(), start);
}

#[tokio::test]
async fn compare_submits_once_with_api_fields_and_recovers_exact_original_selection() {
    let iam = supply();
    iam.with(|f| f.auto_complete_jobs = true);
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let params = r#"{"itemCode":"101010009","shopCodes":["2021003","missing"]}"#;
    let args = [
        "tasks",
        "run",
        "order-config.compare",
        "--profile",
        "supply",
        "--version",
        "1.0.0",
        "--params",
        params,
    ];
    let (value, exit) = h.run(&args).await;
    assert_eq!(exit, 0, "{value}");
    let id = value["data"]["runId"].as_str().unwrap();
    assert_eq!(
        value["data"]["result"]["items"][0]["sources"][1]["reason"],
        "no-row"
    );
    assert_eq!(
        value["meta"]["sources"][0]["contractDigest"],
        operation(BATCH, "1.0.0", true)["contractDigest"]
    );
    let start = iam.count();
    let mut resume = args.to_vec();
    resume.extend(["--run-id", id]);
    let (v, e) = h.run(&resume).await;
    assert_eq!(e, 0, "{v}");
    assert_eq!(
        v["data"]["result"]["items"],
        value["data"]["result"]["items"]
    );
    assert!(
        !iam.paths_since(start)
            .iter()
            .any(|p| p == "/cli-api/v1/jobs")
    );
    let (v, e) = h.run(&["tasks", "status", id]).await;
    assert_eq!(e, 0, "{v}");
    assert_eq!(
        v["data"]["result"]["resultRecovery"]["requiresOriginalParameters"],
        true
    );
    iam.with(|f| {
        let submits: Vec<_> = f
            .seen
            .iter()
            .filter(|r| r.method == "POST" && r.path == "/cli-api/v1/jobs")
            .collect();
        assert_eq!(submits.len(), 1);
        let body: Value = serde_json::from_str(&submits[0].body).unwrap();
        assert_eq!(body["arguments"], json!({"itemCode":"101010009"}));
    });
}

#[tokio::test]
async fn failed_submit_is_unlocated_and_never_resubmitted() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    iam.with(|f| {
        f.overrides
            .insert("jobs.submit", failure(503, "DEPENDENCY_UNAVAILABLE"));
    });
    let args = [
        "tasks",
        "run",
        "order-config.compare",
        "--profile",
        "supply",
        "--params",
        r#"{"itemCode":"1","shopCodes":["A","B"]}"#,
    ];
    let (v, e) = h.run(&args).await;
    assert_failure(&v, e, 5, "DEPENDENCY_UNAVAILABLE");
    let id = v["data"]["runId"].as_str().unwrap();
    let start = iam.count();
    let (v, e) = h.run(&["tasks", "status", id]).await;
    assert_failure(&v, e, 6, "RUN_UNLOCATED");
    let mut resume = args.to_vec();
    resume.extend(["--run-id", id]);
    let (v, e) = h.run(&resume).await;
    assert_failure(&v, e, 6, "RUN_UNLOCATED");
    assert_eq!(iam.count(), start);
}

#[cfg(unix)]
#[tokio::test]
async fn insecure_or_symlinked_record_directory_blocks_prepare() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let iam = task_hrmp();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hr", ENV, "hrmp").await;
    let file = params_file(&h);
    let args = like_args(file.to_str().unwrap());
    let start = iam.count();
    let dir = h.root.path().join("workpacks");
    fs::create_dir(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
    let (v, e) = h.run(&args).await;
    assert_failure(&v, e, 1, "LOCAL_STATE_UNAVAILABLE");
    assert_eq!(iam.count(), start);
    fs::remove_dir(&dir).unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    symlink(elsewhere.path(), &dir).unwrap();
    let (v, e) = h.run(&args).await;
    assert_failure(&v, e, 1, "LOCAL_STATE_UNAVAILABLE");
    assert_eq!(iam.count(), start);
}

#[tokio::test]
async fn lost_prepare_or_authorize_replies_only_locate_original_intent_without_dispatch() {
    for phase in ["prepare", "authorize"] {
        let iam = task_hrmp();
        let h = Harness::new(&[(ENV, &iam)]);
        h.login("hr", ENV, "hrmp").await;
        iam.with(|f| {
            if phase == "prepare" {
                f.drop_prepare_reply_once = true
            } else {
                f.drop_authorize_reply_once = true
            }
        });
        let file = params_file(&h);
        let args = like_args(file.to_str().unwrap());
        let (v, e) = h.run(&args).await;
        assert_failure(&v, e, 5, "NETWORK_ERROR");
        assert_eq!(v["error"]["retryable"], false);
        let id = v["data"]["runId"].as_str().unwrap();
        let start = iam.count();
        let mut resume = args.clone();
        resume.extend(["--run-id", id]);
        let (v, e) = h.run(&resume).await;
        assert_eq!(e, 0, "{v}");
        assert_eq!(v["data"]["result"]["sideEffect"], "none");
        assert_eq!(v["meta"]["complete"], false);
        assert!(
            !iam.paths_since(start)
                .iter()
                .any(|p| p.ends_with("/invoke") || p.ends_with("/authorize"))
        );
        iam.with(|f| {
            assert_eq!(f.writes, 0);
            assert_eq!(f.intents.len(), 1);
            assert_eq!(f.intent_keys.len(), 1);
        });
    }
}

#[tokio::test]
async fn changing_parameter_file_after_prepare_does_not_change_authorized_payload() {
    let iam = task_hrmp();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hr", ENV, "hrmp").await;
    let file = params_file(&h);
    iam.with(|f| f.params_mutation = Some(file.clone()));
    let (v, e) = h.run(&like_args(file.to_str().unwrap())).await;
    assert_eq!(e, 0, "{v}");
    assert_ne!(fs::read_to_string(&file).unwrap(), LIKE_PARAMS);
    let original: Value = serde_json::from_str(LIKE_PARAMS).unwrap();
    iam.with(|f| {
        assert_eq!(f.writes, 1);
        let intent = f.intents.values().next().unwrap();
        assert_eq!(intent["argumentsDigest"], digest(&original.to_string()));
    });
}

#[cfg(unix)]
#[tokio::test]
async fn record_failure_after_prepare_or_authorize_stops_the_next_network_action() {
    use std::os::unix::fs::PermissionsExt;
    for phase in ["intents.prepare", "intents.authorize"] {
        let iam = task_hrmp();
        let h = Harness::new(&[(ENV, &iam)]);
        h.login("hr", ENV, "hrmp").await;
        iam.with(|f| f.record_fault = Some((phase, h.root.path().join("workpacks"))));
        let file = params_file(&h);
        let (v, e) = h.run(&like_args(file.to_str().unwrap())).await;
        assert_failure(&v, e, 1, "LOCAL_STATE_UNAVAILABLE");
        assert!(v["meta"]["recovery"]["intentId"].is_string());
        let id = v["data"]["runId"].as_str().unwrap();
        fs::set_permissions(receipt(&h, id), fs::Permissions::from_mode(0o600)).unwrap();
        let (v, e) = h.run(&["tasks", "status", id]).await;
        if phase == "intents.prepare" {
            assert_failure(&v, e, 6, "RUN_UNLOCATED");
        } else {
            assert_eq!(e, 0, "{v}");
            assert_eq!(v["data"]["result"]["state"], "approved");
        }
        iam.with(|f| {
            assert_eq!(f.writes, 0);
            assert!(!f.seen.iter().any(|r| r.path.ends_with("/invoke")));
            if phase == "intents.prepare" {
                assert!(!f.seen.iter().any(|r| r.path.ends_with("/authorize")));
            }
        });
    }
}

#[tokio::test]
async fn already_approved_bounded_preparation_still_verifies_digests_and_dispatches_once() {
    let iam = task_hrmp();
    iam.with(|f| {
        f.auto_approve_prepared = true;
        f.operations[0]["confirmation"]["boundedPreauthorization"] = json!(true);
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hr", ENV, "hrmp").await;
    let file = params_file(&h);
    let (v, e) = h.run(&like_args(file.to_str().unwrap())).await;
    assert_eq!(e, 0, "{v}");
    assert_eq!(v["meta"]["sideEffect"], "confirmed");
    assert!(v["meta"]["traceId"].is_string());
    assert!(v["data"]["result"]["_meta"].is_null());
    iam.with(|f| {
        assert_eq!(f.writes, 1);
        assert_eq!(
            f.seen
                .iter()
                .filter(|r| r.path.ends_with("/authorize"))
                .count(),
            1
        );
    });
}

#[tokio::test]
async fn expired_preparation_is_saved_but_never_authorized_or_dispatched() {
    let iam = task_hrmp();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hr", ENV, "hrmp").await;
    let id = "E".repeat(43);
    let args: Value = serde_json::from_str(LIKE_PARAMS).unwrap();
    iam.with(|f|{f.overrides.insert("intents.prepare",reply(201,ok(LIKE,json!({"intentId":id,"state":"prepared","sideEffect":"none","operationId":LIKE,"operationVersion":"1.0.0","contractDigest":f.operations[0]["contractDigest"],"argumentsDigest":digest(&args.to_string()),"expiresAt":"2000-01-01T00:00:00Z","confirmationUrl":null}))));});
    let start = iam.count();
    let file = params_file(&h);
    let (v, e) = h.run(&like_args(file.to_str().unwrap())).await;
    assert_failure(&v, e, 6, "INTENT_EXPIRED");
    assert_eq!(v["meta"]["recovery"]["intentId"], id);
    assert!(
        !iam.paths_since(start)
            .iter()
            .any(|p| p.ends_with("/authorize") || p.ends_with("/invoke"))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn confirmed_dispatch_is_preserved_when_terminal_record_save_fails() {
    use std::os::unix::fs::PermissionsExt;
    let iam = task_hrmp();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hr", ENV, "hrmp").await;
    iam.with(|f| f.record_fault = Some(("systems.invoke", h.root.path().join("workpacks"))));
    let file = params_file(&h);
    let (value, exit) = h.run(&like_args(file.to_str().unwrap())).await;
    assert_failure(&value, exit, 1, "LOCAL_STATE_UNAVAILABLE");
    assert_eq!(value["data"]["result"]["state"], "succeeded");
    assert_eq!(value["data"]["result"]["sideEffect"], "confirmed");
    assert_eq!(value["meta"]["sideEffect"], "confirmed");
    assert_eq!(value["meta"]["recordPersistence"], "unconfirmed");
    assert_eq!(value["meta"]["actions"][0]["actor"], "operator");
    assert_eq!(value["error"]["retryable"], false);
    assert_eq!(value["meta"]["complete"], false);
    let id = value["data"]["runId"].as_str().unwrap();
    fs::set_permissions(receipt(&h, id), fs::Permissions::from_mode(0o600)).unwrap();
    let start = iam.count();
    let (status, exit) = h.run(&["tasks", "status", id]).await;
    assert_eq!(exit, 0, "{status}");
    assert_eq!(status["data"]["result"]["sideEffect"], "confirmed");
    assert!(iam.since(start).iter().all(|r| r.method == "GET"));
    iam.with(|f| assert_eq!(f.writes, 1));
}
