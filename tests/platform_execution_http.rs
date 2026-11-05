//! 真正启动原生dt-cli二进制，使用真实平台与Gateway的隔离HTTP验收。
#![cfg(feature = "local-dev")]
#[path = "platform_execution/before.rs"]
mod before;
#[path = "common/platform_browser.rs"]
mod browser_fixture;
#[path = "platform_execution/jobs.rs"]
mod jobs;
#[path = "platform_execution/performance.rs"]
mod performance;
#[path = "platform_execution/support.rs"]
mod support;
#[path = "platform_execution/writes.rs"]
mod writes;

#[test]
#[ignore = "由PlatformExecutionHttpTest和实际Gateway进程提供隔离服务地址"]
fn platform_execution_child() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let folder = tempfile::tempdir().unwrap();
        let harness = support::Harness::new(folder.path());
        harness.login("fixture").await;
        let (missing, _) = harness.run(&[
            "writes",
            "inspect",
            "--id",
            "-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ]);
        assert_eq!(missing["error"]["code"], "WRITE_RECORD_NOT_FOUND");
        harness.login("fixture").await;
        assert_eq!(
            browser_fixture::OPENS.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        for case in harness
            .cases
            .iter()
            .filter(|case| case.operation_id.starts_with("fixture-method-"))
        {
            let input = harness.input(&format!("{}.json", case.operation_id), &case.media);
            let result = harness.ok(&[
                "call",
                "--profile",
                "fixture",
                "--operation",
                &case.operation_id,
                "--version",
                &case.version.to_string(),
                "--input-file",
                input.to_str().unwrap(),
            ]);
            assert_eq!(result["businessSuccess"], true, "{result}");
            assert_eq!(result["contractMatched"], true);
            assert_eq!(result["status"], 200);
            assert!(result["timing"]["clientElapsedMillis"].is_u64());
            assert!(result["timing"]["gateway"]["gatewayTotal"].is_number());
            assert!(result["invocationId"].is_string());
            if case.method == "HEAD" {
                assert_eq!(result["body"], "");
            } else {
                assert!(
                    result["body"]
                        .as_str()
                        .unwrap()
                        .contains("\"nullable\":null")
                );
            }
            assert!(case.path.contains("{id}"));
        }
        let denied_input = harness.input("denied.json", "NONE");
        let before = harness.stats("status").await;
        let (denied, code) = harness.run(&[
            "call",
            "--profile",
            "fixture",
            "--operation",
            "fixture-denied-other",
            "--version",
            "1",
            "--input-file",
            denied_input.to_str().unwrap(),
        ]);
        assert_ne!(code, 0);
        assert_eq!(denied["error"]["code"], "INTERFACE_PERMISSION_REQUIRED");
        assert_eq!(
            before["observations"],
            harness.stats("status").await["observations"]
        );
        let accounts = harness.ok(&["profiles"]);
        assert_eq!(accounts["online"], false);
        assert_eq!(accounts["items"][0]["subjectId"], "1024");
        assert!(accounts["items"][0].get("credentialKey").is_none());
        jobs::run(&harness).await;
        // 单个接口收权未影响另一条独立接口。
        let ordinary = harness.case("fixture-method-get");
        let input = harness.input("unaffected.json", "NONE");
        assert_eq!(
            harness.ok(&[
                "call",
                "--profile",
                "fixture",
                "--operation",
                &ordinary.operation_id,
                "--version",
                "1",
                "--input-file",
                input.to_str().unwrap()
            ])["businessSuccess"],
            true
        );
        writes::run(&harness).await;
        if let Ok(binary) = std::env::var("DT_CLI_BASELINE_BINARY") {
            before::run(&binary).await;
        }
        if std::env::var("DT_CLI_PERF_REPORT").is_ok() {
            performance::run().await;
        }
    });
}
