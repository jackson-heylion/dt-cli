//! 管理协议独立错误贯通，业务请求继续使用原传输规则。
use crate::output::{Failure, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RemoteError {
    /// 服务端稳定错误代码。
    pub code: String,
    /// 安全说明；不渲染未知服务器原文。
    pub message: Option<String>,
    /// 失败阶段。
    pub stage: Option<String>,
    /// 关联标识。
    pub trace_id: Option<String>,
    /// 原计划编号。
    pub plan_id: Option<String>,
    /// 原请求编号。
    pub request_id: Option<String>,
    /// 安全恢复动作。
    pub next_action: Option<String>,
}

pub(super) async fn request<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T> {
    let mut response = request
        .send()
        .await
        .map_err(|_| super::super::transport::network())?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| super::super::transport::network())?
    {
        if bytes.len() + chunk.len() > 3145728 {
            return Err(crate::http::protocol());
        }
        bytes.extend_from_slice(&chunk);
    }
    if status.is_success() {
        return serde_json::from_slice(&bytes).map_err(|_| crate::http::protocol());
    }
    let error: RemoteError = serde_json::from_slice(&bytes).map_err(|_| crate::http::protocol())?;
    let (code, exit, message) = match error.code.as_str() {
        "AUTH_REQUIRED" => ("AUTH_REQUIRED", 4, "管理会话已失效，请重新登录。"),
        "AUTH_DENIED" | "PLATFORM_ROLE_REQUIRED" => (
            "AUTH_DENIED",
            4,
            "当前管理角色、环境或对象归属不允许此操作。",
        ),
        "PLAN_DIGEST_MISMATCH" => (
            "PLAN_DIGEST_MISMATCH",
            2,
            "核对摘要不符，请重新读取原计划。",
        ),
        "PLAN_EXPIRED" => ("PLAN_EXPIRED", 2, "计划已过期，请生成并核对新计划。"),
        "PLAN_PRECONDITION_CHANGED" | "PLAN_VERSION_CHANGED" => (
            "PLAN_PRECONDITION_CHANGED",
            2,
            "原计划前提已变化，整批未应用。",
        ),
        "PLAN_NOT_READY" | "BATCH_NOT_READY" => ("PLAN_NOT_READY", 2, "计划仍有配置缺项，未应用。"),
        "EXECUTION_REQUEST_CONFLICT" => (
            "EXECUTION_REQUEST_CONFLICT",
            2,
            "执行请求号已绑定，请回读原记录。",
        ),
        "EXECUTION_LEASE_CHANGED" => (
            "EXECUTION_LEASE_CHANGED",
            5,
            "执行资格已变化，请回读原计划。",
        ),
        "MANAGEMENT_SECRET_FORBIDDEN" => {
            ("MANAGEMENT_SECRET_FORBIDDEN", 2, "清单不能包含认证秘密。")
        }
        "MANAGEMENT_INPUT_INVALID" | "INVALID_ARGUMENT" => (
            "MANAGEMENT_INPUT_INVALID",
            2,
            "管理输入有未知字段或无效内容。",
        ),
        "SOURCE_SELECTION_AMBIGUOUS" | "OPERATION_IDENTITY_AMBIGUOUS" => (
            "SOURCE_SELECTION_AMBIGUOUS",
            2,
            "所选方法路径缺失或不唯一，请明确选择。",
        ),
        "SOURCE_UNAVAILABLE" | "PLAN_STORAGE_UNAVAILABLE" | "DEPENDENCY_UNAVAILABLE" => (
            "DEPENDENCY_UNAVAILABLE",
            5,
            "管理依赖暂不可用，请保留原计划并回读。",
        ),
        "ALIAS_ALREADY_EXISTS" => (
            "ALIAS_ALREADY_EXISTS",
            2,
            "同系统环境的别名已经占用，请明确选择另一别名。",
        ),
        "GRANT_RECONFIRMATION_REQUIRED" => (
            "GRANT_RECONFIRMATION_REQUIRED",
            2,
            "授权范围变化，须重新明确授权。",
        ),
        _ => (
            "MANAGEMENT_PROTOCOL_ERROR",
            6,
            "服务端管理能力不支持此输入或协议，请核对能力说明。",
        ),
    };
    let mut failure = Failure::new(code, exit, message);
    failure.trace_id = error.trace_id.clone();
    failure.details = Some(serde_json::to_value(&error).map_err(|_| crate::http::protocol())?);
    Err(failure)
}
