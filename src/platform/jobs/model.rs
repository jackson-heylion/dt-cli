use super::super::{
    contract::Published, executor::ExecutionResult, input_model::WireInvocation,
    model::PlatformProfile,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum JobState {
    Running,
    Paused,
    Succeeded,
    Incomplete,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Progress {
    /// 已验证并持久化的完整页数。
    pub pages: u32,
    /// 已完成页的记录数。
    pub records: u64,
    /// 已完成页的原始响应字节数。
    pub bytes: u64,
    /// 累计执行预算毫秒数；中断请求保守计入已预留等待上限，不因恢复重置。
    pub active_millis: u64,
    /// 已持久化的本次请求等待预算，得到完整页后按实际耗时结算。
    pub pending_budget_millis: Option<u64>,
    /// 下一次请求的页号。
    pub next_page: u64,
    /// 下一次请求的游标，可空。
    pub next_cursor: Option<String>,
    /// 已消费游标摘要，用于检测分页循环，命中不重置预算。
    pub cursor_digests: Vec<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Job {
    /// 本地工作包格式版本。
    pub schema_version: u8,
    /// 原本机任务标识。
    pub id: String,
    /// 原profile名称，仅用于展示。
    pub profile_name: String,
    /// 原本人、系统、环境、会话与安全存储键，不包含token。
    pub profile: PlatformProfile,
    /// 原不可变执行版本与摘要，绝不自动切最新版。
    pub published: Published,
    /// 原完整参数与任务固定视角。
    pub request: WireInvocation,
    /// 持久化执行进度。
    pub progress: Progress,
    /// 任务实际状态。
    pub state: JobState,
    /// 固定失败或暂停原因，可空。
    pub reason: Option<String>,
    /// 创建时刻，UTC RFC3339。
    pub created_at: String,
    /// 最近检查点时刻，UTC RFC3339。
    pub updated_at: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Page {
    /// 所属原任务。
    pub job_id: String,
    /// 本地顺序页号，从一开始。
    pub index: u32,
    /// 原固定契约摘要。
    pub contract_digest: String,
    /// 实际请求路径、查询、业务头、正文与视角的摘要。
    pub request_digest: String,
    /// 原始完整业务响应及判定。
    pub result: ExecutionResult,
    /// 本次完整请求实际耗时毫秒，用于中断后的预算结算。
    pub elapsed_millis: u64,
    /// 取得此页的UTC时刻，各页不宣称同一时点快照。
    pub observed_at: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Summary<'a> {
    /// 原任务标识。
    pub job_id: &'a str,
    /// 固定接口标识。
    pub operation_id: &'a str,
    /// 固定执行版本。
    pub operation_version: u64,
    /// 固定契约摘要。
    pub contract_digest: &'a str,
    /// 原人员稳定标识。
    pub subject_id: &'a str,
    /// 固定系统。
    pub system_id: &'a str,
    /// 固定环境。
    pub environment: &'a str,
    /// 任务当前状态。
    pub state: &'a JobState,
    /// 真实持久化进度。
    pub progress: &'a Progress,
    /// 暂停/不完整原因，可空。
    pub reason: &'a Option<String>,
    /// 仅成功完成时为真。
    pub complete: bool,
    /// 分页结果是否承诺同一业务时点快照；始终为否。
    pub point_in_time_snapshot: bool,
    /// 后续操作说明。
    pub message: &'a str,
}
