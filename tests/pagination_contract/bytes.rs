use super::*;

fn sized_page_body(
    target: usize,
    page: i64,
    size: i64,
    id: &str,
    total: i64,
    has_more: bool,
) -> String {
    let build = |pad: usize| -> String {
        let mut item = message(id);
        item["title"] = json!("x".repeat(pad));
        serde_json::to_string(
            &Page::items(vec![item])
                .total(total)
                .has_more(has_more)
                .next(has_more.then_some(page + 1))
                .to_json(page, size),
        )
        .unwrap()
    };
    let base = build(0);
    assert!(base.len() < target, "fixture body cannot grow to {target}");
    let body = build(target - base.len());
    assert_eq!(body.len(), target);
    body
}

// 8: the single-page and aggregate byte ceilings, before, exactly at, and beyond the limit.
#[tokio::test]
async fn byte_ceilings_are_enforced_at_the_boundaries() {
    let page_limit = dt_cli::http::PAGE_BYTE_LIMIT;
    let aggregate_limit = dt_cli::http::AGGREGATE_BYTE_LIMIT;

    // A prior transient error must not reduce the legal size of a single-page response.
    for retry in [false, true] {
        let server = serve(move |index, _, _: &Value| {
            if retry && index == 0 {
                return Reply::error(503, json!({"ok":false})).header("Retry-After", "0");
            }
            Reply::raw(
                200,
                sized_page_body(page_limit, 1, 1, "a1", 1, false).into_bytes(),
            )
        });
        let h = harness(&server.origin, Duration::from_secs(20));
        let (value, exit) = run(
            &h.runtime,
            &[
                "workflow",
                "list",
                "--kind",
                "todo",
                "--profile",
                "p",
                "--page-size",
                "1",
            ],
        )
        .await;
        assert_eq!(exit, 0, "{value}");

        assert_eq!(server.requests().len(), if retry { 2 } else { 1 });
    }

    // One byte beyond the page ceiling.
    let server = serve(move |_, _, _: &Value| {
        Reply::raw(
            200,
            sized_page_body(page_limit + 1, 1, 1, "a1", 1, false).into_bytes(),
        )
    });
    let h = harness(&server.origin, Duration::from_secs(20));
    let (value, exit) = run(
        &h.runtime,
        &[
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--page-size",
            "1",
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "RESULT_LIMIT");

    // Five full pages: exactly the aggregate ceiling, and the last page proves the end.
    let server = serve(move |_, _, body: &Value| {
        let page = body["page"].as_i64().unwrap();
        Reply::raw(
            200,
            sized_page_body(page_limit, page, 1, &format!("a{page}"), 5, page < 5).into_bytes(),
        )
    });
    let h = harness(&server.origin, Duration::from_secs(30));
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
    assert_eq!(exit, 0, "{value}");
    assert_eq!(pagination(&value)["pagesRead"], 5);
    assert_eq!(pagination(&value)["itemsReturned"], 5);
    assert_eq!(pagination(&value)["contentComplete"], true);

    // One more page exists: the aggregate ceiling stops the read before requesting it.
    let server = serve(move |_, _, body: &Value| {
        let page = body["page"].as_i64().unwrap();
        Reply::raw(
            200,
            sized_page_body(page_limit, page, 1, &format!("a{page}"), 6, page < 6).into_bytes(),
        )
    });
    let h = harness(&server.origin, Duration::from_secs(30));
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
    assert_eq!(pagination(&value)["reason"], "bytes");
    assert_eq!(pagination(&value)["pagesRead"], 5);
    assert_eq!(pagination(&value)["itemsReturned"], 5);
    assert_eq!(pagination(&value)["contentComplete"], true);
    assert_eq!(server.requests().len(), 5);
    assert!(aggregate_limit == 5 * page_limit);
}
