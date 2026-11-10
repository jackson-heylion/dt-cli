//! 通过发行命令入口核对固定平台、默认正式环境及浏览器启动前的参数拒绝。
use dt_cli::{Runtime, credentials::FileStore, login::Browser, output::Result};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
struct OpenedTarget {
    /// 系统浏览器收到的平台入口，不保存授权查询串。
    origin: String,
    /// 授权请求绑定的运行环境。
    environment: String,
    /// 本人业务或管理会话的授权路径。
    path: String,
}

#[derive(Default)]
struct BrowserCapture {
    /// 系统浏览器打开调用次数。
    calls: AtomicUsize,
    /// 尚未请求浏览器时为空；仅保存公开入口与环境，不含授权参数。
    target: Mutex<Option<OpenedTarget>>,
}

struct CapturingBrowser {
    /// 测试观察结果，不启动真实桌面浏览器。
    capture: Arc<BrowserCapture>,
}

impl Browser for CapturingBrowser {
    fn open(&self, address: &str) -> Result<()> {
        let url = url::Url::parse(address).unwrap();
        self.capture.calls.fetch_add(1, Ordering::Relaxed);
        *self.capture.target.lock().unwrap() = Some(OpenedTarget {
            origin: url.origin().ascii_serialization(),
            environment: url
                .query_pairs()
                .find(|(name, _)| name == "environment")
                .unwrap()
                .1
                .into_owned(),
            path: url.path().to_owned(),
        });
        Ok(())
    }
}

fn runtime(root: &std::path::Path, capture: Arc<BrowserCapture>) -> Runtime {
    Runtime {
        root: root.to_owned(),
        store: Box::new(FileStore::new(root)),
        browser: Box::new(CapturingBrowser { capture }),
        interactive: false,
        aggregate_budget: std::time::Duration::from_secs(30),
    }
}

fn arguments(management: bool, environment: Option<&str>, origin: Option<&str>) -> Vec<String> {
    let mut args = vec!["dt-cli"];
    if management {
        args.push("manage");
    }
    args.extend(["login", "--profile", "origin-fixture"]);
    if let Some(environment) = environment {
        args.extend(["--environment", environment]);
    }
    if let Some(origin) = origin {
        args.extend(["--origin", origin]);
    }
    args.into_iter().map(str::to_owned).collect()
}

#[tokio::test]
async fn platform_login_rejects_origins_before_browser_or_network() {
    for management in [false, true] {
        for origin in [
            "http://gateway.example.invalid",
            "https://gateway.example.invalid/path",
            "https://gateway.example.invalid/a/..",
            "https://gateway.example.invalid/",
            "https://@gateway.example.invalid",
            "https://user@gateway.example.invalid",
            "https://gateway.example.invalid?query=1",
            "https://gateway.example.invalid#fragment",
            " https://gateway.example.invalid",
            "https://gate\tway.example.invalid",
            "https://gateway.example.invalid:99999",
        ] {
            let root = tempfile::tempdir().unwrap();
            let capture = Arc::new(BrowserCapture::default());
            let rt = runtime(root.path(), capture.clone());
            let (reply, exit, _) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                dt_cli::execute(&rt, arguments(management, Some("prod"), Some(origin))),
            )
            .await
            .expect("非法入口进入浏览器或网络等待");
            assert_eq!(exit, 2, "{reply}");
            assert_eq!(reply["error"]["code"], "INVALID_ARGUMENT");
            assert_eq!(capture.calls.load(Ordering::Relaxed), 0);
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }
}

