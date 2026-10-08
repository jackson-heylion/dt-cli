use super::*;

fn kind_page(body: &Value, ids: &[&str]) -> Reply {
    let mut items: Vec<_> = ids.iter().map(|id| message(id)).collect();
    for item in &mut items {
        item["kind"] = body["kind"].clone();
    }
    Reply::json(&Page::items(items).total(ids.len() as i64).to_json(
        body["page"].as_i64().unwrap(),
        body["pageSize"].as_i64().unwrap(),
    ))
}

#[tokio::test]
async fn brief_groups_real_fields_and_reads_each_requested_kind_once() {
    let server = serve(|_, _, body| kind_page(body, &["1", "2"]));
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &[
            "tasks",
            "run",
            "inbox.brief",
            "--profile",
            "p",
            "--params",
            r#"{"kinds":["todo","cc"]}"#,
        ],
    )
    .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["meta"]["complete"], true);
    assert_eq!(value["data"]["result"]["counts"], json!({"todo":2,"cc":2}));
    assert_eq!(
        value["data"]["result"]["items"].as_array().unwrap().len(),
        4
    );
    assert_eq!(
        value["data"]["result"]["groups"].as_array().unwrap().len(),
        2
    );
    assert_eq!(value["data"]["result"]["items"][0]["formType"], Value::Null);
    assert_eq!(
        value["data"]["result"]["items"][0]["openAction"]["argv"][3],
        "--id"
    );
    assert_eq!(server.requests().len(), 2);
    assert_eq!(server.requests()[0].1["pageSize"], 50);
    assert!(!value.to_string().contains("dueAt"));
}

#[tokio::test]
async fn brief_shares_item_and_page_budget_and_marks_unread_kind_unknown() {
    for params in [
        r#"{"kinds":["todo","cc"],"maxItems":2}"#,
        r#"{"kinds":["todo","cc"],"maxPages":1}"#,
    ] {
        let server = serve(|_, _, body| kind_page(body, &["1", "2"]));
        let h = harness(&server.origin, Duration::from_secs(5));
        let (value, exit) = run(
            &h.runtime,
            &[
                "tasks",
                "run",
                "inbox.brief",
                "--profile",
                "p",
                "--params",
                params,
            ],
        )
        .await;
        assert_eq!(exit, 7, "{value}");
        assert_eq!(value["meta"]["complete"], false);
        assert_eq!(value["data"]["result"]["counts"]["todo"], 2);
        assert_eq!(value["data"]["result"]["counts"]["cc"], Value::Null);
        assert_eq!(value["meta"]["sources"][1]["state"], "not-read");
        assert_eq!(server.requests().len(), 1);
    }
}

