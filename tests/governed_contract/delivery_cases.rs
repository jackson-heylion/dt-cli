use super::*;

#[tokio::test]
async fn read_waits_on_explicit_rate_limit_and_submit_remains_single_dispatch() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let start = iam.count();
    iam.with(|f| f.rate_limit_once = Some("invoke"));
    let (value, code) = h
        .run(&[
            "api",
            "call",
            READ,
            "--profile",
            "supply",
            "--params",
            READ_PARAMS,
        ])
        .await;
    assert_eq!(code, 0, "{value}");
    assert_eq!(
        iam.since(start)
            .iter()
            .filter(|r| r.path.ends_with("/invoke"))
            .count(),
        2
    );
    iam.with(|f| f.rate_limit_once = Some("jobs.submit"));
    let start = iam.count();
    let (value, code) = h
        .run(&[
            "jobs",
            "submit",
            BATCH,
            "--profile",
            "supply",
            "--params",
            r#"{"itemCode":"101010009"}"#,
        ])
        .await;
    assert_failure(&value, code, 5, "RATE_LIMITED");
    assert_eq!(
        iam.since(start)
            .iter()
            .filter(|r| r.path == "/cli-api/v1/jobs")
            .count(),
        1
    );
}

#[tokio::test]
async fn profile_center_applies_only_to_supported_contract_and_explicit_center_wins() {
    let iam = supply();
    iam.with(|f| {
        for o in &mut f.operations {
            if o["operationId"] == READ || o["operationId"] == BATCH {
                o["inputSchema"]["properties"]["deliveryCenterId"] =
                    json!({"type":"integer","minimum":1});
            }
        }
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let start = iam.count();
    let (v, c) = h
        .run(&[
            "profiles",
            "delivery-center",
            "--profile",
            "supply",
            "--id",
            "117",
        ])
        .await;
    assert_eq!(c, 0, "{v}");
    assert_eq!(iam.count(), start);
    let (v, c) = h
        .run(&[
            "api",
            "call",
            READ,
            "--profile",
            "supply",
            "--params",
            READ_PARAMS,
        ])
        .await;
    assert_eq!(c, 0, "{v}");
    let last = iam
        .seen()
        .into_iter()
        .rev()
        .find(|r| r.path.ends_with("/invoke"))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&last.body).unwrap()["arguments"]["deliveryCenterId"],
        117
    );
    let params = r#"{"shopCode":"2021003","itemCode":"101010009","deliveryCenterId":119}"#;
    let (v, c) = h
        .run(&[
            "api",
            "call",
            READ,
            "--profile",
            "supply",
            "--params",
            params,
        ])
        .await;
    assert_eq!(c, 0, "{v}");
    let last = iam
        .seen()
        .into_iter()
        .rev()
        .find(|r| r.path.ends_with("/invoke"))
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&last.body).unwrap()["arguments"]["deliveryCenterId"],
        119
    );
}

#[tokio::test]
async fn old_contract_ignores_profile_center_and_result_download_retries_original_job() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (_, c) = h
        .run(&[
            "profiles",
            "delivery-center",
            "--profile",
            "supply",
            "--id",
            "117",
        ])
        .await;
    assert_eq!(c, 0);
    let (v, c) = h
        .run(&[
            "api",
            "call",
            READ,
            "--profile",
            "supply",
            "--params",
            READ_PARAMS,
        ])
        .await;
    assert_eq!(c, 0, "{v}");
    let last = iam
        .seen()
        .into_iter()
        .rev()
        .find(|r| r.path.ends_with("/invoke"))
        .unwrap();
    assert!(
        serde_json::from_str::<Value>(&last.body).unwrap()["arguments"]
            .get("deliveryCenterId")
            .is_none()
    );
    let (v, c) = h
        .run(&[
            "jobs",
            "submit",
            BATCH,
            "--profile",
            "supply",
            "--params",
            r#"{"itemCode":"101010009"}"#,
        ])
        .await;
    assert_eq!(c, 0, "{v}");
    let id = v["data"]["jobId"].as_str().unwrap();
    iam.with(|f| {
        f.jobs.insert(id.into(), "succeeded".into());
        f.rate_limit_once = Some("jobs.result");
    });
    let start = iam.count();
    let (v, c) = h.run(&["jobs", "result", id, "--profile", "supply"]).await;
    assert_eq!(c, 0, "{v}");
    assert_eq!(
        iam.since(start)
            .iter()
            .filter(|r| r.path.ends_with("/result"))
            .count(),
        2
    );
    assert_eq!(
        iam.since(start)
            .iter()
            .filter(|r| r.path == "/cli-api/v1/jobs")
            .count(),
        0
    );
}
