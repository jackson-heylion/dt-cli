//! 人员管理协议的具名输入、计划、资源和恢复记录。
use super::super::contract::{
    Effect, Limits, Media, Method, Pagination, Parameter, Published, Success, ViewType,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Definition {
    /// 执行契约格式版本。
    pub contract_schema_version: u32,
    /// 计划已固定的稳定接口身份。
    pub operation_id: String,
    /// 固定环境内执行版本。
    pub operation_version: u64,
    /// 最低执行器版本。
    pub minimum_cli_version: String,
    /// 固定系统。
    pub system_id: String,
    /// 固定环境。
    pub environment: String,
    /// 业务接口名称。
    pub title: String,
    /// 实际 HTTP 方法。
    pub method: Method,
    /// 实际路径。
    pub path: String,
    /// 未确认分类可空，未就绪不能应用。
    pub effect: Option<Effect>,
    /// 请求媒体。
    pub media: Media,
    /// 具名参数清单。
    pub parameters: Vec<Parameter>,
    /// 请求 Schema 原文，可空。
    pub request_schema: Option<String>,
    /// 响应 Schema 原文，可空。
    pub response_schema: Option<String>,
    /// 成功规则，缺项可空。
    pub success: Option<Success>,
    /// 分页规则，缺项可空。
    pub pagination: Option<Pagination>,
    /// 合法视角类别。
    pub view_types: Vec<ViewType>,
    /// 数据范围。
    pub data_scope: String,
    /// 敏感字段。
    pub sensitive_fields: Vec<String>,
    /// 固定路由绑定摘要。
    pub route_binding_digest: String,
    /// 硬限额，缺项可空。
    pub limits: Option<Limits>,
    /// 正文是否必填。
    pub request_body_required: bool,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Route {
    /// 明确上游基址，只作配置，不是 CLI 调用地址。
    pub upstream_base_url: String,
    /// 平台支持的认证方式。
    pub authentication_mode: Authentication,
    /// 业务身份头名称，可空。
    pub credential_header: Option<String>,
    /// 业务身份头前缀，可空。
    pub credential_prefix: Option<String>,
    /// 路由超时毫秒。
    pub timeout_millis: u32,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum Authentication {
    None,
    NativeAuth,
    NativeIdentity,
    MachineBearer,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct SystemConfiguration {
    /// 系统稳定编码。
    pub system_id: String,
    /// 系统业务名称。
    pub title: String,
    /// 固定环境。
    pub environment: String,
    /// 受控来源地址，可空。
    pub source_url: Option<String>,
    /// 已明确允许的来源 origin。
    pub allowed_origins: Vec<String>,
    /// 私有路由配置。
    pub route: Route,
    /// 可选共享默认模板。
    pub template: Option<Definition>,
    /// 系统是否启用。
    pub enabled: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SystemView {
    /// 系统环境稳定标识。
    pub id: String,
    /// 统一目录主键。
    pub api_service_id: u64,
    /// 已核权系统配置。
    pub configuration: SystemConfiguration,
    /// 最后同步 UTC 秒，可空。
    pub last_synced_at: Option<u64>,
    /// 安全同步错误，可空。
    pub last_sync_error: Option<String>,
    /// 当前配置摘要。
    pub configuration_digest: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Grant {
    /// 实际版本由计划固定，清单可为零。
    pub version: u64,
    /// 内部主体类型。
    pub principal_type: Principal,
    /// 明确主体标识。
    pub principal_id: String,
    /// 本次许可或撤销。
    pub enabled: bool,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum Principal {
    User,
    UserDeny,
    AllEmployees,
    PlatformRole,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Overrides {
    /// 完整具名调整定义，仅应用明确字段。
    pub adjusted: Definition,
    /// 服务端固定字段枚举路径。
    pub fields: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Selection {
    /// 明确方法与路径。
    pub source_key: String,
    /// 可选字段覆盖。
    pub overrides: Option<Overrides>,
    /// 明确确认实际分类。
    pub classification_confirmed: bool,
    /// 可选明确业务别名。
    pub alias: Option<String>,
    /// 明确核对来源冲突。
    #[serde(default)]
    pub acknowledge_source: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Source {
    /// FILE 或 CONFIGURED。
    pub kind: SourceKind,
    /// 文件原文，配置来源为空。
    pub document: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub(super) enum SourceKind {
    File,
    Configured,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Manifest {
    /// 管理协议版本。
    pub protocol_version: u32,
    /// 必须等于当前管理环境。
    pub environment: String,
    /// 明确系统配置。
    pub system: SystemConfiguration,
    /// 文档来源。
    pub source: Source,
    /// 明确所选操作，最多一百项。
    pub selections: Vec<Selection>,
    /// 明确内部授权。
    pub grants: Vec<Grant>,
    /// 明确沿用兼容内部授权。
    pub carry_internal: bool,
}
#[derive(Clone, Deserialize, Serialize)]
pub(super) struct Issue {
    /// 缺项对应的稳定字段。
    pub field: String,
    /// 可披露说明。
    pub message: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SourceOperation {
    /// 原文辅助身份，可空。
    pub source_operation_id: Option<String>,
    /// 明确方法与路径。
    pub source_key: String,
    /// 固定来源定义。
    pub contract: Definition,
    /// 来源能力问题。
    pub issues: Vec<Issue>,
    /// 文档直接提取的定义。
    pub extracted: Definition,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Operation {
    /// 所选方法路径。
    pub source_key: String,
    /// 已有目录主键，新建为零。
    pub api_id: u64,
    /// 已有草稿编号，新建可空。
    pub draft_id: Option<String>,
    /// 固定草稿修订。
    pub expected_revision: u64,
    /// 固定开关修订。
    pub expected_access_revision: u64,
    /// 原发布版本。
    pub previous_version: u64,
    /// 计划实际目标版本。
    pub expected_version: u64,
    /// NEW、UPDATE 或 UNCHANGED。
    pub change: String,
    /// 固定目标执行定义。
    pub contract: Definition,
    /// 固定受控来源。
    pub source: SourceOperation,
    /// 固定人工修订。
    pub overrides: Overrides,
    /// 分类明确确认。
    pub classification_confirmed: bool,
    /// 明确可选业务别名。
    pub alias: Option<String>,
    /// 计划时已登记别名，用于显示实际差异。
    pub previous_alias: Option<String>,
    /// 禁止应用的缺项。
    pub issues: Vec<Issue>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Snapshot {
    /// 固定系统配置。
    pub system: SystemConfiguration,
    /// 原系统摘要，新建可空。
    pub expected_system_digest: Option<String>,
    /// 原系统归属摘要，新建可空。
    pub expected_ownership_digest: Option<String>,
    /// 原所选授权摘要。
    pub expected_grants_digest: String,
    /// 原所选接口和目标别名占用摘要。
    pub expected_aliases_digest: String,
    /// 固定文档快照摘要。
    pub source_digest: String,
    /// 固定所选目标。
    pub operations: Vec<Operation>,
    /// 明确内部授权主体。
    pub grants: Vec<Grant>,
    /// 内部继承确认。
    pub carry_internal: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Plan {
    /// 管理协议版本。
    pub protocol_version: u32,
    /// 稳定计划编号。
    pub plan_id: String,
    /// 核对内容摘要。
    pub digest: String,
    /// 真实操作者。
    pub actor_id: String,
    /// 固定环境。
    pub environment: String,
    /// 绝对到期 UTC 秒。
    pub expires_at: u64,
    /// READY、NOT_READY 或 EXPIRED。
    pub state: String,
    /// 可审核完整目标与前提。
    pub snapshot: Snapshot,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Application {
    /// 管理协议版本。
    pub protocol_version: u32,
    /// 原计划编号。
    pub plan_id: String,
    /// 原计划摘要。
    pub digest: String,
    /// 原执行请求号，可空。
    pub request_id: Option<String>,
    /// 主事务权威状态。
    pub state: String,
    /// 实际固定发布结果。
    pub published: Vec<Published>,
    /// 授权投影状态。
    pub projection: String,
    /// 独立业务验证状态。
    pub business_verification: String,
    /// 安全下一步动作。
    pub next_action: String,
    /// 原应用关联标识，可空。
    pub trace_id: Option<String>,
    /// 已持久化失败代码，可空。
    pub failure_code: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Page<T> {
    /// 服务端当前页具名资源。
    pub items: Vec<T>,
    /// 服务端核权后总数。
    pub total: u64,
    /// 页码。
    pub page: u32,
    /// 每页数量。
    pub page_size: u32,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Capabilities {
    /// 当前协议版本。
    pub protocol_version: u32,
    /// 真实管理操作者。
    pub actor_id: String,
    /// 当前管理角色。
    pub role: String,
    /// 固定环境。
    pub environment: String,
    /// 已实现操作。
    pub operations: Vec<String>,
    /// 单计划最多接口数。
    pub maximum_operations: u32,
    /// 计划有效秒数。
    pub plan_lifetime_seconds: u32,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LocalExecution {
    /// 恢复记录格式版本。
    pub schema_version: u32,
    /// 固定平台地址。
    pub origin: String,
    /// 固定当前主体。
    pub subject_id: String,
    /// 固定环境。
    pub environment: String,
    /// 原计划编号。
    pub plan_id: String,
    /// 已核对摘要。
    pub digest: String,
    /// 原执行请求号。
    pub request_id: String,
    /// 发送前持久化 UNKNOWN，权威回读后再更新。
    pub state: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Apply<'a> {
    /// 原内容摘要。
    pub digest: &'a str,
    /// 原请求号。
    pub request_id: &'a str,
}
