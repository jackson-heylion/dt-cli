//! 受控协议错误：白名单分类独立于原请求重发许可。
use crate::output::{Details, Failure};
fn code(value: Option<&str>) -> Option<String> {
    value
        .filter(|v| {
            matches!(
                *v,
                "AUTH_REQUIRED"
                    | "AUTH_DENIED"
                    | "ACCESS_TOKEN_EXPIRED"
                    | "PLATFORM_ROLE_REQUIRED"
                    | "INTERFACE_PERMISSION_REQUIRED"
                    | "DATA_SCOPE_DENIED"
                    | "VIEW_REQUIRED"
                    | "VIEW_NOT_ALLOWED"
                    | "CONTRACT_MISMATCH"
                    | "EXECUTION_CONTRACT_MISMATCH"
                    | "INTERFACE_VERSION_CHANGED"
                    | "INVALID_ARGUMENT"
                    | "RESOURCE_UNAVAILABLE"
                    | "RATE_LIMITED"
                    | "RESPONSE_LIMIT"
                    | "DEPENDENCY_UNAVAILABLE"
                    | "UPSTREAM_UNAVAILABLE"
                    | "SOURCE_UNAVAILABLE"
                    | "PLAN_STORAGE_UNAVAILABLE"
                    | "PLAN_DIGEST_MISMATCH"
                    | "PLAN_EXPIRED"
                    | "PLAN_PRECONDITION_CHANGED"
                    | "PLAN_VERSION_CHANGED"
                    | "PLAN_NOT_READY"
                    | "BATCH_NOT_READY"
                    | "EXECUTION_REQUEST_CONFLICT"
                    | "EXECUTION_LEASE_CHANGED"
                    | "MANAGEMENT_SECRET_FORBIDDEN"
                    | "MANAGEMENT_INPUT_INVALID"
                    | "SOURCE_SELECTION_AMBIGUOUS"
                    | "OPERATION_IDENTITY_AMBIGUOUS"
                    | "ALIAS_ALREADY_EXISTS"
                    | "GRANT_RECONFIRMATION_REQUIRED"
                    | "WRITE_RESULT_UNKNOWN"
                    | "BUSINESS_CREDENTIAL_REJECTED"
                    | "DISCOVERY_UNAVAILABLE"
            )
        })
        .map(str::to_owned)
}
pub(super) fn trace(value: Option<&str>) -> Option<String> {
    value
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 128
                && !v.starts_with("dtp_")
                && !v.starts_with("dtcli_")
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
        .map(str::to_owned)
}
pub(super) fn details(
    status: u16,
    remote_code: Option<&str>,
    stage: Option<&str>,
    trace_id: Option<&str>,
) -> Details {
    Details {
        code: code(remote_code),
        http_status: Some(status),
        stage: stage
            .filter(|v| {
                matches!(
                    *v,
                    "DISCOVERY"
                        | "RESOLVE"
                        | "AUTHENTICATION"
                        | "AUTHORIZATION"
                        | "ROUTING"
                        | "GATEWAY"
                        | "UPSTREAM"
                        | "CONTRACT"
                        | "DISPATCH"
                        | "MANAGEMENT"
                        | "READ"
                        | "WRITE"
                )
            })
            .map(str::to_owned),
        trace_id: trace(trace_id),
        ..Default::default()
    }
}
pub(super) fn gateway(
    status: u16,
    remote_code: Option<&str>,
    stage: Option<&str>,
    trace_id: Option<&str>,
) -> Failure {
    let (category, exit, message) = match remote_code {
        Some("INTERFACE_PERMISSION_REQUIRED") => (
            "INTERFACE_PERMISSION_REQUIRED",
            4,
            "当前接口缺少执行许可，请联系平台管理员核对内部授权。",
        ),
        Some("DATA_SCOPE_DENIED") => ("DATA_SCOPE_DENIED", 4, "当前本人数据范围不允许此请求。"),
        Some("VIEW_REQUIRED" | "VIEW_NOT_ALLOWED") => (
            "VIEW_NOT_ALLOWED",
            4,
            "原任务视角不在当前合法范围，请核对原对象。",
        ),
        Some("CONTRACT_MISMATCH" | "EXECUTION_CONTRACT_MISMATCH" | "INTERFACE_VERSION_CHANGED") => {
            (
                "EXECUTION_CONTRACT_MISMATCH",
                4,
                "固定执行版本或契约已变化，未继续原请求。",
            )
        }
        Some("RESPONSE_LIMIT") => ("RESPONSE_LIMIT", 6, "业务响应超过当前执行限额。"),
        Some("DEPENDENCY_UNAVAILABLE" | "UPSTREAM_UNAVAILABLE") => (
            "DEPENDENCY_UNAVAILABLE",
            5,
            "执行依赖暂不可用，请核对原请求结果。",
        ),
        Some("BUSINESS_CREDENTIAL_REJECTED") => (
            "BUSINESS_CREDENTIAL_REJECTED",
            4,
            "本人业务身份暂不可用，请核对原系统。",
        ),
        _ => match status {
            401 => (
                "AUTH_REQUIRED",
                4,
                "平台会话已失效，请沿用原任务环境重新登录。",
            ),
            403 => ("AUTH_DENIED", 4, "原请求权限已被拒绝。"),
            429 => ("RATE_LIMITED", 5, "请求过于频繁，请保留原上下文。"),
            503 => (
                "DEPENDENCY_UNAVAILABLE",
                5,
                "执行依赖暂不可用，请核对原请求结果。",
            ),
            _ => (
                "GATEWAY_REQUEST_DENIED",
                6,
                "网关未确认请求可执行，请核对原记录。",
            ),
        },
    };
    let mut failure = Failure::new(category, exit, message);
    let fields = details(status, remote_code, stage, trace_id);
    failure.trace_id = fields.trace_id.clone();
    failure.details = Some(Box::new(fields));
    failure
}
pub(super) fn read_retry(mut failure: Failure) -> Failure {
    failure.retryable = matches!(
        failure.code,
        "NETWORK_ERROR" | "RATE_LIMITED" | "DEPENDENCY_UNAVAILABLE"
    ) && failure.details.as_ref().is_none_or(|d| {
        d.http_status.is_some()
            || d.reason.as_deref().is_some_and(|reason| {
                matches!(
                    reason,
                    "request_timeout"
                        | "proxy_connect_failed"
                        | "dns_resolution_failed"
                        | "tls_failed"
                        | "connection_failed"
                        | "unknown"
                )
            })
    });
    failure
}
