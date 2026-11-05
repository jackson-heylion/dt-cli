use super::super::{
    contract::Published, executor::ExecutionResult, input_model::*, model::PlatformProfile,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum WriteState {
    Prepared,
    Authorized,
    Dispatched,
    Succeeded,
    Failed,
    ResultUnknown,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Intent {
    /// 服务端原意图标识。
    pub id: String,
    /// 原客户端业务指令标识，恢复不换键。
    pub client_request_id: String,
    /// 用户明确业务指令。
    pub instruction: String,
    /// IAM人员稳定标识。
    pub subject_id: String,
    /// 原平台独立会话。
    pub session_id: String,
    /// 固定系统。
    pub system_id: String,
    /// 固定环境。
    pub environment: String,
    /// 稳定接口标识。
    pub operation_id: String,
    /// 原执行版本。
    pub operation_version: u64,
    /// 不可变契约摘要。
    pub contract_digest: String,
    /// 完整参数和身份绑定摘要。
    pub fingerprint: String,
    /// 原明确视角和对象，可空。
    pub view: Option<TaskView>,
    /// 当前可信写入状态。
    pub state: WriteState,
    /// 原意图UTC绝对到期秒数。
    pub expires_at: i64,
    /// 实际上游HTTP状态，可空。
    pub http_status: Option<u16>,
    /// 原业务响应字节摘要，可空。
    pub response_digest: Option<String>,
}
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stage {
    Preparing,
    Prepared,
    Authorizing,
    Authorized,
    DispatchAttempted,
    Finished,
    ResultUnknown,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Record {
    /// 本地记录格式版本。
    pub schema_version: u8,
    /// 持久化的本地业务指令标识。
    pub id: String,
    /// 原profile名称，仅展示使用。
    pub profile_name: String,
    /// 原身份、会话和环境快照，不包含token。
    pub profile: PlatformProfile,
    /// 原不可变执行定义。
    pub published: Published,
    /// 原完整业务参数，不包含服务器管理的业务凭证。
    pub request: WireInvocation,
    /// 用户本次明确业务指令。
    pub instruction: String,
    /// 本地持久化阶段，不把不明结果重新排队。
    pub stage: Stage,
    /// 原服务端意图，可空表示准备响应未确认。
    pub intent: Option<Intent>,
    /// 已收到的原业务响应，可空。
    pub result: Option<ExecutionResult>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct View {
    /// 可用于后续核对的本地记录标识。
    pub record_id: String,
    /// 当前本地阶段。
    pub stage: Stage,
    /// 服务端原意图，可空。
    pub intent: Option<Intent>,
    /// 原业务响应，仅收到完整响应时存在。
    pub result: Option<ExecutionResult>,
    /// 当前状态的安全说明。
    pub message: String,
}
