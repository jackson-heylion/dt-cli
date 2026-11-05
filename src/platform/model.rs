use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PlatformProfile {
    /// 凭证用途；旧本机业务记录缺省为 CLI，管理记录独立保存。
    #[serde(default)]
    pub purpose: SessionPurpose,
    /// 元数据格式版本。
    pub schema_version: u8,
    /// 平台对外 origin，与 IAM 和业务系统隔离。
    pub origin: String,
    /// 登录时固定的环境。
    pub environment: String,
    /// 平台会话为platform；任务系统由不可变契约固定。
    pub system_id: String,
    /// IAM 验证的稳定人员标识。
    pub subject_id: String,
    /// IAM 验证的身份域。
    pub domain: IdentityDomain,
    /// 平台独立会话标识。
    pub session_id: String,
    /// 会话绝对到期时刻，UTC RFC3339。
    pub expires_at: String,
    /// 既有安全存储中的独立键，不包含凭证。
    pub credential_key: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum IdentityDomain {
    Employee,
    External,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TokenReply {
    /// 平台访问凭证。
    #[serde(rename = "access_token")]
    pub access_token: String,
    /// 平台刷新凭证。
    #[serde(rename = "refresh_token")]
    pub refresh_token: String,
    /// 必须为 Bearer。
    #[serde(rename = "token_type")]
    pub token_type: String,
    /// 访问凭证期限，单位秒。
    #[serde(rename = "expires_in")]
    pub expires_in: i64,
    /// 独立会话标识。
    pub authorization_id: String,
    /// 会话绝对期限。
    pub authorization_expires_at: String,
    /// 固定环境。
    pub environment: String,
    /// 平台会话范围，值为platform。
    pub system_id: String,
    /// 服务器签发时确定的访问凭证绝对到期时刻，恢复响应不重新起算。
    pub access_expires_at: String,
    /// 服务器签发时刻，UTC RFC3339。
    pub server_time: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionView {
    /// 平台独立会话标识。
    pub session_id: String,
    /// IAM 稳定人员标识。
    pub subject_id: String,
    /// IAM 人员身份域。
    pub domain: IdentityDomain,
    /// IAM 验证的展示名称。
    pub display_name: String,
    /// 会话用途，CLI 或 PORTAL。
    pub purpose: String,
    /// 会话绝对到期时刻。
    pub expires_at: String,
    /// 是否为当前访问会话。
    pub current: bool,
    /// 是否已被撤销。
    pub revoked: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SessionPage {
    /// 当前页会话记录。
    pub items: Vec<SessionView>,
    /// 服务器记录总数。
    pub total: u64,
    /// 从 1 开始的页码。
    pub page: u32,
    /// 每页记录数。
    pub page_size: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RemoteError {
    /// 服务端固定错误码，消息原文不进入输出。
    pub code: Option<String>,
    /// 新发现协议的失败阶段，可空。
    pub stage: Option<String>,
    /// 服务端关联号，不包含业务载荷。
    pub trace_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LogoutView {
    /// 当前会话撤销结果。
    pub remote_revocation: &'static str,
    /// 当前本机记录清理结果。
    pub local_cleanup: &'static str,
}

/// 平台人员会话的独立用途与固定受众，不能由清单覆盖。
#[derive(Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum SessionPurpose {
    /// 员工业务调用用途。
    #[default]
    Cli,
    /// 本人开放平台管理用途。
    Management,
}
impl SessionPurpose {
    pub fn text(self) -> &'static str {
        match self {
            Self::Cli => "CLI",
            Self::Management => "MANAGEMENT",
        }
    }
    pub fn namespace(self) -> &'static str {
        match self {
            Self::Cli => "/platform/cli",
            Self::Management => "/platform/management",
        }
    }
    pub fn directory(self) -> &'static str {
        match self {
            Self::Cli => "platform",
            Self::Management => "platform-management",
        }
    }
    pub fn access_prefix(self) -> &'static str {
        match self {
            Self::Cli => "dtp_cli_at_",
            Self::Management => "dtp_mgmt_at_",
        }
    }
    pub fn refresh_prefix(self) -> &'static str {
        match self {
            Self::Cli => "dtp_cli_rt_",
            Self::Management => "dtp_mgmt_rt_",
        }
    }
}
