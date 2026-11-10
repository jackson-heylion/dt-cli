use serde::Serialize;
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Recovery {
    /// 原本人记录，可空时要求员工明确选择。
    pub profile: Option<String>,
    /// 原任务环境，可空时不生成登录动作。
    pub environment: Option<String>,
    /// 原业务或管理用途。
    pub purpose: String,
    /// 原写入或任务编号，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    /// 原管理计划编号，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// 原管理执行请求编号，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// 可信命令参数数组；没有 Shell 源码。
    pub actions: Vec<Action>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    /// 固定动作标识。
    pub id: &'static str,
    /// 可直接传给进程的参数数组，限定原身份及范围。
    pub argv: Vec<String>,
    /// 员工完成本人验证时为 true。
    pub requires_interaction: bool,
    /// 此动作的固定安全说明。
    pub reason: &'static str,
}
