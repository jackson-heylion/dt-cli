use serde_json::Value;
use std::process::Command;
fn invoke(args: &[&str]) -> (Value, i32, String) {
    let (value, exit, stderr, _) = invoke_timed(args);
    (value, exit, stderr)
}
fn invoke_timed(args: &[&str]) -> (Value, i32, String, std::time::Duration) {
    // Parallel cold launches of the same native image can queue in the OS loader.
    // Measure each process, not time spent waiting behind another test's launches.
    static NATIVE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _serial = NATIVE.lock().unwrap();
    let start = std::time::Instant::now();
    let out = Command::new(env!("CARGO_BIN_EXE_dt-cli"))
        .args(args)
        .output()
        .unwrap();
    let elapsed = start.elapsed();
    (
        serde_json::from_slice(&out.stdout).unwrap(),
        out.status.code().unwrap(),
        String::from_utf8(out.stderr).unwrap(),
        elapsed,
    )
}
#[test]
fn offline_native_contracts() {
    for args in [
        vec!["help"],
        vec!["version"],
        vec!["discover"],
        vec!["schema", "iam.profile.get"],
        vec!["whoami", "--profile", "中文 空格 profile", "--dry-run"],
        vec!["doctor"],
    ] {
        let (v, exit, err) = invoke(&args);
        assert_eq!(exit, 0, "{v}");
        assert_eq!(v["ok"], true);
        assert!(err.is_empty());
    }
    let (v, _, _) = invoke(&["version"]);
    assert!(!v["data"]["buildCommit"].as_str().unwrap().is_empty());
    assert_eq!(v["data"]["catalogDigest"].as_str().unwrap().len(), 64);
    let (found, _, _) = invoke(&["discover", "--query", "whoami"]);
    assert!(
        found["data"]["operations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|op| op.get("parameters").is_none() && op.get("inputSchema").is_none())
    );
}
#[test]
fn rejects_unknown_inputs_without_echoing_secrets() {
    for args in [
        vec!["api", "call", "auth.login", "--profile", "p"],
        vec!["whoami", "--token", "sensitive-secret"],
        vec!["discover", "--limit", "21"],
        vec!["discover", "--limit", "-1"],
        vec!["schema", "iam.not-registered"],
        vec![
            "api",
            "call",
            "iam.profile.get",
            "--profile",
            "p",
            "--params",
            "{\"userId\":\"sensitive-secret\"}",
            "--dry-run",
        ],
    ] {
        let (v, exit, err) = invoke(&args);
        assert_eq!(exit, 2, "{v}");
        assert!(!v.to_string().contains("sensitive-secret"));
        assert!(!err.contains("sensitive-secret"));
    }
}

#[test]
fn generic_calls_validate_defaults_and_large_integers_before_preview() {
    for (operation, params) in [
        (
            "iam.workflow.list",
            Some(r#"{"kind":"todo","kind":"done"}"#),
        ),
        ("iam.workflow.list", None),
        ("iam.workflow.open", None),
        ("iam.workflow.applications", None),
        (
            "iam.workflow.list",
            Some(r#"{"kind":"todo","page":18446744073709551615}"#),
        ),
        (
            "iam.workflow.list",
            Some(r#"{"kind":"todo","maxItems":18446744073709551615,"all":true}"#),
        ),
        (
            "iam.workflow.list",
            Some(r#"{"kind":"todo","pageSize":18446744073709551615}"#),
        ),
    ] {
        let mut args = vec!["api", "call", operation, "--profile", "p", "--dry-run"];
        if let Some(params) = params {
            args.extend(["--params", params]);
        }
        let (value, exit, _) = invoke(&args);
        assert_eq!(exit, 2, "{value}");
        assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
    }
}
#[test]
fn noninteractive_login_never_hangs() {
    let (v, exit, _, elapsed) = invoke_timed(&[
        "auth",
        "login",
        "--profile",
        "p",
        "--environment",
        "unknown",
    ]);
    assert_eq!(exit, 8, "{v}");
    assert!(elapsed.as_secs() < 5, "native login took {elapsed:?}");
}

#[test]
fn help_and_complex_parameter_input_share_the_catalog() {
    for args in [
        vec!["whoami", "--help"],
        vec!["auth", "login", "--help"],
        vec![
            "api",
            "call",
            "iam.profile.get",
            "--profile",
            "p",
            "--params",
            "{ \n }",
            "--dry-run",
        ],
    ] {
        let (v, code, _) = invoke(&args);
        assert_eq!(code, 0, "{v}");
    }
}

#[tokio::test]
async fn offline_commands_never_touch_os_credentials_or_browser() {
    struct Forbidden;
    impl dt_cli::credentials::CredentialStore for Forbidden {
        fn read(
            &self,
            _: &str,
        ) -> dt_cli::output::Result<Option<dt_cli::credentials::Credentials>> {
            panic!("offline credential read")
        }
        fn write(
            &self,
            _: &str,
            _: &dt_cli::credentials::Credentials,
        ) -> dt_cli::output::Result<()> {
            panic!("offline credential write")
        }
        fn delete(&self, _: &str) -> dt_cli::output::Result<()> {
            panic!("offline credential delete")
        }
    }
    impl dt_cli::login::Browser for Forbidden {
        fn open(&self, _: &str) -> dt_cli::output::Result<()> {
            panic!("offline browser")
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let rt = dt_cli::Runtime {
        root: temp.path().into(),
        environments: Default::default(),
        store: Box::new(Forbidden),
        browser: Box::new(Forbidden),
        interactive: false,
        aggregate_budget: std::time::Duration::from_secs(30),
    };
    let installation = temp.path().join("installation");
    let installation_path = installation.to_str().unwrap();
    for args in [
        vec!["discover"],
        vec!["schema", "iam.profile.get"],
        vec!["doctor"],
        vec!["upgrade", "--check", "--directory", installation_path],
        vec!["auth", "login", "--profile", "p", "--dry-run"],
        vec!["whoami", "--profile", "p", "--dry-run"],
    ] {
        let (v, code, _) = dt_cli::execute(
            &rt,
            std::iter::once("dt-cli")
                .chain(args)
                .map(str::to_owned)
                .collect(),
        )
        .await;
        assert_eq!(code, 0, "{v}");
    }
    assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn cancellation_releases_loopback_and_profile_lock() {
    cancellation_case(false);
}

#[cfg(unix)]
#[test]
fn governed_cancellation_releases_loopback_and_profile_lock() {
    cancellation_case(true);
}

#[cfg(unix)]
fn cancellation_case(governed: bool) {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("child-output");
    let log = std::fs::File::create(&output).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "cancellation_child", "--nocapture"])
        .env("DT_CLI_CANCEL_FIXTURE", root.path())
        .env("DT_CLI_CANCEL_GOVERNED", if governed { "1" } else { "0" })
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !root.path().join("listening-port").exists() {
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!(
                "child did not reach browser wait: {}",
                std::fs::read_to_string(&output).unwrap()
            );
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        Command::new("kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let progress_log = std::fs::read_to_string(&output).unwrap();
            assert!(
                progress_log.contains("\"stage\":\"waiting_for_browser\""),
                "{progress_log}"
            );
            assert!(
                progress_log.contains("\"stage\":\"failed\""),
                "{progress_log}"
            );
            assert!(
                status.success(),
                "{}",
                std::fs::read_to_string(output).unwrap()
            );
            break;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            panic!("cancellation hung");
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
}

#[cfg(unix)]
#[test]
#[ignore = "spawned by cancellation_releases_loopback_and_profile_lock"]
fn cancellation_child() {
    struct WaitingBrowser(std::path::PathBuf);
    impl dt_cli::login::Browser for WaitingBrowser {
        fn open(&self, url: &str) -> dt_cli::output::Result<()> {
            let url = url::Url::parse(url).unwrap();
            let redirect = url
                .query_pairs()
                .find(|(k, _)| k == "redirect_uri")
                .unwrap()
                .1
                .to_string();
            let port = url::Url::parse(&redirect).unwrap().port().unwrap();
            std::fs::write(self.0.join("listening-port"), port.to_string()).unwrap();
            Ok(())
        }
    }
    let root = std::path::PathBuf::from(std::env::var_os("DT_CLI_CANCEL_FIXTURE").unwrap());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let rt = dt_cli::Runtime {
                root: root.clone(),
                environments: std::collections::BTreeMap::from([(
                    "fixture".into(),
                    dt_cli::profile::Environment {
                        api_origin: "https://iam.fixture.invalid".into(),
                        portal_origin: "https://portal.fixture.invalid".into(),
                        recovery_url: "https://portal.fixture.invalid/cli".into(),
                        launch_path: "/cli/launch".into(),
                    },
                )]),
                store: Box::new(dt_cli::credentials::FileStore::new(&root)),
                browser: Box::new(WaitingBrowser(root.clone())),
                interactive: false,
                aggregate_budget: std::time::Duration::from_secs(30),
            };
            let mut args = [
                "dt-cli",
                "auth",
                "login",
                "--profile",
                "cancel",
                "--environment",
                "fixture",
                "--interaction",
                "browser",
            ]
            .map(str::to_owned)
            .to_vec();
            if std::env::var("DT_CLI_CANCEL_GOVERNED").as_deref() == Ok("1") {
                args.extend(["--system".to_owned(), "fixture-system".to_owned()]);
            }
            let (v, code, _) = dt_cli::execute(&rt, args).await;
            assert_eq!(code, 8, "{v}");
            let port = std::fs::read_to_string(root.join("listening-port"))
                .unwrap()
                .parse::<u16>()
                .unwrap();
            let listener =
                std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).unwrap();
            drop(listener);
            let lock = dt_cli::profile::lock(&root, "cancel").await.unwrap();
            drop(lock);
            assert!(dt_cli::profile::read(&root, "cancel").unwrap().is_none());
            assert!(!dt_cli::governed::profile_exists(&root, "cancel").unwrap());
        });
}

#[test]
fn diagnostic_previews_reject_unknown_operations() {
    for args in [
        vec![
            "auth",
            "check",
            "--profile",
            "p",
            "--operation",
            "unknown",
            "--dry-run",
        ],
        vec!["doctor", "--operation", "unknown", "--dry-run"],
    ] {
        let (v, code, _) = invoke(&args);
        assert_eq!(code, 2, "{v}");
        assert_eq!(v["error"]["code"], "UNKNOWN_OPERATION");
    }
    let (v, code, _) = invoke(&[
        "auth",
        "check",
        "--profile",
        "p",
        "--operation",
        "help",
        "--dry-run",
    ]);
    assert_eq!(code, 0);
    assert_eq!(v["data"]["onlineVerified"], false);
    assert_eq!(v["data"]["reason"], "no-safe-probe");
}

#[test]
fn rejected_operation_is_not_echoed_as_an_operation_id() {
    let secret_shaped_input = "dtcli_a_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let (value, code, stderr) = invoke(&["api", "call", secret_shaped_input, "--profile", "p"]);
    assert_eq!(code, 2);
    assert_eq!(value["operationId"], "api.call");
    assert!(!value.to_string().contains(secret_shaped_input));
    assert!(!stderr.contains(secret_shaped_input));
}

#[test]
fn altered_profile_binding_is_rejected_before_credentials_are_read() {
    let root = tempfile::tempdir().unwrap();
    let profile = serde_json::json!({"environment":"prod","subjectId":"1","authorizationId":"fixture-auth",
        "authorizationExpiresAt":"2026-09-22T00:00:00Z","accessExpiresAt":"2026-09-15T12:00:00Z",
        "checkedAt":"2026-09-15T00:00:00Z","credentialKey":"stg/1/fixture-auth"});
    std::fs::write(root.path().join("p.json"), profile.to_string()).unwrap();
    let result = dt_cli::profile::read(root.path(), "p");
    assert!(matches!(result,Err(e) if e.code == "PROFILE_INVALID"));
}

#[test]
fn root_and_group_help_expose_available_commands() {
    let (root, code, _) = invoke(&["--help"]);
    assert_eq!(code, 0);
    assert!(
        root["data"]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|op| op["operationId"] == "iam.profile.get")
    );
    let (group, code, _) = invoke(&["auth", "--help"]);
    assert_eq!(code, 0);
    let commands = group["data"]["commands"].as_array().unwrap();
    assert!(!commands.is_empty());
    assert!(
        commands
            .iter()
            .all(|op| op["command"].as_str().unwrap().starts_with("auth "))
    );
    let out = Command::new(env!("CARGO_BIN_EXE_dt-cli"))
        .args(["version", "--format", "table"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let (version, _, _) = invoke(&["version"]);
    let text = String::from_utf8(out.stdout).unwrap();
    for field in [
        "cliVersion",
        "catalogVersion",
        "buildCommit",
        "catalogDigest",
    ] {
        assert!(text.contains(version["data"][field].as_str().unwrap()));
    }
}

#[test]
fn managed_native_launcher_preserves_argv_and_exit_code() {
    use sha2::{Digest, Sha256};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("安装 空格 中文");
    let bytes = std::fs::read(env!("CARGO_BIN_EXE_dt-cli")).unwrap();
    let key = format!("{:x}", Sha256::digest(&bytes));
    let name = if cfg!(windows) {
        "dt-cli.exe"
    } else {
        "dt-cli"
    };
    let bin = root.join("bin");
    let version = root.join("versions").join(&key);
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&version).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_dt-cli"), bin.join(name)).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_dt-cli"), version.join(name)).unwrap();
    std::fs::write(bin.join("dt-cli.launcher.json"), r#"{"schemaVersion":1}"#).unwrap();
    std::fs::write(root.join("active"), format!("{key}\n\n")).unwrap();
    let sentinel = temp.path().join("must-not-exist");
    let query = format!(
        "$(touch '{}'); 中文 \"引号\"\n第二行 > literal",
        sentinel.display()
    );
    let output = Command::new(bin.join(name))
        .args(["discover", "--query", &query])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["data"]["operations"], serde_json::json!([]));
    assert!(!sentinel.exists());
    let output = Command::new(bin.join(name))
        .args(["unknown-command"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn launcher_keeps_rollback_available_when_current_program_is_missing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let bin = root.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let name = if cfg!(windows) {
        "dt-cli.exe"
    } else {
        "dt-cli"
    };
    std::fs::copy(env!("CARGO_BIN_EXE_dt-cli"), bin.join(name)).unwrap();
    std::fs::write(bin.join("dt-cli.launcher.json"), r#"{"schemaVersion":1}"#).unwrap();
    std::fs::write(
        root.join("trust.json"),
        r#"{"schemaVersion":1,"releaseType":"candidate","identity":null}"#,
    )
    .unwrap();
    std::fs::write(root.join("active"), format!("{}\n\n", "a".repeat(64))).unwrap();
    for args in [
        vec!["upgrade", "--rollback"],
        vec!["--format", "json", "upgrade", "--rollback"],
        vec!["--format=json", "upgrade", "--rollback"],
    ] {
        let output = Command::new(bin.join(name)).args(args).output().unwrap();
        let value: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["operationId"], "upgrade");
        assert_eq!(value["error"]["code"], "ROLLBACK_UNAVAILABLE");
    }
}

#[test]
fn login_methods_are_discoverable_and_preview_has_no_identity_side_effects() {
    for method in ["password", "dingtalk", "sms"] {
        let (value, exit, _) = invoke(&[
            "auth",
            "login",
            "--profile",
            "login-preview",
            "--environment",
            "stg",
            "--login-method",
            method,
            "--dry-run",
        ]);
        assert_eq!(exit, 0, "{value}");
        let (value, exit, _) = invoke(&[
            "auth",
            "login",
            "--profile",
            "login-preview",
            "--environment",
            "stg",
            "--system",
            "hrmp",
            "--login-method",
            method,
            "--dry-run",
        ]);
        assert_eq!(exit, 0, "{value}");
    }
    let (value, exit, _) = invoke(&[
        "auth",
        "login",
        "--profile",
        "login-preview",
        "--login-method",
        "sensitive-secret",
        "--dry-run",
    ]);
    assert_eq!(exit, 2, "{value}");
    assert!(!value.to_string().contains("sensitive-secret"));
    let (value, exit, _) = invoke(&["auth", "login", "--help"]);
    assert_eq!(exit, 0, "{value}");
    assert!(value.to_string().contains("dingtalk"));
}

#[test]
fn business_schema_and_call_require_matching_catalog_before_parameter_validation() {
    for args in [
        vec!["schema", "supply-chain-server.delivery-centers.list"],
        vec![
            "api",
            "call",
            "supply-chain-server.delivery-centers.list",
            "--params",
            "{}",
        ],
    ] {
        let (value, code, _) = invoke(&args);
        assert_eq!(code, 2, "{value}");
        assert_eq!(value["error"]["code"], "PROFILE_REQUIRED");
        assert_eq!(
            value["meta"]["actions"][0]["argv"],
            serde_json::json!([
                "dt-cli",
                "profiles",
                "list",
                "--system",
                "supply-chain-server"
            ])
        );
    }
    let (value, code, _) = invoke(&["discover", "--query", "配送中心"]);
    assert_eq!(code, 0);
    assert_eq!(value["data"]["catalogSource"], "bundled-cli");
    assert_eq!(value["data"]["cliVersion"], env!("CARGO_PKG_VERSION"));
    assert_ne!(value["data"]["cliVersion"], value["data"]["catalogVersion"]);
    let (help, code, _) = invoke(&["auth", "login", "--help"]);
    assert_eq!(code, 0);
    assert!(
        help["data"]["inputSchema"]["properties"]["interaction"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x == "browser")
    );
}

#[tokio::test]
async fn storage_diagnostic_is_explicit_and_keeps_default_doctor_offline() {
    struct NoBrowser;
    impl dt_cli::login::Browser for NoBrowser {
        fn open(&self, _: &str) -> dt_cli::output::Result<()> {
            panic!("diagnostic opened browser")
        }
    }
    let root = tempfile::tempdir().unwrap();
    let rt = dt_cli::Runtime {
        root: root.path().into(),
        environments: std::collections::BTreeMap::new(),
        store: Box::new(dt_cli::credentials::FileStore::new(root.path())),
        browser: Box::new(NoBrowser),
        interactive: false,
        aggregate_budget: std::time::Duration::from_secs(30),
    };
    let run = |args: Vec<&str>| dt_cli::execute(&rt, args.into_iter().map(str::to_owned).collect());
    let (value, code, _) = run(vec!["dt-cli", "doctor"]).await;
    assert_eq!(code, 0, "{value}");
    assert!(!root.path().join("credentials").exists());
    let (value, code, _) = run(vec!["dt-cli", "doctor", "--storage"]).await;
    assert_eq!(code, 0, "{value}");
    assert_eq!(value["data"]["credentials"]["status"], "verified");
    for area in ["credentials", "governed"] {
        assert_eq!(
            std::fs::read_dir(root.path().join(area)).unwrap().count(),
            0
        );
    }
    let (value, code, _) = run(vec!["dt-cli", "doctor", "--storage", "--dry-run"]).await;
    assert_eq!(code, 2, "{value}");
}
