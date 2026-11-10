//! 由开放平台 HTTP 测试启动；仅替代操作系统浏览器，真实平台负责会话。
#![cfg(feature = "local-dev")]
use dt_cli::{Runtime, credentials::FileStore};
use serde::Deserialize;

#[path = "common/platform_browser.rs"]
mod browser_fixture;
use browser_fixture::PlatformBrowser;
#[test]
#[ignore = "由开放平台 PlatformSessionHttpTest 传入隔离 HTTP 服务地址"]
fn platform_flow_child() {
    let platform = std::env::var("DT_PLATFORM_HTTP_FIXTURE_ORIGIN").unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let rt = Runtime {
            root: root.path().into(),
            store: Box::new(FileStore::new(root.path())),
            browser: Box::new(PlatformBrowser),
            interactive: false,
            aggregate_budget: std::time::Duration::from_secs(30),
        };
        let run = |args: Vec<String>| dt_cli::execute(&rt, args);
        let (logged, status, _) = run([
            "dt-cli",
            "login",
            "--profile",
            "fixture",
            "--origin",
            &platform,
            "--environment",
            "fixture",
            "--system",
            "supply-chain-server",
        ]
        .map(str::to_owned)
        .to_vec())
        .await;
        assert_eq!(status, 0, "{logged}");
        assert_eq!(logged["data"]["subjectId"], "1024");
        let id = logged["data"]["sessionId"].as_str().unwrap();
        assert!(!logged.to_string().contains("dtp_cli_at_"));
        assert!(!logged.to_string().contains("dtp_cli_rt_"));
        let (checked, status, _) = run(["dt-cli", "check", "--profile", "fixture"]
            .map(str::to_owned)
            .to_vec())
        .await;
        assert_eq!(status, 0, "{checked}");
        let (list, status, _) = run(["dt-cli", "sessions", "--profile", "fixture"]
            .map(str::to_owned)
            .to_vec())
        .await;
        assert_eq!(status, 0, "{list}");
        assert!(list["data"]["total"].as_u64().unwrap() >= 1);
        if std::env::var_os("DT_PLATFORM_HTTP_GATEWAY_FIXTURE").is_some() {
            let (catalog, status, _) = run(["dt-cli", "catalog", "--profile", "fixture"]
                .map(str::to_owned)
                .to_vec())
            .await;
            assert_eq!(status, 0, "{catalog}");
            assert!(catalog["data"]["total"].as_u64().unwrap() >= 1);
            #[derive(Deserialize)]
            #[serde(rename_all = "camelCase")]
            struct GatewayCase {
                /// 平台生成的稳定接口标识。
                operation_id: String,
                /// 当前文档样例的实际路径。
                path: String,
                /// 固定执行版本。
                version: u64,
            }
            let cases: Vec<GatewayCase> = match std::env::var("DT_PLATFORM_HTTP_GATEWAY_CASES") {
                Ok(raw) => serde_json::from_str(&raw).unwrap(),
                Err(_) => vec![GatewayCase {
                    operation_id: "fixture-read-orders".into(),
                    path: "/orders/123".into(),
                    version: 1,
                }],
            };
            for case in cases {
                let (called, status, _) = run([
                    "dt-cli",
                    "platform",
                    "call",
                    "--profile",
                    "fixture",
                    "--operation",
                    &case.operation_id,
                    "--version",
                    &case.version.to_string(),
                    "--path",
                    &case.path,
                    "--query",
                    "q=a%20b",
                ]
                .map(str::to_owned)
                .to_vec())
                .await;
                assert_eq!(status, 0, "{called}");
                assert_eq!(called["data"]["businessSuccess"], true);
                assert_eq!(
                    called["data"]["body"],
                    "{\"code\":0,\"data\":{\"untouched\":[1,2],\"nullable\":null}}"
                );
                assert!(!called.to_string().contains("dtp_cli_at_"));
            }
        }
        let (revoked, status, _) =
            run(
                ["dt-cli", "revoke", "--profile", "fixture", "--session", id]
                    .map(str::to_owned)
                    .to_vec(),
            )
            .await;
        assert_eq!(status, 0, "{revoked}");
        let (denied, status, _) = run(["dt-cli", "check", "--profile", "fixture"]
            .map(str::to_owned)
            .to_vec())
        .await;
        assert_eq!(status, 4, "{denied}");
        let (logout, status, _) = run(["dt-cli", "logout", "--profile", "fixture"]
            .map(str::to_owned)
            .to_vec())
        .await;
        assert_eq!(status, 0, "{logout}");
        assert_eq!(logout["data"]["localCleanup"], "confirmed");
    });
}

#[test]
fn native_platform_help_and_unknown_input_keep_output_safe() {
    let binary = env!("CARGO_BIN_EXE_dt-cli");
    let help = std::process::Command::new(binary)
        .args(["--help"])
        .output()
        .unwrap();
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("platform login"));
    let invalid = std::process::Command::new(binary)
        .args([
            "platform",
            "check",
            "--profile",
            "x",
            "--token",
            "synthetic-sensitive-value",
        ])
        .output()
        .unwrap();
    assert!(!invalid.status.success());
    assert!(!String::from_utf8_lossy(&invalid.stdout).contains("synthetic-sensitive-value"));
    assert!(!String::from_utf8_lossy(&invalid.stderr).contains("synthetic-sensitive-value"));
}
