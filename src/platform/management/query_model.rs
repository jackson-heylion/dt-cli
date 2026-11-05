//! 管理目录查询的具名资源；分页由服务端完成。
use super::model::{Definition, Issue};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Access {
    /// 员工入口开关。
    pub internal_enabled: bool,
    /// 外部订阅入口开关。
    pub external_enabled: bool,
    /// 全部调用是否暂停。
    pub suspended: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Draft {
    /// 统一目录接口主键。
    pub api_id: u64,
    /// 系统环境稳定标识。
    pub system_environment_id: String,
    /// 当前具名定义。
    pub contract: Definition,
    /// 当前并发修订。
    pub revision: u64,
    /// 来源变化状态。
    pub change_state: String,
    /// 当前缺项。
    pub issues: Vec<Issue>,
    /// 当前发布版本。
    pub published_version: u64,
    /// 当前调用入口开关。
    pub access: Access,
    /// 分类是否已明确确认。
    pub classification_confirmed: bool,
    /// 当前兼容性结果。
    pub compatibility: Compatibility,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Compatibility {
    /// 本次检查的草稿修订。
    pub revision: u64,
    /// 对应执行版本。
    pub execution_version: u64,
    /// 用于比较的 CLI 版本。
    pub evaluated_cli_version: String,
    /// 要求的执行器版本。
    pub required_cli_version: String,
    /// 实际版本下限，配置缺项可空。
    pub effective_minimum_cli_version: Option<String>,
    /// 配置是否就绪。
    pub ready: bool,
    /// 兼容性状态。
    pub status: String,
    /// 缺项说明。
    pub problems: Vec<Problem>,
    /// 调用模板，缺项可空。
    pub example: Option<Example>,
}
#[derive(Deserialize, Serialize)]
pub(super) struct Problem {
    /// 问题代码。
    pub code: String,
    /// 缺项字段。
    pub field: String,
    /// 安全原因。
    pub message: String,
    /// 补齐动作。
    pub action: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Example {
    /// 建议输入文件名。
    pub input_file_name: String,
    /// 固定契约输入模板 JSON 原文。
    pub input_json: String,
    /// 需要补充的业务字段。
    pub fields_to_fill: Vec<String>,
    /// 固定接口与版本的命令。
    pub commands: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DraftPage {
    /// 当前数据库页草稿。
    pub items: Vec<Draft>,
    /// 服务端匹配总数。
    pub total: u64,
    /// 当前页码。
    pub page: u32,
    /// 当前页大小。
    pub page_size: u32,
    /// 当前系统待办聚合。
    pub attention: Attention,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Attention {
    /// 从未发布的数量。
    pub unpublished_count: u64,
    /// 新导入的数量。
    pub new_count: u64,
    /// 冲突待处理的数量。
    pub conflict_count: u64,
    /// 来源缺失的数量。
    pub missing_count: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AuditContext {
    /// 已核验环境。
    pub environment: Option<String>,
    /// 目标系统环境。
    pub system_environment_id: Option<String>,
    /// 内容摘要。
    pub digest: Option<String>,
    /// 原执行请求号。
    pub request_id: Option<String>,
    /// 管理阶段。
    pub stage: Option<String>,
    /// 安全拒绝代码。
    pub code: Option<String>,
    /// 原应用关联号。
    pub trace_id: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AuditEvent {
    /// 稳定审计编号。
    pub id: u64,
    /// 原关联标识。
    pub trace_id: String,
    /// 管理事件类型。
    pub event_type: String,
    /// 真实操作者，认证未通过可空。
    pub actor_id: Option<String>,
    /// 实际角色。
    pub actor_role: String,
    /// 实际结果。
    pub outcome: String,
    /// 安全说明。
    pub reason: String,
    /// 系统时间。
    pub occurred_at: Option<String>,
    /// 脱敏具名上下文。
    pub context: Option<AuditContext>,
}
