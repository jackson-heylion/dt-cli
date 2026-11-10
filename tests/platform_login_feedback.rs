//! 平台登录反馈通过公开命令核对；不打开真实浏览器或连接外部平台。
use dt_cli::{
    Runtime,
    credentials::FileStore,
    login::Browser,
    output::{Failure, Result},
};
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct BrowserProbe {
    /// 系统打开调用计数，用于确认默认登录和显式禁用行为。
    calls: Arc<AtomicUsize>,
    /// 是否模拟系统打开调用失败；成功模式也不声称窗口可见。
    fail: bool,
}
impl Browser for BrowserProbe {
    fn open(&self, _: &str) -> Result<()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.fail {
            Err(Failure::new(
                "BROWSER_OPEN_FAILED",
                6,
                "模拟浏览器未能启动。",
            ))
        } else {
            Ok(())
        }
    }
}

fn runtime(root: &std::path::Path, calls: Arc<AtomicUsize>, fail: bool) -> Runtime {
    Runtime {
        root: root.to_owned(),
        store: Box::new(FileStore::new(root)),
        browser: Box::new(BrowserProbe { calls, fail }),
        interactive: false,
        aggregate_budget: std::time::Duration::from_secs(30),
    }
}

fn arguments(management: bool, disabled: bool) -> Vec<String> {
    let mut args = vec!["dt-cli"];
    if management {
        args.push("manage");
    }
    args.extend([
        "login",
        "--profile",
        "feedback-fixture",
        "--environment",
        "prod",
    ]);
    if disabled {
        args.push("--no-browser");
    }
    args.into_iter().map(str::to_owned).collect()
}

#[tokio::test(start_paused = true)]
async fn timeout_preserves_launch_evidence_and_releases_local_resources() {
    for (fail, disabled, management, launch, expected_calls) in [
        (false, false, false, "requested", 1),
        (true, false, false, "failed", 1),
        (false, true, false, "disabled", 0),
        (false, false, true, "requested", 1),
    ] {
        let root = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let rt = runtime(root.path(), calls.clone(), fail);
        let (result, exit, _) = dt_cli::execute(&rt, arguments(management, disabled)).await;
        assert_eq!(exit, 5, "{result}");
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"]["code"], "TIMEOUT");
        assert_eq!(result["error"]["retryable"], false);
        let details = &result["error"]["details"];
        assert_eq!(details["stage"], "timed_out");
        assert_eq!(details["browserLaunch"], launch);
        assert_eq!(details["browserWindowVerified"], false);
        assert_eq!(details["callbackStatus"], "waiting");
        assert_eq!(details["waitSeconds"], 300);
        assert_eq!(calls.load(Ordering::Relaxed), expected_calls);
        let address: SocketAddr = details["callbackAddress"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!(address.ip().is_loopback());
        let listener = std::net::TcpListener::bind(address).unwrap();
        drop(listener);
        let directory = if management {
            "platform-management"
        } else {
            "platform"
        };
        assert_eq!(
            std::fs::read_dir(root.path().join(directory))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            std::fs::read_dir(root.path().join("credentials"))
                .unwrap()
                .count(),
            0
        );
        let lock = dt_cli::profile::lock(root.path(), &format!("{directory}-feedback-fixture"))
            .await
            .unwrap();
        drop(lock);
        let text = result.to_string();
        for hidden in [
            "https://open-platform",
            "redirect_uri",
            "state=",
            "code_challenge",
            "code_verifier",
        ] {
            assert!(!text.contains(hidden), "failure leaked login parameters");
        }
    }
}

#[tokio::test]
async fn storage_failure_stops_before_requesting_a_browser() {
    let root = tempfile::tempdir().unwrap();
    let credentials = root.path().join("credentials");
    std::fs::write(&credentials, "preserve-existing-object").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let rt = runtime(root.path(), calls.clone(), false);
    let (result, exit, _) = dt_cli::execute(&rt, arguments(false, false)).await;
    assert_eq!(exit, 1, "{result}");
    assert_eq!(result["error"]["code"], "CREDENTIAL_STORE_UNAVAILABLE");
    assert_eq!(result["error"]["details"]["area"], "credentials");
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        std::fs::read_to_string(&credentials).unwrap(),
        "preserve-existing-object"
    );
    assert!(!root.path().join("platform").exists());
}
