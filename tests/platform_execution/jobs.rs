use super::support::Harness;
use serde_json::Value;

#[derive(serde::Deserialize)]
struct BudgetJobResult {
    /// 原生 CLI 返回的已保存分页结果。
    page: BudgetStoredPage,
}

#[derive(serde::Deserialize)]
struct BudgetStoredPage {
    /// 本页实际收到的业务执行结果。
    result: BudgetExecutionResult,
}

#[derive(serde::Deserialize)]
struct BudgetExecutionResult {
    /// 未经截断的业务响应原文。
    body: String,
}

#[derive(serde::Deserialize)]
struct BudgetReply {
    /// 隔离上游返回的本页业务数据。
    data: BudgetPage,
}

#[derive(serde::Deserialize)]
struct BudgetPage {
    /// 按固定页大小读取的有序记录。
    rows: Vec<BudgetRow>,
}

#[derive(serde::Deserialize)]
struct BudgetRow {
    /// 夹具中从 1 开始的唯一记录标识。
    id: u64,
}

fn job_id(data: &Value) -> String {
    data["jobId"].as_str().unwrap().to_owned()
}
async fn interrupt_second_page(h: &Harness, input: &str) -> String {
    let before = h.stats("status").await["page2"].as_u64().unwrap();
    let stdout = tempfile::NamedTempFile::new_in(&h.root).unwrap();
    let stderr = tempfile::NamedTempFile::new_in(&h.root).unwrap();
    let mut child = h
        .command(&[
            "jobs",
            "start",
            "--profile",
            "fixture",
            "--operation",
            "fixture-page-orders",
            "--version",
            "1",
            "--input-file",
            input,
        ])
        .stdout(stdout.reopen().unwrap())
        .stderr(stderr.reopen().unwrap())
        .spawn()
        .unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if h.stats("status").await["page2"].as_u64().unwrap() > before {
            break;
        }
        if child.try_wait().unwrap().is_some() {
            panic!(
                "读取子进程提前结束: {}",
                std::fs::read_to_string(stdout.path()).unwrap()
            );
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "未观察到第二页实际到达上游"
        );
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    child.kill().unwrap();
    child.wait().unwrap();
    h.stats("release").await;
    let list = h.ok(&["jobs", "list"]);
    let id = list["items"][0]["jobId"].as_str().unwrap().to_owned();
    let status = h.ok(&["jobs", "status", "--id", &id]);
    assert_eq!(status["state"], "paused");
    assert_eq!(status["progress"]["pages"], 1);
    let first = h.ok(&["jobs", "results", "--id", &id, "--page", "1"]);
    assert_eq!(first["checkpointCommitted"], true);
    assert!(
        first["page"]["result"]["body"]
            .as_str()
            .unwrap()
            .contains("untouchedField")
    );
    id
}
pub async fn run(h: &Harness) {
    let input = h.root.join("paging.json");
    std::fs::write(&input, "{}").unwrap();
    let input = input.to_str().unwrap();
    let complete = h.ok(&[
        "jobs",
        "start",
        "--profile",
        "fixture",
        "--operation",
        "fixture-budget-pages",
        "--version",
        "1",
        "--input-file",
        input,
    ]);
    assert_eq!(complete["complete"], true);
    assert_eq!(complete["progress"]["records"], 150);
    let id = job_id(&complete);
    let mut ids = Vec::new();
    for page in ["1", "2"] {
        let result: BudgetJobResult =
            serde_json::from_value(h.ok(&["jobs", "results", "--id", &id, "--page", page]))
                .unwrap();
        let body: BudgetReply = serde_json::from_str(&result.page.result.body).unwrap();
        ids.extend(body.data.rows.into_iter().map(|row| row.id));
    }
    assert_eq!(ids, (1..=150).collect::<Vec<u64>>());
    let limited = h.ok(&[
        "jobs",
        "start",
        "--profile",
        "fixture",
        "--operation",
        "fixture-budget-limited",
        "--version",
        "1",
        "--input-file",
        input,
    ]);
    assert_eq!(limited["complete"], false);
    assert_eq!(limited["progress"]["records"], 100);
    assert_eq!(limited["progress"]["pages"], 1);
    let uncommitted = h.ok(&["jobs", "results", "--id", &job_id(&limited), "--page", "2"]);
    assert_eq!(uncommitted["checkpointCommitted"], false);
    for (operation, reason, records, pages) in [
        ("fixture-budget-byte", "RECORD_OR_BYTE_LIMIT", 0, 0),
        ("fixture-budget-time", "TIME_LIMIT", 100, 1),
    ] {
        let result = h.ok(&[
            "jobs",
            "start",
            "--profile",
            "fixture",
            "--operation",
            operation,
            "--version",
            "1",
            "--input-file",
            input,
        ]);
        assert_eq!(result["complete"], false);
        assert_eq!(result["reason"], reason, "{operation}");
        assert_eq!(result["state"], "incomplete", "{operation}");
        assert_eq!(result["progress"]["records"], records);
        assert_eq!(result["progress"]["pages"], pages);
    }
    for (operation, records, pages, complete) in [
        ("fixture-budget-empty", 0, 1, true),
        ("fixture-budget-exact", 100, 1, true),
        ("fixture-budget-exact-no-total", 100, 2, true),
        ("fixture-budget-short-no-total", 150, 2, true),
        ("fixture-budget-exact-limit-no-total", 100, 1, false),
    ] {
        let result = h.ok(&[
            "jobs",
            "start",
            "--profile",
            "fixture",
            "--operation",
            operation,
            "--version",
            "1",
            "--input-file",
            input,
        ]);
        assert_eq!(result["progress"]["records"], records, "{operation}");
        assert_eq!(result["progress"]["pages"], pages, "{operation}");
        assert_eq!(result["complete"], complete, "{operation}");
    }
    let before = h.stats("status").await;
    let id = interrupt_second_page(h, input).await;
    h.login("renewed").await;
    let resumed = h.ok(&["jobs", "resume", "--profile", "renewed", "--id", &id]);
    assert_eq!(resumed["state"], "succeeded");
    assert_eq!(resumed["complete"], true);
    assert_eq!(resumed["progress"]["records"], 5);
    assert_eq!(resumed["pointInTimeSnapshot"], false);
    let after = h.stats("status").await;
    assert_eq!(
        after["page1"].as_u64().unwrap() - before["page1"].as_u64().unwrap(),
        1
    );
    assert_eq!(
        after["page2"].as_u64().unwrap() - before["page2"].as_u64().unwrap(),
        2
    );
    let preparation_before = h.stats("preparation").await;
    let page_before = h.stats("status").await;
    let mut samples = Vec::new();
    for (operation, body) in [
        (
            "fixture-json-pages",
            format!(
                "{{\"bodyJson\":{}}}",
                serde_json::to_string(super::support::BODY).unwrap()
            ),
        ),
        (
            "fixture-form-pages",
            "{\"form\":[{\"name\":\"amount\",\"value\":123.0000000000000000000000001}]}".into(),
        ),
        ("fixture-cursor-pages", "{}".into()),
    ] {
        let path = h.root.join(format!("{operation}.json"));
        std::fs::write(&path, body).unwrap();
        let result = h.ok(&[
            "jobs",
            "start",
            "--profile",
            "fixture",
            "--operation",
            operation,
            "--version",
            "1",
            "--input-file",
            path.to_str().unwrap(),
        ]);
        assert_eq!(result["complete"], true, "{result}");
        assert_eq!(result["progress"]["pages"], 3);
        assert_eq!(result["progress"]["records"], 5);
        let connections = h.connections(operation).await;
        assert_eq!(connections["requests"], 3, "{operation}: {connections}");
        assert_eq!(connections["connections"], 1, "{operation}: {connections}");
        samples.push(
            serde_json::json!({"operation":operation,"pages":3,"records":5,
            "gateway":connections}),
        );
    }
    let preparation_after = h.stats("preparation").await;
    let page_after = h.stats("status").await;
    // 三种分页任务各3页：每任务仅1契约+1会话准备+1运行地址，后续页只调用Gateway。
    for field in ["contract", "runtime", "me"] {
        assert_eq!(
            preparation_after[field].as_u64().unwrap()
                - preparation_before[field].as_u64().unwrap(),
            3,
            "{field}"
        );
    }
    if let Ok(path) = std::env::var("DT_CLI_AFTER_REPORT") {
        let report = serde_json::json!({"cliVersion":"0.6.0","samples":samples,
            "contract":3,"me":3,"runtime":3,"scope":"isolated local HTTP; one process per task"});
        std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    assert_eq!(
        page_after["page1"].as_u64().unwrap() - page_before["page1"].as_u64().unwrap(),
        2
    );
    assert_eq!(
        page_after["page2"].as_u64().unwrap() - page_before["page2"].as_u64().unwrap(),
        2
    );
    assert_eq!(
        page_after["page3"].as_u64().unwrap() - page_before["page3"].as_u64().unwrap(),
        2
    );
    let limited = h.ok(&[
        "jobs",
        "start",
        "--profile",
        "fixture",
        "--operation",
        "fixture-limited-orders",
        "--version",
        "1",
        "--input-file",
        input,
    ]);
    assert_eq!(limited["state"], "incomplete");
    assert_eq!(limited["reason"], "PAGE_LIMIT");
    assert_eq!(limited["complete"], false);
    let looped = h.ok(&[
        "jobs",
        "start",
        "--profile",
        "fixture",
        "--operation",
        "fixture-cursor-loop",
        "--version",
        "1",
        "--input-file",
        input,
    ]);
    assert_eq!(looped["state"], "incomplete");
    assert_eq!(looped["progress"]["pages"], 1);
    h.stats("block-next").await;
    let paused = interrupt_second_page(h, input).await;
    let original = h.stats("status").await;
    h.grant(h.case("fixture-page-orders"), false).await;
    let rejected = h.ok(&["jobs", "resume", "--profile", "fixture", "--id", &paused]);
    assert_eq!(rejected["state"], "paused");
    assert_eq!(h.stats("status").await["page2"], original["page2"]);
    let preserved = h.ok(&["jobs", "results", "--id", &paused, "--page", "1"]);
    assert_eq!(preserved["checkpointCommitted"], true);
    h.grant(h.case("fixture-page-orders"), true).await;
    let path = h.root.join(format!("platform/jobs/{paused}.json"));
    let bytes = std::fs::read(&path).unwrap();
    let mut modified: Value = serde_json::from_slice(&bytes).unwrap();
    modified["record"]["profile"]["subjectId"] = Value::String("2048".into());
    std::fs::write(&path, serde_json::to_vec(&modified).unwrap()).unwrap();
    let (denied, code) = h.run(&["jobs", "resume", "--profile", "fixture", "--id", &paused]);
    assert_ne!(code, 0);
    assert_eq!(denied["error"]["code"], "LOCAL_STATE_UNAVAILABLE");
    std::fs::write(&path, bytes).unwrap();
    h.login_as("switched", 'F', "3072").await;
    let (switched, code) = h.run(&["jobs", "resume", "--profile", "switched", "--id", &paused]);
    assert_ne!(code, 0);
    assert_eq!(switched["error"]["code"], "TASK_CONTEXT_CHANGED");
    assert_eq!(h.stats("status").await["page2"], original["page2"]);
    h.login("other").await;
    let (wrong, code) = h.run(&["jobs", "resume", "--profile", "other", "--id", &paused]);
    assert_eq!(code, 0, "{wrong}");
    assert_eq!(wrong["data"]["complete"], true);
    assert_eq!(
        h.stats("status").await["page2"].as_u64().unwrap(),
        original["page2"].as_u64().unwrap() + 1
    );
    let actual = h.stats("status").await;
    assert!(
        actual["observations"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|o| o["path"] == "/execution/json-pages")
            .all(|o| o["body"]
                .as_str()
                .unwrap()
                .contains("123.0000000000000000000000001"))
    );
    assert!(!job_id(&limited).is_empty());
}
