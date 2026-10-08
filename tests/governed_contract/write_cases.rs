use super::*;

#[tokio::test]
async fn writes_prepare_an_intent_and_only_an_approved_intent_is_sent() {
    let iam = hrmp();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hrmp-stg", ENV, "hrmp").await;
    let before = iam.count();
    let (value, exit) = h
        .run(&[
            "api",
            "call",
            LIKE,
            "--profile",
            "hrmp-stg",
            "--params",
            LIKE_PARAMS,
        ])
        .await;
    assert_failure(&value, exit, 4, "SCOPE_DENIED");
    for (params, key) in [
        (r#"{"beLikedPersonId":1003}"#, "key-valid-000000001"),
        (LIKE_PARAMS, "short"),
    ] {
        let (value, exit) = h
            .run_with(
                &h.sealed(),
                &[
                    "intents",
                    "prepare",
                    LIKE,
                    "--profile",
                    "hrmp-stg",
                    "--params",
                    params,
                    "--idempotency-key",
                    key,
                ],
            )
            .await;
        assert_failure(&value, exit, 2, "INVALID_ARGUMENT");
    }
    let (preview, exit) = h
        .run_with(
            &h.sealed(),
            &[
                "intents",
                "prepare",
                LIKE,
                "--profile",
                "hrmp-stg",
                "--params",
                LIKE_PARAMS,
                "--dry-run",
            ],
        )
        .await;
    assert_eq!(exit, 0, "{preview}");
    assert_eq!(preview["data"]["httpRequests"], 0);
    assert_eq!(
        iam.count(),
        before,
        "refused or previewed with zero requests"
    );

    let (prepared, exit) = h
        .run(&[
            "intents",
            "prepare",
            LIKE,
            "--profile",
            "hrmp-stg",
            "--params",
            LIKE_PARAMS,
            "--idempotency-key",
            "key-like-000000001",
        ])
        .await;
    assert_eq!(exit, 0, "{prepared}");
    let id = prepared["data"]["intentId"].as_str().unwrap().to_owned();
    assert_eq!(prepared["data"]["state"], "prepared");
    assert_eq!(prepared["data"]["sideEffect"], "none");
    assert!(prepared["data"]["confirmationUrl"].is_null());
    let sent = iam.since(before);
    let exchange = sent
        .iter()
        .find(|s| s.path == "/cli-api/oauth2/exchange")
        .unwrap();
    assert_eq!(
        exchange.form()["operation_contracts"],
        serde_json::to_string(&[key(&iam.with(|f| f.operations[0].clone()))]).unwrap()
    );
    let prepare = sent
        .iter()
        .find(|s| s.path == "/cli-api/v1/intents")
        .unwrap();
    assert_eq!(prepare.idempotency.as_deref(), Some("key-like-000000001"));
    let (conflict, exit) = h
        .run(&[
            "intents",
            "prepare",
            LIKE,
            "--profile",
            "hrmp-stg",
            "--params",
            &LIKE_PARAMS.replace("1003", "1009"),
            "--idempotency-key",
            "key-like-000000001",
        ])
        .await;
    assert_failure(&conflict, exit, 6, "IDEMPOTENCY_CONFLICT");

    // Status never sends; an unapproved intent is refused by IAM and not retryable.
    let start = iam.count();
    let (status, exit) = h
        .run(&["intents", "status", &id, "--profile", "hrmp-stg"])
        .await;
    assert_eq!(exit, 0, "{status}");
    assert_eq!(status["data"]["state"], "prepared");
    assert_eq!(
        intents(&iam, start),
        vec![format!("GET /cli-api/v1/intents/{id}")]
    );
    let (early, exit) = h
        .run(&["intents", "invoke", &id, "--profile", "hrmp-stg"])
        .await;
    assert_failure(&early, exit, 6, "APPROVAL_REQUIRED");
    assert_eq!(early["error"]["retryable"], false);
    assert_eq!(early["meta"]["recovery"]["intentId"], id.as_str());
    assert_eq!(iam.with(|f| f.writes), 0);

    approve(&iam, &id);
    let (done, exit) = h
        .run(&["intents", "invoke", &id, "--profile", "hrmp-stg"])
        .await;
    assert_eq!(exit, 0, "{done}");
    assert_eq!(done["data"]["state"], "succeeded");
    assert_eq!(done["data"]["sideEffect"], "confirmed");
    assert_eq!(done["data"]["downstreamRecordId"], Value::Null);
    assert_eq!(done["meta"]["traceId"], "trace-dispatch");
    let (again, exit) = h
        .run(&["intents", "invoke", &id, "--profile", "hrmp-stg"])
        .await;
    assert_eq!(exit, 0, "{again}");
    assert_eq!(
        iam.with(|f| f.writes),
        1,
        "a repeated invoke never sends again"
    );
    let (status, _) = h
        .run(&["intents", "status", &id, "--profile", "hrmp-stg"])
        .await;
    assert_eq!(status["data"]["state"], "succeeded");
    let (cancel, exit) = h
        .run(&["intents", "cancel", &id, "--profile", "hrmp-stg"])
        .await;
    assert_failure(&cancel, exit, 6, "ALREADY_DISPATCHED");
    assert_eq!(cancel["meta"]["recovery"]["state"], "succeeded");
}

// B3/B4 未知结果：202、断连与 Ctrl-C 都只报告 SIDE_EFFECT_UNKNOWN，不重试、不声称未执行。
#[tokio::test]
async fn malformed_and_gateway_write_replies_keep_intent_recovery() {
    let iam = hrmp();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hrmp-stg", ENV, "hrmp").await;
    let (prepared, exit) = h
        .run(&[
            "intents",
            "prepare",
            LIKE,
            "--profile",
            "hrmp-stg",
            "--params",
            LIKE_PARAMS,
        ])
        .await;
    assert_eq!(exit, 0, "{prepared}");
    let id = prepared["data"]["intentId"].as_str().unwrap();
    approve(&iam, id);
    for response in [
        reply(200, json!({"ok":true,"data":{}})),
        reply(
            200,
            json!({"schemaVersion":"1.0","ok":true,"error":{"code":"BROKEN"}}),
        ),
        reply(502, json!({"error":{"code":"UPSTREAM_CONTRACT_MISMATCH"}})),
        (503, vec![], b"gateway unavailable".to_vec()),
        (302, vec![], vec![]),
    ] {
        iam.with(|f| {
            f.overrides.insert("invoke", response);
        });
        let start = iam.count();
        let (value, exit) = h
            .run(&["intents", "invoke", id, "--profile", "hrmp-stg"])
            .await;
        assert_failure(&value, exit, 6, "SIDE_EFFECT_UNKNOWN");
        assert_eq!(value["error"]["retryable"], false);
        assert_eq!(value["meta"]["recovery"]["intentId"], id);
        assert_eq!(value["meta"]["recovery"]["sideEffect"], "unknown");
        assert_eq!(
            iam.paths_since(start)
                .iter()
                .filter(|p| p.ends_with("/invoke"))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn unknown_outcomes_lost_answers_and_interrupts_are_never_reported_as_not_executed() {
    // SIGINT is process-wide: keep it away from concurrently running login tests.
    #[cfg(unix)]
    if std::env::var_os("DT_CLI_WRITE_INTERRUPT_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "unknown_outcomes_lost_answers_and_interrupts_are_never_reported_as_not_executed",
                "--test-threads=1",
            ])
            .env("DT_CLI_WRITE_INTERRUPT_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let iam = hrmp();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hrmp-stg", ENV, "hrmp").await;
    let prepare = |key: &'static str, target: &'static str| {
        let params = LIKE_PARAMS.replace("1003", target);
        let h = &h;
        async move {
            let (value, exit) = h
                .run(&[
                    "intents",
                    "prepare",
                    LIKE,
                    "--profile",
                    "hrmp-stg",
                    "--params",
                    &params,
                    "--idempotency-key",
                    key,
                ])
                .await;
            assert_eq!(exit, 0, "{value}");
            value["data"]["intentId"].as_str().unwrap().to_owned()
        }
    };
    let assert_unknown = |value: &Value, exit: u8, id: &str| {
        assert_failure(value, exit, 6, "SIDE_EFFECT_UNKNOWN");
        assert_eq!(value["error"]["retryable"], false, "{value}");
        assert_eq!(value["meta"]["recovery"]["intentId"], id, "{value}");
        assert_ne!(value["error"]["code"], "LOGIN_CANCELLED");
    };

    let answered = prepare("key-unknown-00000001", "1004").await;
    approve(&iam, &answered);
    iam.with(|f| f.dispatch = Some("unknown"));
    let (value, exit) = h
        .run(&["intents", "invoke", &answered, "--profile", "hrmp-stg"])
        .await;
    assert_unknown(&value, exit, &answered);
    assert_eq!(value["meta"]["state"], "unknown");
    assert_eq!(value["meta"]["sideEffect"], "unknown");
    assert!(!value.to_string().contains("写入原文不外泄"));

    let dropped = prepare("key-dropped-00000001", "1005").await;
    approve(&iam, &dropped);
    iam.with(|f| f.dispatch = Some("drop"));
    let (value, exit) = h
        .run(&["intents", "invoke", &dropped, "--profile", "hrmp-stg"])
        .await;
    assert_unknown(&value, exit, &dropped);

    #[cfg(unix)]
    {
        // Ctrl-C while the dispatch is in flight: the outcome is unknown, never "cancelled".
        let held = prepare("key-held-0000000001", "1006").await;
        approve(&iam, &held);
        iam.with(|f| f.dispatch = Some("hold"));
        let listening = tokio::spawn(async { tokio::signal::ctrl_c().await.ok() });
        let origin = iam.fake.clone();
        let interrupt = tokio::spawn(async move {
            for _ in 0..100 {
                let arrived = origin
                    .lock()
                    .unwrap()
                    .seen
                    .iter()
                    .any(|s| s.path.ends_with("/invoke") && s.body.contains("intentId"));
                if arrived {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
            std::process::Command::new("kill")
                .args(["-INT", &std::process::id().to_string()])
                .status()
                .unwrap();
        });
        let (value, exit) = h
            .run(&["intents", "invoke", &held, "--profile", "hrmp-stg"])
            .await;
        interrupt.await.unwrap();
        listening.abort();
        assert_unknown(&value, exit, &held);
        assert_eq!(iam.with(|f| f.writes), 3);
    }

    // Querying afterwards never sends; the unknown intent stays unknown.
    let start = iam.count();
    let (status, exit) = h
        .run(&["intents", "status", &answered, "--profile", "hrmp-stg"])
        .await;
    assert_eq!(exit, 0, "{status}");
    assert_eq!(status["data"]["state"], "unknown");
    assert!(
        iam.paths_since(start)
            .iter()
            .all(|p| !p.ends_with("/invoke"))
    );
    assert_eq!(iam.with(|f| f.writes), if cfg!(unix) { 3 } else { 2 });

    // A confirmation page anywhere but this IAM is a protocol failure, never shown.
    let foreign = prepare("key-foreign-00000001", "1007").await;
    iam.with(|f| {
        f.intents.get_mut(&foreign).unwrap()["confirmationUrl"] =
            json!("https://phishing.invalid/cli-governed-intent.html");
    });
    let (value, exit) = h
        .run(&["intents", "status", &foreign, "--profile", "hrmp-stg"])
        .await;
    assert_failure(&value, exit, 5, "DEPENDENCY_UNAVAILABLE");
    assert!(!value.to_string().contains("phishing"));
}
