use super::*;

#[tokio::test]
async fn task_compare_previews_only_supported_api_fields_without_online_access() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let start = iam.count();
    let sealed = h.sealed();
    let (value, exit) = h
        .run_with(
            &sealed,
            &[
                "tasks",
                "plan",
                "order-config.compare",
                "--profile",
                "supply",
                "--version",
                "1.0.0",
                "--params",
                r#"{"itemCode":"101010009","shopCodes":["A","B"]}"#,
            ],
        )
        .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["data"]["result"]["operationVersion"], "1.0.0");
    assert_eq!(value["data"]["result"]["permissionVerified"], false);
    assert_eq!(value["data"]["result"]["recordWrites"], 0);
    assert_eq!(iam.count(), start);
    assert!(!h.root.path().join("workpacks").exists());
}

#[tokio::test]
async fn task_versions_consent_and_output_support_are_distinct_local_failures() {
    let iam = Iam::start(
        SYSTEM,
        ENV,
        vec![
            operation(BATCH, "1.0.0", true),
            operation(BATCH, "2.0.0", false),
        ],
    );
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let start = iam.count();
    let (shown, exit) = h
        .run_with(
            &h.sealed(),
            &[
                "tasks",
                "show",
                "order-config.compare",
                "--profile",
                "supply",
            ],
        )
        .await;
    assert_eq!(exit, 0);
    assert_eq!(shown["data"]["availability"]["reason"], "VERSION_REQUIRED");
    let (ambiguous, exit) = h
        .run_with(
            &h.sealed(),
            &[
                "tasks",
                "plan",
                "order-config.compare",
                "--profile",
                "supply",
                "--params",
                r#"{"itemCode":"1","shopCodes":["A","B"]}"#,
            ],
        )
        .await;
    assert_failure(&ambiguous, exit, 2, "VERSION_REQUIRED");
    assert_eq!(ambiguous["meta"]["taskSchemaVersion"], 1);
    assert_eq!(ambiguous["meta"]["effect"], "read");
    assert_eq!(ambiguous["meta"]["availability"]["state"], "blocked");
    assert_eq!(ambiguous["meta"]["actions"][0]["actor"], "employee");
    assert_eq!(ambiguous["meta"]["actions"][0]["kind"], "manual");
    let (shown, exit) = h
        .run_with(
            &h.sealed(),
            &[
                "tasks",
                "show",
                "order-config.compare",
                "--profile",
                "supply",
                "--version",
                "2.0.0",
            ],
        )
        .await;
    assert_eq!(exit, 0);
    assert_eq!(shown["data"]["availability"]["reason"], "SCOPE_DENIED");
    let (plan, exit) = h
        .run_with(
            &h.sealed(),
            &[
                "tasks",
                "plan",
                "order-config.compare",
                "--profile",
                "supply",
                "--version",
                "1.0.0",
                "--params",
                r#"{"itemCode":"1","shopCodes":["A","A"]}"#,
            ],
        )
        .await;
    assert_failure(&plan, exit, 2, "INVALID_ARGUMENT");
    assert_eq!(iam.count(), start);
}

#[tokio::test]
async fn write_task_preview_requires_explicit_identity_and_current_confirmation_channel() {
    let iam = hrmp();
    iam.with(|f| f.operations[0]["outputSchema"]=json!({"type":"object","properties":{"intentId":{"type":"string"},"state":{"type":"string"},"sideEffect":{"type":"string"}}}));
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hr", ENV, "hrmp").await;
    let start = iam.count();
    let (value, exit) = h
        .run_with(
            &h.sealed(),
            &[
                "tasks",
                "plan",
                "like.send",
                "--profile",
                "hr",
                "--params",
                LIKE_PARAMS,
            ],
        )
        .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["data"]["result"]["effect"], "write");
    assert_eq!(iam.count(), start);
    let (value, exit) = h
        .run_with(
            &h.sealed(),
            &["tasks", "plan", "like.send", "--params", LIKE_PARAMS],
        )
        .await;
    assert_failure(&value, exit, 2, "PROFILE_REQUIRED");
    assert_eq!(iam.count(), start);
    let path = h.root.path().join("governed/hr.catalog.json");
    let mut cache: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    cache["operations"][0]["confirmation"]["channel"] = json!("iam-browser");
    std::fs::write(&path, cache.to_string()).unwrap();
    let (value, exit) = h
        .run_with(
            &h.sealed(),
            &["tasks", "show", "like.send", "--profile", "hr"],
        )
        .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["data"]["availability"]["state"], "unsupported");
    assert_eq!(iam.count(), start);
}
