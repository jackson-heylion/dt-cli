use dt_cli::{
    Runtime,
    credentials::{CredentialStore, FileStore},
};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const BODY: &str = "{ \"note\": null, \"amount\": 123.0000000000000000000000001 }";
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Case {
    /// 平台生成或明确登记的稳定接口标识。
    pub operation_id: String,
    /// 接口主键，供实际授权变更验收。
    pub api_id: u64,
    /// 真实HTTP方法。
    pub method: String,
    /// 已声明媒体类型。
    pub media: String,
    /// 固定路径模板。
    pub path: String,
    /// 固定执行版本。
    pub version: u64,
}
pub struct Harness {
    /// 隔离的真实原生CLI配置与私有结果目录。
    pub root: PathBuf,
    /// 当前真实平台origin。
    pub platform: String,
    /// 当前真实Gatewayorigin。
    pub gateway: String,
    /// 控制通道受控HTTP夹具地址。
    pub control: String,
    /// 仅控制测试夹具的随机身份。
    pub control_key: String,
    /// 本机隔离平台的管理员Cookie，只用于已登记测试接口撤权。
    pub admin_cookie: String,
    /// 实际登记并发布授权的案例清单。
    pub cases: Vec<Case>,
}
impl Harness {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.into(),
            platform: std::env::var("DT_PLATFORM_HTTP_FIXTURE_ORIGIN").unwrap(),
            gateway: std::env::var("DT_PLATFORM_HTTP_GATEWAY_ORIGIN").unwrap(),
            control: std::env::var("DT_EXECUTION_FIXTURE_CONTROL").unwrap(),
            control_key: std::env::var("DT_EXECUTION_FIXTURE_CONTROL_KEY").unwrap(),
            admin_cookie: std::env::var("DT_EXECUTION_FIXTURE_ADMIN_COOKIE").unwrap(),
            cases: serde_json::from_str(&std::env::var("DT_EXECUTION_FIXTURE_CASES").unwrap())
                .unwrap(),
        }
    }
    pub fn case(&self, name: &str) -> &Case {
        self.cases
            .iter()
            .find(|case| case.operation_id == name)
            .unwrap()
    }
    pub fn command(&self, args: &[&str]) -> std::process::Command {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_dt-cli"));
        command
            .args(args)
            .env("DT_CLI_LOCAL_TEST_CONFIG", &self.root);
        command
    }
    pub fn run(&self, args: &[&str]) -> (Value, i32) {
        let output = self.command(args).output().unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(!stdout.contains("dtp_cli_at_"));
        assert!(!stdout.contains("dtp_cli_rt_"));
        assert!(!stderr.contains("dtp_cli_at_"));
        assert!(!stderr.contains("dtp_cli_rt_"));
        (
            serde_json::from_str(&stdout)
                .unwrap_or_else(|_| panic!("原生CLI未输出协议JSON: {stderr}")),
            output.status.code().unwrap(),
        )
    }
    pub fn ok(&self, args: &[&str]) -> Value {
        let (value, status) = self.run(args);
        assert_eq!(status, 0, "{value}");
        value["data"].clone()
    }
    pub fn input(&self, name: &str, media: &str) -> PathBuf {
        let body = if media == "JSON" {
            format!(",\"bodyJson\":{}", serde_json::to_string(BODY).unwrap())
        } else if media == "FORM" {
            ",\"form\":[{\"name\":\"labels\",\"value\":[\"x\",\"y\"]},{\"name\":\"enabled\",\"value\":true}]".into()
        } else {
            String::new()
        };
        let raw = format!(
            "{{\"path\":[{{\"name\":\"id\",\"value\":42}}],\"query\":[{{\"name\":\"tags\",\"value\":[\"a b\",\"中\"]}}],\"headers\":[{{\"name\":\"Language\",\"value\":\"zh-CN\"}}]{body}}}"
        );
        let path = self.root.join(name);
        std::fs::write(&path, raw).unwrap();
        path
    }
    pub async fn login(&self, name: &str) {
        self.login_as(name, 'C', "1024").await;
    }
    pub async fn login_as(&self, name: &str, code: char, subject: &str) {
        let rt = Runtime {
            root: self.root.clone(),
            store: Box::new(FileStore::new(&self.root)),
            browser: Box::new(crate::browser_fixture::IdentityBrowser { code }),
            interactive: false,
            aggregate_budget: std::time::Duration::from_secs(30),
        };
        let args = [
            "dt-cli",
            "login",
            "--profile",
            name,
            "--origin",
            &self.platform,
            "--environment",
            "fixture",
            "--system",
            "supply-chain-server",
        ]
        .map(str::to_owned)
        .to_vec();
        let (value, code, _) = dt_cli::execute(&rt, args).await;
        assert_eq!(code, 0, "{value}");
        assert_eq!(value["data"]["subjectId"], subject);
    }
    pub async fn stats(&self, action: &str) -> Value {
        reqwest::Client::new()
            .get(format!("{}/__fixture/{action}", self.control))
            .header("X-Fixture-Key", &self.control_key)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
    pub async fn connections(&self, operation: &str) -> Value {
        reqwest::Client::new()
            .get(format!(
                "{}/__fixture/connections/{operation}",
                self.gateway
            ))
            .header("X-Fixture-Key", &self.control_key)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
    pub async fn grant(&self, case: &Case, enabled: bool) {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Grant {
            /// 固定版本。
            version: u64,
            /// 明确员工授权。
            principal_type: &'static str,
            /// 本地HTTP夹具的IAM稳定人员标识。
            principal_id: &'static str,
            /// 撤销或恢复。
            enabled: bool,
        }
        let response = reqwest::Client::new()
            .put(format!(
                "{}/platform/registry/drafts/{}:fixture/grants",
                self.platform, case.api_id
            ))
            .header("Cookie", &self.admin_cookie)
            .header("Origin", &self.platform)
            .json(&Grant {
                version: case.version,
                principal_type: "USER",
                principal_id: "1024",
                enabled,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
    }
    pub fn access(&self) -> String {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ProfileKey {
            /// 只定位本机受控测试凭证键。
            credential_key: String,
        }
        let profile: ProfileKey = serde_json::from_slice(
            &std::fs::read(self.root.join("platform/fixture.json")).unwrap(),
        )
        .unwrap();
        FileStore::new(&self.root)
            .read(&profile.credential_key)
            .unwrap()
            .unwrap()
            .access_token
            .clone()
    }
}
