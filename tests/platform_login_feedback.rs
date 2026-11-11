//! 平台登录反馈通过公开命令核对；不打开真实浏览器或连接外部平台。
use dt_cli::{
    Runtime,
    credentials::FileStore,
    login::Browser,
    output::{Failure, Result},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
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
