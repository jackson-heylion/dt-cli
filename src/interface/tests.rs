//! 固定平台入口的命令契约验证，不为断言默认环境访问公网。
use super::*;
use request::Action;

#[test]
fn employee_and_management_login_keep_explicit_environment_or_default_to_production() {
    for management in [false, true] {
        for environment in [None, Some("prod"), Some("stg")] {
            let mut args = vec!["dt-cli"];
            if management {
                args.push("manage");
            }
            args.extend(["login", "--profile", "fixture"]);
            if let Some(environment) = environment {
                args.extend(["--environment", environment]);
            }
            let parsed = command().try_get_matches_from(args).unwrap();
            let input = parse::platform(&parsed).unwrap();
            let login = match input.action {
                Action::Login(login) => login,
                Action::Manage(input) => input.login.unwrap(),
                _ => panic!("未解析为登录"),
            };
            assert_eq!(login.environment, environment.unwrap_or("prod"));
            assert!(login.origin.is_none());
        }
    }
}

#[tokio::test]
async fn storage_doctor_creates_platform_directory_with_the_login_storage_policy() {
    let root = tempfile::tempdir().unwrap();
    let rt = crate::Runtime {
        root: root.path().into(),
        store: Box::new(crate::credentials::FileStore::new(root.path())),
        browser: Box::new(crate::login::SystemBrowser),
        interactive: false,
        aggregate_budget: std::time::Duration::from_secs(30),
    };
    let (result, exit, _) = crate::execute(
        &rt,
        vec!["dt-cli".into(), "doctor".into(), "--storage".into()],
    )
    .await;
    assert_eq!(exit, 0, "{result}");
    assert_eq!(result["data"]["platform"]["atomicReplace"], true);
    let directory = root.path().join("platform");
    assert!(crate::private_store::validate(&directory, true).unwrap());
    assert_eq!(std::fs::read_dir(directory).unwrap().count(), 0);
}
