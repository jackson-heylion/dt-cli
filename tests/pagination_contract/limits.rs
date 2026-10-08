use super::*;

#[tokio::test]
async fn default_reads_one_page_and_all_starts_at_page_one() {
    let server = serve(three_pages);
    let h = harness(&server.origin, Duration::from_secs(5));

    let (value, exit) = run(
        &h.runtime,
        &["workflow", "list", "--kind", "todo", "--profile", "p"],
    )
    .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(server.requests().len(), 1);
    assert_eq!(body_of(&value)["items"].as_array().unwrap().len(), 2);
    assert_eq!(server.requests()[0].1["pageSize"], 50);
    assert!(server.requests()[0].1.get("all").is_none());

    let server = serve(three_pages);
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
    assert_eq!(exit, 0, "{value}");
    assert_eq!(server.pages(), vec![1, 2, 3]);
    assert!(
        server
            .requests()
            .iter()
            .all(|(_, body)| body["pageSize"] == 2)
    );
    assert!(
        server
            .requests()
            .iter()
            .all(|(_, body)| body.get("all").is_none() && body.get("maxPages").is_none())
    );
    assert_eq!(pagination(&value)["mode"], "all");
    assert_eq!(pagination(&value)["scopeComplete"], true);
    assert_eq!(pagination(&value)["truncated"], false);
    assert_eq!(pagination(&value)["reason"], Value::Null);
    assert_eq!(pagination(&value)["pagesRead"], 3);
    assert_eq!(pagination(&value)["itemsReturned"], 5);
    assert_eq!(pagination(&value)["matchedTotal"], 5);
    assert_eq!(body_of(&value).as_array().unwrap().len(), 5);
    assert_eq!(pagination(&value)["consistency"], Value::Null);
    assert_eq!(value["meta"]["source"]["consistency"], "non_snapshot");
    assert_eq!(value["meta"]["source"]["freshness"], "unknown");
    assert!(value["meta"]["source"]["fetchedAt"].is_string());
    assert_eq!(value["meta"]["traceId"], "0123456789abcdef");
}

// 3 and 7: within maxPages, exactly at maxPages, and still more data.
#[tokio::test]
async fn page_ceiling_is_inclusive_only_when_the_scope_ends() {
    let server = serve(three_pages);
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
            "--max-pages",
            "3",
        ],
    )
    .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(server.requests().len(), 3);

    let server = serve(three_pages);
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
            "--max-pages",
            "2",
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "RESULT_LIMIT");
    assert_eq!(pagination(&value)["reason"], "max-pages");
    assert_eq!(pagination(&value)["truncated"], true);
    assert_eq!(pagination(&value)["scopeComplete"], false);
    assert_eq!(pagination(&value)["contentComplete"], true);
    assert_eq!(pagination(&value)["pageComplete"], true);
    assert_eq!(server.requests().len(), 2);
    assert_eq!(body_of(&value).as_array().unwrap().len(), 4);

    let server = serve(three_pages);
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
            "--max-pages",
            "1",
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(pagination(&value)["reason"], "max-pages");
    assert_eq!(body_of(&value).as_array().unwrap().len(), 2);
}

// 4, 5 and 6: the retention ceiling, including the page that exactly reaches it.
#[tokio::test]
async fn item_ceiling_separates_fetched_from_returned() {
    // A last page that exactly reaches maxItems and proves the range ended stays successful.
    let server = serve(|_, _, body: &Value| three_pages(0, "", body));
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
            "--max-items",
            "5",
        ],
    )
    .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(pagination(&value)["itemsReturned"], 5);
    assert_eq!(pagination(&value)["itemsFetched"], 5);
    assert_eq!(pagination(&value)["scopeComplete"], true);

    // More data behind the ceiling: complete records are retained and the read stops.
    let server = serve(three_pages);
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
            "--max-items",
            "3",
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "RESULT_LIMIT");
    assert_eq!(pagination(&value)["reason"], "max-items");
    assert_eq!(pagination(&value)["itemsReturned"], 3);
    assert_eq!(pagination(&value)["itemsFetched"], 4);
    assert_eq!(pagination(&value)["scopeComplete"], false);
    assert_eq!(server.requests().len(), 2);

    // A page that returns more than the remaining allowance reports both counters separately.
    let server = serve(|_, _, body: &Value| {
        listing(
            body["page"].as_i64().unwrap(),
            body["pageSize"].as_i64().unwrap(),
            &["a1", "b2", "c3"],
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
            "--page-size",
            "3",
            "--max-items",
            "2",
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(pagination(&value)["itemsFetched"], 3);
    assert_eq!(pagination(&value)["itemsReturned"], 2);
}

// 12: a genuinely empty result set is a success, not an error.
#[tokio::test]
async fn an_empty_scope_succeeds_and_a_missing_total_does_not() {
    let server = serve(|_, _, body: &Value| {
        Reply::json(&Page::new(&[]).total(0).has_more(false).to_json(
            body["page"].as_i64().unwrap(),
            body["pageSize"].as_i64().unwrap(),
        ))
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
    assert_eq!(exit, 0, "{value}");
    assert_eq!(body_of(&value).as_array().unwrap().len(), 0);
    assert_eq!(pagination(&value)["matchedTotal"], 0);
    assert_eq!(pagination(&value)["scopeComplete"], true);
    assert_eq!(pagination(&value)["pagesRead"], 1);

    let server = serve(|_, _, body: &Value| {
        Reply::json(&Page::new(&[]).unknown_total().has_more(false).to_json(
            body["page"].as_i64().unwrap(),
            body["pageSize"].as_i64().unwrap(),
        ))
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
    assert_eq!(pagination(&value)["scopeComplete"], Value::Null);
    assert_eq!(pagination(&value)["matchedTotal"], Value::Null);
}

// 13, 14 and 15: duplicates, a changed total and contradictory continuation hints.