#[tokio::test]
async fn valid_https_origins_cannot_override_application_endpoints() {
    for management in [false, true] {
        for (environment, origin) in [
            (None, "https://iadmin.jmj1995.com"),
            (None, "https://open-platform-stg.datousoft.com"),
            (Some("prod"), "https://gateway.jmj1995.com"),
            (Some("prod"), "https://open-platform-stg.datousoft.com"),
            (Some("stg"), "https://open-platform.jmj1995.com"),
            (Some("stg"), "https://iadmin.jmj1995.com"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let capture = Arc::new(BrowserCapture::default());
            let rt = runtime(root.path(), capture.clone());
            let (reply, exit, _) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                dt_cli::execute(&rt, arguments(management, environment, Some(origin))),
            )
            .await
            .expect("其他站点进入浏览器或网络等待");
            assert_eq!(exit, 2, "{reply}");
            assert_eq!(
                reply["error"]["code"],
                if cfg!(feature = "local-dev") {
                    "PLATFORM_ORIGIN_MISMATCH"
                } else {
                    "INVALID_ARGUMENT"
                }
            );
            assert_eq!(reply["error"]["retryable"], false);
            assert_eq!(capture.calls.load(Ordering::Relaxed), 0);
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }
}

#[tokio::test(start_paused = true)]
async fn application_selects_fixed_endpoint_and_defaults_to_production() {
    for management in [false, true] {
        for (environment, origin, expected_environment, expected_origin) in [
            (None, None, "prod", "https://open-platform.jmj1995.com"),
            (
                Some("prod"),
                None,
                "prod",
                "https://open-platform.jmj1995.com",
            ),
            (
                Some("stg"),
                None,
                "stg",
                "https://open-platform-stg.datousoft.com",
            ),
            (
                None,
                Some("https://open-platform.jmj1995.com"),
                "prod",
                "https://open-platform.jmj1995.com",
            ),
            (
                Some("stg"),
                Some("https://open-platform-stg.datousoft.com"),
                "stg",
                "https://open-platform-stg.datousoft.com",
            ),
        ] {
            if origin.is_some() && !cfg!(feature = "local-dev") {
                continue;
            }
            let root = tempfile::tempdir().unwrap();
            let capture = Arc::new(BrowserCapture::default());
            let rt = runtime(root.path(), capture.clone());
            let (reply, exit, _) =
                dt_cli::execute(&rt, arguments(management, environment, origin)).await;
            assert_eq!(exit, 5, "{reply}");
            assert_eq!(reply["error"]["code"], "TIMEOUT");
            assert_eq!(capture.calls.load(Ordering::Relaxed), 1);
            let opened = capture.target.lock().unwrap().clone().unwrap();
            assert_eq!(opened.origin, expected_origin);
            assert_eq!(opened.environment, expected_environment);
            assert_eq!(
                opened.path,
                if management {
                    "/platform/management/oauth2/authorize"
                } else {
                    "/platform/cli/oauth2/authorize"
                }
            );
        }
    }
}

#[tokio::test]
async fn unsupported_environment_is_rejected_before_local_storage_or_browser() {
    for management in [false, true] {
        for environment in ["fixture", "local", "production", "stage"] {
            let root = tempfile::tempdir().unwrap();
            let capture = Arc::new(BrowserCapture::default());
            let rt = runtime(root.path(), capture.clone());
            let (reply, exit, _) =
                dt_cli::execute(&rt, arguments(management, Some(environment), None)).await;
            assert_eq!(exit, 2, "{reply}");
            assert_eq!(reply["error"]["code"], "PLATFORM_ENVIRONMENT_UNSUPPORTED");
            assert_eq!(capture.calls.load(Ordering::Relaxed), 0);
            assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        }
    }
}

#[tokio::test]
async fn public_help_only_requires_environment_selection() {
    let root = tempfile::tempdir().unwrap();
    let capture = Arc::new(BrowserCapture::default());
    let rt = runtime(root.path(), capture.clone());
    for args in [
        vec!["dt-cli", "login", "--help"],
        vec!["dt-cli", "manage", "--help"],
    ] {
        let (reply, exit, _) =
            dt_cli::execute(&rt, args.into_iter().map(str::to_owned).collect()).await;
        assert_eq!(exit, 0, "{reply}");
        let commands = reply["data"]["commands"].to_string();
        assert!(commands.contains("--environment"));
        assert!(commands.contains("prod|stg"));
        assert!(!commands.contains("--origin"));
    }
    assert_eq!(capture.calls.load(Ordering::Relaxed), 0);
}
