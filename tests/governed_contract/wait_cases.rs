use super::*;

#[tokio::test]
async fn wait_downloads_succeeded_job_once_and_does_not_submit_again() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (job, exit) = h
        .run(&[
            "jobs",
            "submit",
            BATCH,
            "--profile",
            "supply",
            "--params",
            r#"{"itemCode":"1"}"#,
        ])
        .await;
    assert_eq!(exit, 0);
    let id = job["data"]["jobId"].as_str().unwrap();
    iam.with(|f| {
        f.jobs.insert(id.into(), "succeeded".into());
    });
    let start = iam.count();
    let (value, exit) = h
        .run(&["jobs", "wait", id, "--profile", "supply", "--timeout", "1"])
        .await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["data"]["complete"], true);
    let paths = iam.paths_since(start);
    assert_eq!(paths.len(), 2);
    assert_eq!(paths.iter().filter(|p| p.ends_with("/result")).count(), 1);
    assert!(
        iam.since(start)
            .iter()
            .all(|r| r.method == "GET" && r.path.starts_with("/cli-api/v1/jobs/"))
    );
}

#[tokio::test]
async fn wait_timeout_keeps_job_and_respects_retry_after_without_cancel() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let id = "J".repeat(43);
    iam.with(|f| {
        f.jobs.insert(id.clone(), "running".into());
        f.overrides.insert(
            "jobs.status",
            (
                200,
                vec![("Retry-After".into(), "20".into())],
                serde_json::to_vec(&ok("jobs.status", json!({"jobId":id,"state":"running"})))
                    .unwrap(),
            ),
        );
    });
    let start = iam.count();
    let (value, exit) = h
        .run(&["jobs", "wait", &id, "--profile", "supply", "--timeout", "1"])
        .await;
    assert_eq!(exit, 7, "{value}");
    assert_eq!(value["error"]["code"], "JOB_WAIT_TIMEOUT");
    assert_eq!(value["data"]["jobId"], id);
    assert_eq!(value["data"]["state"], "running");
    assert_eq!(iam.paths_since(start).len(), 1);
    assert_eq!(iam.with(|f| f.jobs[&id].clone()), "running");
}

#[tokio::test]
async fn wait_terminal_and_download_failures_return_original_id_without_resubmit() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let id = "K".repeat(43);
    for state in ["failed", "cancelled", "succeeded"] {
        iam.with(|f| {
            f.jobs.insert(id.clone(), state.into());
            if state == "succeeded" {
                f.overrides
                    .insert("jobs.result", failure(403, "SCOPE_DENIED"));
            }
        });
        let start = iam.count();
        let (value, exit) = h
            .run(&["jobs", "wait", &id, "--profile", "supply", "--timeout", "1"])
            .await;
        assert_ne!(exit, 0, "{value}");
        assert_eq!(value["data"]["jobId"], id);
        assert_eq!(value["meta"]["complete"], false);
        assert!(
            iam.since(start)
                .iter()
                .all(|r| r.method == "GET" && r.path.starts_with("/cli-api/v1/jobs/"))
        );
    }
}

#[cfg(unix)]
#[tokio::test]
async fn cancelling_wait_in_a_real_process_never_cancels_the_remote_job() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let id = "W".repeat(43);
    iam.with(|f| {
        f.jobs.insert(id.clone(), "running".into());
    });
    let profile: Value =
        serde_json::from_slice(&std::fs::read(h.file("supply.profile.json")).unwrap()).unwrap();
    let credentials = h.store.get(profile["credentialKey"].as_str().unwrap());
    let start = iam.count();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "wait_cases::wait_cancellation_child",
            "--ignored",
            "--nocapture",
        ])
        .env("DT_TEST_WAIT_ROOT", h.root.path())
        .env("DT_TEST_WAIT_ORIGIN", &iam.origin)
        .env(
            "DT_TEST_WAIT_CREDENTIALS",
            serde_json::to_string(&credentials).unwrap(),
        )
        .env("DT_TEST_WAIT_ID", &id)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while iam.count() == start && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(iam.count() > start, "child did not observe job");
    assert!(
        std::process::Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
        panic!("wait child did not stop");
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let value: Value = text
        .lines()
        .find_map(|line| serde_json::from_str::<Value>(line).ok())
        .unwrap();
    assert_eq!(value["error"]["code"], "WAIT_CANCELLED");
    assert_eq!(value["data"]["jobId"], id);
    assert_eq!(iam.with(|f| f.jobs[&id].clone()), "running");
    assert!(iam.since(start).iter().all(|r| r.method == "GET"));
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "started by cancelling_wait_in_a_real_process_never_cancels_the_remote_job"]
async fn wait_cancellation_child() {
    let root = std::path::PathBuf::from(std::env::var("DT_TEST_WAIT_ROOT").unwrap());
    let origin = std::env::var("DT_TEST_WAIT_ORIGIN").unwrap();
    let id = std::env::var("DT_TEST_WAIT_ID").unwrap();
    let profile: Value =
        serde_json::from_slice(&std::fs::read(root.join("governed/supply.profile.json")).unwrap())
            .unwrap();
    let credentials: Credentials =
        serde_json::from_str(&std::env::var("DT_TEST_WAIT_CREDENTIALS").unwrap()).unwrap();
    let store = MemoryStore::default();
    store
        .write(profile["credentialKey"].as_str().unwrap(), &credentials)
        .unwrap();
    let rt = Runtime {
        root,
        environments: BTreeMap::from([(ENV.into(), environment(&origin))]),
        store: Box::new(store),
        browser: Box::new(NoBrowser),
        interactive: false,
        aggregate_budget: Duration::from_secs(30),
    };
    let (value, exit, _) = dt_cli::execute(
        &rt,
        [
            "dt-cli",
            "jobs",
            "wait",
            &id,
            "--profile",
            "supply",
            "--timeout",
            "60",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    )
    .await;
    println!("{value}");
    assert_eq!(exit, 8);
}
