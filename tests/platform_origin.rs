//! 通过公开 CLI 命令入口拒绝非法 origin；浏览器与网络等待不能掩盖配置错误。
use dt_cli::{Runtime, credentials::FileStore, login::SystemBrowser};

#[test]
fn platform_login_rejects_origins_before_browser_or_network() {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let runtime = Runtime {
            root: root.path().into(),
            store: Box::new(FileStore::new(root.path())),
            environments: Default::default(),
            browser: Box::new(SystemBrowser),
            interactive: false,
            aggregate_budget: std::time::Duration::from_secs(30),
        };
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
            let command = [
                "dt-cli",
                "platform",
                "login",
                "--profile",
                "origin-fixture",
                "--environment",
                "fixture",
                "--origin",
                origin,
                "--no-browser",
            ]
            .map(str::to_owned)
            .to_vec();
            let (reply, exit, _) = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                dt_cli::execute(&runtime, command),
            )
            .await
            .unwrap_or_else(|_| panic!("非法 origin 进入浏览器/网络等待：{origin}"));
            assert_eq!(reply["error"]["code"], "INVALID_ARGUMENT", "{origin}");
            assert_ne!(exit, 0);
        }
    });
}
