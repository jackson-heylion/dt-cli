use serde_json::Value;
use std::process::Command;

fn invoke(args: &[&str]) -> (Value, i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_dt-cli"))
        .args(args)
        .output()
        .unwrap();
    (
        serde_json::from_slice(&out.stdout).unwrap(),
        out.status.code().unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn root_help_discovers_current_platform_without_legacy_commands() {
    let (value, code, stderr) = invoke(&["help"]);
    assert_eq!(code, 0, "{value}");
    assert!(stderr.is_empty());
    let help = value["data"].to_string();
    for name in ["login", "call", "jobs", "writes", "manage", "skill"] {
        assert!(help.contains(name), "{name}: {help}");
    }
    for old in ["iam.workflow", "governed", "whoami", "auth login"] {
        assert!(!help.contains(old), "{old}: {help}");
    }
}

#[test]
fn nested_help_and_format_positions_share_one_command_definition() {
    for args in [
        vec!["jobs"],
        vec!["manage"],
        vec!["writes", "prepare", "-h"],
        vec!["context", "use", "--help"],
        vec!["--format", "json", "call", "--help"],
        vec!["jobs", "status", "--format", "json", "--help"],
    ] {
        let (value, code, stderr) = invoke(&args);
        assert_eq!(code, 0, "{args:?}: {value}");
        assert!(stderr.is_empty());
        assert!(value["data"]["commands"].is_array());
    }
    let (call, _, _) = invoke(&["call", "--help"]);
    assert!(call["data"].to_string().contains("input-file"));
}
#[test]
fn rejected_legacy_and_sensitive_arguments_stay_offline_and_redacted() {
    for args in [
        vec![
            "login",
            "--profile",
            "dtp_cli_rt_sensitive-secret",
            "--environment",
            "prod",
        ],
        vec!["platform", "login"],
        vec!["auth", "login"],
        vec!["whoami"],
        vec!["workflow", "list"],
        vec!["tasks", "run"],
        vec!["intents", "invoke"],
        vec!["skill", "prepare"],
        vec!["call", "--token", "dtp_cli_at_sensitive-secret"],
        vec![
            "call",
            "--operation",
            "orders",
            "--version",
            "sensitive-secret",
            "--path",
            "/orders",
        ],
        vec![
            "writes",
            "prepare",
            "--operation",
            "orders",
            "--input-file",
            "sensitive-secret",
            "--instruction",
            "dtp_cli_rt_sensitive-secret",
            "--unexpected",
        ],
    ] {
        let (value, code, stderr) = invoke(&args);
        assert_eq!(code, 2, "{args:?}: {value}");
        assert!(!value.to_string().contains("sensitive-secret"));
        assert!(!stderr.contains("sensitive-secret"));
    }
}