#[tokio::test]
async fn brief_later_kind_failure_keeps_prior_results_and_first_failure_is_not_zero() {
    let server = serve(|_, _, body| {
        if body["kind"] == "todo" {
            kind_page(body, &["1"])
        } else {
            Reply::error(
                403,
                json!({"schemaVersion":"1.0","ok":false,"error":{"code":"AUTH_DENIED"},"data":null,"meta":{}}),
            )
        }
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &[
            "tasks",
            "run",
            "inbox.brief",
            "--profile",
            "p",
            "--params",
            r#"{"kinds":["todo","cc","done"]}"#,
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(
        value["data"]["result"]["items"].as_array().unwrap().len(),
        1
    );
    assert_eq!(value["data"]["result"]["counts"]["cc"], Value::Null);
    assert_eq!(value["meta"]["sources"][2]["state"], "not-read");
    let server = serve(|_, _, _| {
        Reply::error(
            403,
            json!({"schemaVersion":"1.0","ok":false,"error":{"code":"AUTH_DENIED"},"data":null,"meta":{}}),
        )
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &["tasks", "run", "inbox.brief", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 4, "{value}");
    assert_eq!(value["data"]["result"]["counts"]["todo"], Value::Null);
}

#[tokio::test]
async fn brief_dry_run_has_no_credentials_network_browser_or_record_writes() {
    let h = harness("http://127.0.0.1:9", Duration::from_secs(1));
    let dir = &h._root;
    let rt = sealed(dir.path(), "http://127.0.0.1:9", Duration::from_secs(1));
    let (value, exit) = run(
        &rt,
        &[
            "tasks",
            "run",
            "inbox.brief",
            "--profile",
            "p",
            "--params",
            "{}",
            "--dry-run",
        ],
    )
    .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["data"]["result"]["httpRequests"], 0);
    assert_eq!(
        value["data"]["result"]["limits"]["aggregateBytes"],
        10485760
    );
    assert!(!dir.path().join("workpacks").exists());
}

#[tokio::test]
async fn brief_shares_byte_limit_across_kinds_instead_of_resetting_each_query() {
    let server = serve(|_, _, body| {
        let page = body["page"].as_i64().unwrap();
        let items: Vec<_> = (0..50)
            .map(|offset| {
                let mut item = message(&(page * 100 + offset).to_string());
                item["kind"] = body["kind"].clone();
                item["title"] = json!("x".repeat(40000));
                item
            })
            .collect();
        Reply::json(
            &Page::items(items)
                .total(150)
                .has_more(page < 3)
                .next(if page < 3 { Some(page + 1) } else { None })
                .to_json(page, 50),
        )
    });
    let h = harness(&server.origin, Duration::from_secs(10));
    let (value, exit) = run(
        &h.runtime,
        &[
            "tasks",
            "run",
            "inbox.brief",
            "--profile",
            "p",
            "--params",
            r#"{"kinds":["todo","cc","done"]}"#,
        ],
    )
    .await;
    assert_eq!(exit, 7, "{}", value["error"]);
    assert_eq!(value["data"]["result"]["counts"]["todo"], 150);
    assert_eq!(value["data"]["result"]["counts"]["cc"], 100);
    assert_eq!(value["data"]["result"]["counts"]["done"], Value::Null);
    assert!(value["meta"]["budget"]["bytesRead"].as_u64().unwrap() <= 10485760);
    assert_eq!(server.requests().len(), 6);
}

#[tokio::test]
async fn brief_shares_deadline_across_kinds_and_does_not_read_after_stop() {
    let server = serve(|_, _, body| kind_page(body, &["1"]).delay(200));
    let h = harness(&server.origin, Duration::from_millis(350));
    let started = std::time::Instant::now();
    let (value, exit) = run(
        &h.runtime,
        &[
            "tasks",
            "run",
            "inbox.brief",
            "--profile",
            "p",
            "--params",
            r#"{"kinds":["todo","cc","done"]}"#,
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["data"]["result"]["counts"]["todo"], 1);
    assert_eq!(value["meta"]["sources"][2]["state"], "not-read");
    assert_eq!(server.requests().len(), 2);
    assert!(started.elapsed() < Duration::from_millis(650));
}

#[tokio::test]
async fn brief_table_preserves_real_id_times_filters_and_selected_open_action() {
    let server = serve(|_, _, body| kind_page(body, &["1"]));
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit, table) = run_table(
        &h.runtime,
        &[
            "tasks",
            "run",
            "inbox.brief",
            "--profile",
            "p",
            "--format",
            "table",
        ],
    )
    .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["meta"]["availability"]["state"], "ready");
    assert_eq!(value["data"]["result"]["filters"]["kinds"], json!(["todo"]));
    for literal in [
        "ID",
        "1\ttodo",
        "2026-09-01T02:00:00Z",
        "2026-09-01T03:00:00Z",
        "processedAt",
        "endedAt",
        "filters",
        "选中后打开",
        "workflow",
        "--id",
    ] {
        assert!(table.contains(literal), "{literal}: {table}");
    }
    assert!(!table.contains("dtcli_a_"));
}
