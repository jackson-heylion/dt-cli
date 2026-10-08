use super::*;

#[tokio::test]
async fn jobs_lifecycle_only_returns_complete_results() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let start = iam.count();
    let (submitted, exit) = h
        .run(&[
            "jobs",
            "submit",
            BATCH,
            "--profile",
            "supply",
            "--params",
            r#"{"itemCode":"101010009","locatedPartitionId":3}"#,
        ])
        .await;
    assert_eq!(exit, 0, "{submitted}");
    let job = submitted["data"]["jobId"].as_str().unwrap().to_owned();
    assert_eq!(job.len(), 43);
    assert_eq!(submitted["data"]["state"], "queued");
    assert_eq!(submitted["data"]["complete"], false);
    assert_eq!(
        iam.paths_since(start),
        vec!["POST /cli-api/oauth2/exchange", "POST /cli-api/v1/jobs",]
    );
    let seen = iam.since(start);
    assert!(
        seen[1]
            .authorization
            .as_deref()
            .unwrap()
            .starts_with("Bearer dtcli_g_s_")
    );
    let (status, exit) = h
        .run(&["jobs", "status", &job, "--profile", "supply"])
        .await;
    assert_eq!(exit, 0, "{status}");
    assert_eq!(status["data"]["state"], "queued");
    let last = iam.seen().last().cloned().unwrap();
    assert!(
        last.authorization
            .as_deref()
            .unwrap()
            .starts_with("Bearer dtcli_g_a_"),
        "status uses the parent access token, never the system token"
    );
    let (value, exit) = h
        .run(&["jobs", "result", &job, "--profile", "supply"])
        .await;
    assert_failure(&value, exit, 7, "PARTIAL_RESULT");
    assert!(value["data"].is_null());
    // A server that claims a result before completion is still refused.
    iam.with(|f| {
        f.overrides.insert(
            "jobs.result",
            reply(
                200,
                ok(
                    "job.result",
                    json!({"jobId":job,"state":"running","complete":false,"items":items()}),
                ),
            ),
        )
    });
    let (value, exit) = h
        .run(&["jobs", "result", &job, "--profile", "supply"])
        .await;
    assert_failure(&value, exit, 7, "PARTIAL_RESULT");
    assert!(!value.to_string().contains("101010009"), "{value}");
    iam.with(|f| f.overrides.clear());
    let (value, exit) = h
        .run(&["jobs", "cancel", &job, "--profile", "supply"])
        .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["data"]["state"], "cancelled");
    let (value, exit) = h
        .run(&["jobs", "cancel", &job, "--profile", "supply"])
        .await;
    assert_failure(&value, exit, 6, "INTENT_TERMINAL");
    let (value, exit) = h
        .run(&["jobs", "result", &job, "--profile", "supply"])
        .await;
    assert_failure(&value, exit, 7, "PARTIAL_RESULT");
    let (done, _) = h
        .run(&[
            "jobs",
            "submit",
            BATCH,
            "--profile",
            "supply",
            "--params",
            r#"{"itemName":"白芝麻"}"#,
        ])
        .await;
    let done = done["data"]["jobId"].as_str().unwrap().to_owned();
    iam.with(|f| f.jobs.insert(done.clone(), "succeeded".into()));
    let (value, exit) = h
        .run(&["jobs", "result", &done, "--profile", "supply"])
        .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["data"]["complete"], true);
    assert_eq!(value["data"]["items"][0]["minOrderNum"], "1");
    let unknown = "U".repeat(43);
    let (value, exit) = h
        .run(&["jobs", "status", &unknown, "--profile", "supply"])
        .await;
    assert_failure(&value, exit, 6, "RESOURCE_UNAVAILABLE");
}

// A2 隔离：按系统、环境、发行方和父授权隔离凭证与目录；篡改的目录拒绝使用。
#[tokio::test]
async fn complete_batch_results_up_to_the_ten_mib_contract_are_downloadable() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let job = "J".repeat(43);
    let rows = vec![items()[0].clone(); 10_000];
    let payload = ok(
        "job.result",
        json!({"jobId": job, "state": "succeeded", "complete": true, "items": rows}),
    );
    let bytes = serde_json::to_vec(&payload).unwrap().len();
    assert!(bytes > 2 * 1024 * 1024 && bytes < 10 * 1024 * 1024);
    iam.with(|f| {
        f.jobs.insert(job.clone(), "succeeded".into());
        f.overrides.insert("jobs.result", reply(200, payload));
    });
    let (value, exit) = h
        .run(&["jobs", "result", &job, "--profile", "supply"])
        .await;
    assert_eq!(exit, 0, "{}", value["error"]);
    assert_eq!(value["data"]["complete"], true);
    assert_eq!(value["data"]["items"].as_array().unwrap().len(), 10_000);

    let oversized = vec![items()[0].clone(); 48_000];
    let payload = ok(
        "job.result",
        json!({"jobId": job, "state": "succeeded", "complete": true, "items": oversized}),
    );
    assert!(serde_json::to_vec(&payload).unwrap().len() > 10 * 1024 * 1024 + 64 * 1024);
    iam.with(|f| {
        f.overrides.insert("jobs.result", reply(200, payload));
    });
    let (value, exit) = h
        .run(&["jobs", "result", &job, "--profile", "supply"])
        .await;
    assert_failure(&value, exit, 5, "DEPENDENCY_UNAVAILABLE");
    // Other governed responses keep the 2 MiB bound.
    let big_status = ok(
        "job.status",
        json!({"jobId": job, "state": "succeeded", "padding": "x".repeat(2 * 1024 * 1024)}),
    );
    iam.with(|f| {
        f.overrides.insert("jobs.status", reply(200, big_status));
    });
    let (value, exit) = h
        .run(&["jobs", "status", &job, "--profile", "supply"])
        .await;
    assert_failure(&value, exit, 5, "DEPENDENCY_UNAVAILABLE");
}
