use super::*;

#[tokio::test]
async fn duplicates_changed_totals_and_contradictions_stop_the_read() {
    // Duplicate inside one page: keep the first occurrence, drop the repeat.
    let server = serve(|_, _, body: &Value| {
        listing(
            body["page"].as_i64().unwrap(),
            body["pageSize"].as_i64().unwrap(),
            &["a1", "a2", "a1"],
            3,
            false,
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
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "DATA_CHANGED");
    assert_eq!(pagination(&value)["reason"], "data-changed");
    assert_eq!(pagination(&value)["itemsFetched"], 3);
    assert_eq!(pagination(&value)["itemsReturned"], 2);

    // Duplicate across pages: the first occurrence survives.
    let server = serve(|_, _, body: &Value| {
        let page = body["page"].as_i64().unwrap();
        if page == 1 {
            listing(1, 2, &["a1", "b2"], 4, true)
        } else {
            listing(2, 2, &["c3", "b2"], 4, false)
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
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "DATA_CHANGED");
    let kept: Vec<&str> = body_of(&value)
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(kept, vec!["a1", "b2", "c3"]);

    // The reported total changes between pages.
    let server = serve(|_, _, body: &Value| {
        let page = body["page"].as_i64().unwrap();
        listing(
            page,
            2,
            if page == 1 {
                &["a1", "b2"]
            } else {
                &["c3", "d4"]
            },
            if page == 1 { 4 } else { 9 },
            page < 3,
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
            "--page-size",
            "2",
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "DATA_CHANGED");
    assert_eq!(pagination(&value)["matchedTotal"], 4);
    assert_eq!(pagination(&value)["itemsFetched"], 4);
    let kept: Vec<&str> = body_of(&value)
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(kept, vec!["a1", "b2", "c3", "d4"]);

    // hasMore=true must name exactly the next page.
    let server = serve(|_, _, body: &Value| {
        Reply::json(
            &Page::new(&["a1"])
                .total(4)
                .has_more(true)
                .next(Some(7))
                .to_json(
                    body["page"].as_i64().unwrap(),
                    body["pageSize"].as_i64().unwrap(),
                ),
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
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "PAGINATION_UNKNOWN");
    assert_eq!(value["data"], Value::Null);
    assert_eq!(value["meta"]["paginationDiagnostic"]["stage"], "contract");
    assert_eq!(
        value["meta"]["paginationDiagnostic"]["values"]["nextPage"],
        7
    );
    assert_eq!(value["meta"]["paginationDiagnostic"]["request"]["page"], 1);
    assert_eq!(server.requests().len(), 1);

    // hasMore=false must not also promise a next page, even with data already read.
    let server = serve(|_, _, body: &Value| {
        let page = body["page"].as_i64().unwrap();
        let size = body["pageSize"].as_i64().unwrap();
        if page == 1 {
            listing(1, size, &["a1"], 4, true)
        } else {
            Reply::json(
                &Page::new(&["b2"])
                    .total(4)
                    .has_more(false)
                    .next(Some(3))
                    .to_json(page, size),
            )
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
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "PAGINATION_UNKNOWN");
    assert_eq!(pagination(&value)["pagesRead"], 1);
    assert_eq!(pagination(&value)["scopeComplete"], Value::Null);
    assert_eq!(body_of(&value).as_array().unwrap().len(), 1);
}

// 16 and 17: an anomalous empty follow-up page and malformed pagination metadata.
#[tokio::test]
async fn anomalous_empty_pages_and_malformed_metadata_are_never_complete() {
    let server = serve(|_, _, body: &Value| {
        let page = body["page"].as_i64().unwrap();
        if page == 1 {
            listing(1, 1, &["a1"], 2, true)
        } else {
            Reply::json(
                &Page::new(&[])
                    .total(2)
                    .has_more(false)
                    .to_json(page, body["pageSize"].as_i64().unwrap()),
            )
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
            "1",
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "PAGINATION_UNKNOWN");
    assert_eq!(pagination(&value)["pagesRead"], 1);
    assert_eq!(server.requests().len(), 2);

    // Missing pagination object.
    let server = serve(|_, _, body: &Value| {
        Reply::json(&Page::new(&["a1"]).no_pagination().to_json(
            body["page"].as_i64().unwrap(),
            body["pageSize"].as_i64().unwrap(),
        ))
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &["workflow", "list", "--kind", "todo", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "PAGINATION_UNKNOWN");

    // Unknown server metadata fields fail closed while retaining the safe trace.
    let server = serve(|_, _, body: &Value| {
        let mut response = Page::new(&["a1"]).to_json(
            body["page"].as_i64().unwrap(),
            body["pageSize"].as_i64().unwrap(),
        );
        response["meta"]["pagination"]["serverExtension"] =
            json!("https://example.invalid/?ticket=synthetic-diagnostic-value");
        Reply::json(&response)
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &["workflow", "list", "--kind", "todo", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "PAGINATION_UNKNOWN");
    assert_eq!(value["meta"]["traceId"], "0123456789abcdef");
    assert_eq!(value["data"], Value::Null);
    assert_eq!(
        value["meta"]["paginationDiagnostic"]["stage"],
        "deserialize"
    );
    assert_eq!(
        value["meta"]["paginationDiagnostic"]["unexpectedFieldCount"],
        1
    );
    assert_eq!(
        value["meta"]["paginationDiagnostic"]["fieldTypes"]["pageSize"],
        "number"
    );
    assert!(
        !value
            .to_string()
            .contains("https://example.invalid/?ticket=synthetic-diagnostic-value")
    );

    let (aggregated, aggregate_exit) = run(
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
    assert_eq!(aggregate_exit, 7, "{aggregated}");
    assert_eq!(aggregated["error"]["code"], "PAGINATION_UNKNOWN");
    assert_eq!(
        aggregated["meta"]["paginationDiagnostic"]["stage"],
        "deserialize"
    );
    assert!(
        !aggregated
            .to_string()
            .contains("https://example.invalid/?ticket=synthetic-diagnostic-value")
    );

    // A required pagination field that is missing altogether.
    let server = serve(|_, _, body: &Value| {
        Reply::json(&Page::new(&["a1"]).omit_field("pagesRead").to_json(
            body["page"].as_i64().unwrap(),
            body["pageSize"].as_i64().unwrap(),
        ))
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &["workflow", "list", "--kind", "todo", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "PAGINATION_UNKNOWN");

    // A page number that does not match the request.
    let server = serve(|_, _, body: &Value| {
        Reply::json(&Page::new(&["a1"]).page(99).to_json(
            body["page"].as_i64().unwrap(),
            body["pageSize"].as_i64().unwrap(),
        ))
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &["workflow", "list", "--kind", "todo", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "PAGINATION_UNKNOWN");

    // A body that is not JSON at all.
    let server = serve(|_, _, _: &Value| Reply::raw(200, b"not-json".to_vec()));
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &["workflow", "list", "--kind", "todo", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 5, "{value}");
    assert_eq!(value["error"]["code"], "DEPENDENCY_UNAVAILABLE");

    // A message without a usable identifier.
    let server = serve(|_, _, body: &Value| {
        let mut item = message("a1");
        item["id"] = json!("");
        Reply::json(&Page::items(vec![item]).total(1).has_more(false).to_json(
            body["page"].as_i64().unwrap(),
            body["pageSize"].as_i64().unwrap(),
        ))
    });
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &["workflow", "list", "--kind", "todo", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 5, "{value}");
    assert_eq!(value["error"]["code"], "DEPENDENCY_UNAVAILABLE");
}

// 10 and 11: subsequent-page failures keep the earlier pages; first-page failures do not fake them.
#[tokio::test]
async fn truncated_or_inconsistent_pages_cannot_become_complete_aggregates() {
    for (field, replacement) in [
        ("truncated", Some(json!(true))),
        ("reason", Some(json!("server-limit"))),
        ("matchedTotal", Some(json!(-1))),
        ("matchedTotal", None),
        ("hasMore", None),
        ("scopeComplete", None),
        ("nextPage", None),
        ("reason", None),
    ] {
        let server = serve(move |_, _, body: &Value| {
            let mut value = Page::new(&["a1"]).total(1).has_more(false).to_json(
                body["page"].as_i64().unwrap(),
                body["pageSize"].as_i64().unwrap(),
            );
            let pagination = value["meta"]["pagination"].as_object_mut().unwrap();
            if let Some(replacement) = &replacement {
                pagination.insert(field.into(), replacement.clone());
            } else {
                pagination.remove(field);
            }
            Reply::json(&value)
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
        assert_eq!(exit, 7, "{field}: {value}");
        assert_eq!(value["error"]["code"], "PAGINATION_UNKNOWN");
        assert_ne!(pagination(&value)["scopeComplete"], true);
    }
}
