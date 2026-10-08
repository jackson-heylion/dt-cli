use super::*;

#[tokio::test]
async fn later_page_failures_keep_the_first_page_and_first_page_failures_do_not() {
    type Case = (&'static str, Box<dyn Fn() -> Reply + Send + Sync>);
    let cases: Vec<Case> = vec![
        (
            "library error",
            Box::new(|| {
                Reply::error(
                    500,
                    json!({"ok":false,"error":{"code":"INTERNAL_ERROR"},"meta":{}}),
                )
            }),
        ),
        (
            "oversized page",
            Box::new(|| {
                Reply::error(
                    413,
                    json!({"ok":false,"error":{"code":"RESULT_LIMIT"},"meta":{}}),
                )
            }),
        ),
        (
            "throttled with unusable hint",
            Box::new(|| {
                Reply::error(
                    429,
                    json!({"ok":false,"error":{"code":"DEPENDENCY_UNAVAILABLE"},"meta":{}}),
                )
                .header("Retry-After", "not-a-delay")
            }),
        ),
        ("transport failure", Box::new(Reply::abort)),
    ];
    for (name, build) in cases {
        let server = serve(move |_, _, body: &Value| {
            let page = body["page"].as_i64().unwrap();
            if page == 1 {
                listing(1, 2, &["a1", "b2"], 4, true)
            } else {
                build()
            }
        });
        let h = harness(&server.origin, Duration::from_secs(5));
        let (value, exit) = run(
            &h.runtime,
            &[
                "workflow",
                "list",
                "--kind",
                "todo",
                "--profile",
                "p",
                "--all",
                "--page-size",
                "2",
            ],
        )
        .await;
        assert_eq!(exit, 7, "{name}: {value}");
        assert!(
            matches!(
                value["error"]["code"].as_str().unwrap(),
                "PARTIAL_RESULT" | "RESULT_LIMIT"
            ),
            "{name}: {value}"
        );
        assert_eq!(body_of(&value).as_array().unwrap().len(), 2, "{name}");
        assert_eq!(pagination(&value)["pagesRead"], 1, "{name}");
        assert_eq!(pagination(&value)["pageComplete"], false, "{name}");
        assert_eq!(pagination(&value)["scopeComplete"], false, "{name}");
    }

    // The first page failing keeps the original classification and reports no records at all.
    let server = serve(|_, _, _: &Value| {
        Reply::error(
            500,
            json!({"ok":false,"error":{"code":"INTERNAL_ERROR"},"meta":{"traceId":"0123456789abcdef"}}),
        )
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &[
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--all",
        ],
    )
    .await;
    assert_eq!(exit, 1, "{value}");
    assert_eq!(value["error"]["code"], "INTERNAL_ERROR");
    assert_eq!(value["data"], Value::Null);
    assert_eq!(pagination(&value)["pagesRead"], 0);
    assert_eq!(pagination(&value)["pageComplete"], false);
    assert_eq!(pagination(&value)["mode"], "all");

    let server = serve(|_, _, _: &Value| Reply::error(503, json!({"ok":false,"meta":{}})));
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &[
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--all",
        ],
    )
    .await;
    assert_eq!(exit, 5, "{value}");
    assert_eq!(value["data"], Value::Null);
}

// 18: one resend budget shared by transport failures and the expiry recovery.
#[tokio::test]
async fn one_resend_budget_covers_transport_and_expiry_recovery() {
    let transports = Arc::new(Mutex::new(0usize));
    let counter = transports.clone();
    let server = serve(move |_, path: &str, _: &Value| {
        if path.ends_with("/cli/oauth2/token") {
            return Reply::json(&json!({"error":"unexpected"}));
        }
        if *counter.lock().unwrap() == 0 {
            *counter.lock().unwrap() = 1;
            return Reply::abort();
        }
        Reply::error(
            401,
            json!({"ok":false,"error":{"code":"ACCESS_TOKEN_EXPIRED"},"meta":{}}),
        )
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &["workflow", "list", "--kind", "todo", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 3, "{value}");
    assert_eq!(value["error"]["code"], "ACCESS_TOKEN_EXPIRED");
    let listing_calls = server
        .requests()
        .iter()
        .filter(|(path, _)| path.contains("messages/query"))
        .count();
    assert_eq!(listing_calls, 2, "the budget forbids a third business send");

    // The expiry recovery itself may use the budget once and then succeed.
    let calls = Arc::new(Mutex::new(0usize));
    let counter = calls.clone();
    let server = serve(move |_, path: &str, body: &Value| {
        if path.ends_with("/cli/oauth2/token") {
            return Reply::raw(200, b"{\"broken\":true}".to_vec());
        }
        let mut calls = counter.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            return Reply::error(
                401,
                json!({"ok":false,"error":{"code":"ACCESS_TOKEN_EXPIRED"},"meta":{}}),
            );
        }
        let page = body["page"].as_i64().unwrap();
        listing(page, body["pageSize"].as_i64().unwrap(), &["a1"], 1, false)
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &["workflow", "list", "--kind", "todo", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 3, "{value}");
    assert_eq!(value["error"]["code"], "AUTH_REFRESH_OUTCOME_UNKNOWN");
}

// 19: Retry-After is honoured only when it fits the remaining budget.
#[tokio::test]
async fn retry_responses_share_the_byte_allowance() {
    let first = json!({"ok":false,"error":{"code":"UNAVAILABLE"},"padding":"x".repeat(200)});
    let second = Page::new(&["a1"]).total(1).has_more(false).to_json(1, 1);
    let total =
        serde_json::to_vec(&first).unwrap().len() + serde_json::to_vec(&second).unwrap().len();
    for allowance in [total, total - 1] {
        let first = first.clone();
        let second = second.clone();
        let server = serve(move |index, _, _: &Value| {
            if index == 0 {
                Reply::error(503, first.clone()).header("Retry-After", "0")
            } else {
                Reply::json(&second)
            }
        });
        let h = harness(&server.origin, Duration::from_secs(5));
        let p = dt_cli::profile::read(&h.runtime.root, "p")
            .unwrap()
            .unwrap();
        let env = h.runtime.environment("fixture").unwrap();
        let client = dt_cli::http::client().unwrap();
        let mut credentials = h.store.read(CREDENTIAL_KEY).unwrap().unwrap();
        let mut cancel = pagination::Cancel::disabled();
        let result = pagination::fetch_page(
            &h.runtime,
            &mut cancel,
            std::time::Instant::now() + Duration::from_secs(5),
            &client,
            &p,
            env,
            &mut credentials,
            &json!({"kind":"todo","page":1,"pageSize":1}),
            allowance,
        )
        .await;
        let pagination::Attempt::Page(response) = result else {
            panic!("unexpected deadline");
        };
        if allowance == total {
            assert!(response.failure.is_none());
            assert_eq!(response.bytes, total);
            assert_eq!(response.data.len(), 1);
        } else {
            assert_eq!(response.failure.unwrap().code, "RESULT_LIMIT");
            assert!(response.data.is_empty());
        }
        assert_eq!(server.requests().len(), 2);
    }
}

#[tokio::test]
async fn retry_after_respects_the_remaining_budget() {
    let calls = Arc::new(Mutex::new(0usize));
    let counter = calls.clone();
    let server = serve(move |_, _: &str, body: &Value| {
        let mut calls = counter.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            return Reply::error(429, json!({"ok":false,"meta":{}})).header("Retry-After", "1");
        }
        listing(
            body["page"].as_i64().unwrap(),
            body["pageSize"].as_i64().unwrap(),
            &["a1"],
            1,
            false,
        )
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &["workflow", "list", "--kind", "todo", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(server.requests().len(), 2);

    let server = serve(|_, _, _: &Value| {
        Reply::error(503, json!({"ok":false,"meta":{}})).header("Retry-After", "30")
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &["workflow", "list", "--kind", "todo", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 5, "{value}");
    assert_eq!(server.requests().len(), 1, "no wait beyond the budget");
}

// 9: the absolute budget stops a read that is still waiting for data.
#[tokio::test]
async fn the_absolute_budget_bounds_the_read() {
    let server = serve(|_, _, _: &Value| Reply::json(&json!({})).delay(2_000));
    let h = harness(&server.origin, Duration::from_millis(600));
    let (value, exit) = run(
        &h.runtime,
        &[
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--all",
        ],
    )
    .await;
    assert_eq!(exit, 5, "{value}");
    assert_eq!(value["error"]["code"], "TIMEOUT");
    assert_eq!(value["data"], Value::Null);
    assert_eq!(pagination(&value)["pagesRead"], 0);

    let server = serve(|_, _, body: &Value| {
        let page = body["page"].as_i64().unwrap();
        if page == 1 {
            listing(1, 1, &["a1"], 3, true)
        } else {
            Reply::json(&Page::new(&["b2"]).to_json(page, 1)).delay(2_000)
        }
    });
    let h = harness(&server.origin, Duration::from_millis(600));
    let (value, exit) = run(
        &h.runtime,
        &[
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--all",
            "--page-size",
            "1",
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "RESULT_LIMIT");
    assert_eq!(pagination(&value)["reason"], "deadline");
    assert_eq!(pagination(&value)["truncated"], true);
    assert_eq!(pagination(&value)["pagesRead"], 1);
    assert_eq!(body_of(&value).as_array().unwrap().len(), 1);
}

// 20: cancelling stops the in-flight page and never requests the next one.
