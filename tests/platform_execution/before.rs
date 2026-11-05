//! 固定旧版原生二进制，仅量测同系统、同契约、同数据的三页读取。
use super::support::{BODY, Harness};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BaselineProfile {
    /// 原生CLI公开元数据格式。
    schema_version: u8,
    /// 当前隔离平台入口。
    origin: String,
    /// 固定测试环境。
    environment: String,
    /// 旧版仅接受单个系统范围。
    system_id: String,
    /// 已验证IAM稳定人员标识。
    subject_id: String,
    /// 固定员工身份域。
    domain: String,
    /// 当前独立CLI会话标识。
    session_id: String,
    /// 原会话绝对期限。
    expires_at: String,
    /// 安全存储引用，报告不输出此字段。
    credential_key: String,
}

pub async fn run(binary: &str) {
    let folder = tempfile::tempdir().unwrap();
    let h = Harness::new(folder.path());
    h.login("fixture").await;
    // 只预置旧版系统范围本地记录；不修改二进制或服务端授权，登录不计入分页基准。
    let profile = h.root.join("platform/fixture.json");
    let mut row: BaselineProfile =
        serde_json::from_slice(&std::fs::read(&profile).unwrap()).unwrap();
    row.system_id = "supply-chain-server".into();
    std::fs::write(profile, serde_json::to_vec(&row).unwrap()).unwrap();
    let input = h.root.join("before.json");
    std::fs::write(
        &input,
        format!("{{\"bodyJson\":{}}}", serde_json::to_string(BODY).unwrap()),
    )
    .unwrap();
    let initial = h.stats("preparation").await;
    let output = std::process::Command::new(binary)
        .args([
            "platform",
            "jobs",
            "start",
            "--profile",
            "fixture",
            "--operation",
            "fixture-before-pages",
            "--version",
            "1",
            "--input-file",
            input.to_str().unwrap(),
        ])
        .env("DT_CLI_LOCAL_TEST_CONFIG", &h.root)
        .output()
        .unwrap();
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "{result}");
    assert_eq!(result["data"]["complete"], true);
    assert_eq!(result["data"]["progress"]["pages"], 3);
    assert_eq!(result["data"]["progress"]["records"], 5);
    let end = h.stats("preparation").await;
    let counts = json!({
        "version":"df225a9 / 0.6.0", "pages":3, "records":5,
        "contract":end["contract"].as_u64().unwrap()-initial["contract"].as_u64().unwrap(),
        "runtime":end["runtime"].as_u64().unwrap()-initial["runtime"].as_u64().unwrap(),
        "me":end["me"].as_u64().unwrap()-initial["me"].as_u64().unwrap(),
        "gateway":h.connections("fixture-before-pages").await
    });
    assert_eq!(counts["contract"], 4);
    assert_eq!(counts["runtime"], 3);
    assert_eq!(counts["me"], 4);
    assert_eq!(counts["gateway"]["requests"], 3);
    assert_eq!(counts["gateway"]["connections"], 3);
    if let Ok(path) = std::env::var("DT_CLI_BASELINE_REPORT") {
        std::fs::write(path, serde_json::to_vec_pretty(&counts).unwrap()).unwrap();
    }
}
