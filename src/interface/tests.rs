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
