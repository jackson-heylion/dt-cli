use super::*;

#[tokio::test]
async fn aggregation_flags_are_validated_offline() {
    let server = serve(|_, _, _: &Value| Reply::json(&json!({})));
    let root = tempfile::tempdir().unwrap();
    let runtime = sealed(root.path(), &server.origin, Duration::from_secs(5));
    let cases: Vec<Vec<&str>> = vec![
        vec![
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--max-pages",
            "5",
        ],
        vec![
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--max-items",
            "5",
        ],
        vec![
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--all",
            "--page",
            "2",
        ],
        vec![
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--all",
            "--max-pages",
            "0",
        ],
        vec![
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--all",
            "--max-pages",
            "21",
        ],
        vec![
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--all",
            "--max-items",
            "0",
        ],
        vec![
            "workflow",
            "list",
            "--kind",
            "todo",
            "--profile",
            "p",
            "--all",
            "--max-items",
            "1001",
        ],
        vec![
            "api",
            "call",
            "iam.workflow.list",
            "--profile",
            "p",
            "--params",
            "{\"kind\":\"todo\",\"all\":false,\"maxPages\":5}",
        ],
        vec![
            "api",
            "call",
            "iam.workflow.list",
            "--profile",
            "p",
            "--params",
            "{\"kind\":\"todo\",\"all\":true,\"page\":2}",
        ],
        vec![
            "api",
            "call",
            "iam.workflow.list",
            "--profile",
            "p",
            "--params",
            "{\"kind\":\"todo\",\"all\":true,\"maxPages\":999}",
        ],
        vec![
            "api",
            "call",
            "iam.workflow.list",
            "--profile",
            "p",
            "--params",
            "{\"kind\":\"todo\",\"all\":true,\"maxItems\":1.5}",
        ],
        vec![
            "api",
            "call",
            "iam.workflow.list",
            "--profile",
            "p",
            "--params",
            "{\"kind\":\"todo\",\"all\":true,\"maxPages\":-1}",
        ],
    ];
    for args in cases {
        let (value, exit) = run(&runtime, &args).await;
        assert_eq!(exit, 2, "{args:?}: {value}");
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT", "{args:?}");
    }
    assert_eq!(server.requests().len(), 0);
}

// `--all` with page 1 stays legal, and a rejected aggregation never reaches the network.
#[tokio::test]
async fn an_all_request_on_page_one_is_legal() {
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
            "--page",
            "1",
        ],
    )
    .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(server.pages(), vec![1, 2, 3]);

    let server = serve(three_pages);
    let h = harness(&server.origin, Duration::from_secs(5));
    let (value, exit) = run(
        &h.runtime,
        &[
            "api",
            "call",
            "iam.workflow.list",
            "--profile",
            "p",
            "--params",
            "{\"kind\":\"todo\",\"all\":true,\"page\":1,\"pageSize\":2,\"maxItems\":4}",
        ],
    )
    .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(server.pages(), vec![1, 2, 3]);
    assert_eq!(pagination(&value)["reason"], "max-items");
}
