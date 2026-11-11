//! 失败仍可携带的具名结果；只在最终信封边界序列化。
use serde::Serialize;
#[derive(Serialize)]
#[serde(untagged)]
pub(crate) enum Data {
    /// 失败没有业务载荷。
    Empty,
    /// 本人的合法视角候选。
    Views(Vec<crate::platform::contract::ViewOption>),
    /// 原完整执行结果。
    Execution(crate::platform::executor::ExecutionResult),
    /// 原读取工作包检查点。
    Job(crate::platform::jobs::model::Summary),
    /// 原写入记录与状态。
    Write(crate::platform::writes::model::View),
    /// 尚待明确选择的授权接口候选。
    Resolution(crate::platform::discovery::Resolution),
    /// 固定接口输入模板。
    Template(crate::platform::discovery::Detail),
    /// 本机清理与远端撤销分项结果。
    Logout(crate::platform::model::LogoutView),
    /// 原管理计划应用结果。
    Application(crate::platform::management::model::Application),
}
impl std::fmt::Debug for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PartialResult")
    }
}
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Meta {
    /// 原范围完整性，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub complete: Option<bool>,
    /// 当前候选需要明确选择时为 true。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_required: Option<bool>,
    /// 原任务当前固定阶段，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<&'static str>,
    /// 固定恢复要求，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<&'static str>,
    /// 模板原文件摘要，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_digest: Option<String>,
    /// 原本机写入记录，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
    /// 原业务派发是否允许重发；未知结果为 false。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_business_request: Option<bool>,
    /// 准备阶段未派发时为 false。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub business_dispatch: Option<bool>,
    /// 原登录保存失败后的远端撤销结果，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_revocation: Option<&'static str>,
    /// 登录新会话的本地保存结论，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_save: Option<LoginSave>,
    /// 本次新凭证的补偿清理结果，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_cleanup: Option<CredentialCleanup>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LoginSave {
    NotSaved,
    Unconfirmed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CredentialCleanup {
    Confirmed,
    Failed,
    NotAttempted,
}
