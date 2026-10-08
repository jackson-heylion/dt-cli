use crate::{
    credentials::Credentials,
    output::{Failure, Result},
    profile::Environment,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::time::Duration;

pub fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| Failure::new("INTERNAL_ERROR", 1, "无法建立 HTTP 客户端。"))
}
pub fn network() -> Failure {
    Failure::new("NETWORK_ERROR", 5, "IAM 请求失败；认证交换不会自动重试。")
}
pub async fn bounded(mut r: reqwest::Response) -> Result<Value> {
    let mut bytes = Vec::new();
    while let Some(c) = r.chunk().await.map_err(|_| network())? {
        if bytes.len() + c.len() > 2 * 1024 * 1024 {
            return Err(Failure::new(
                "DEPENDENCY_UNAVAILABLE",
                5,
                "IAM 响应超过协议上限。",
            ));
        }
        bytes.extend_from_slice(&c);
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| Failure::new("DEPENDENCY_UNAVAILABLE", 5, "IAM 响应不符合协议。"))
}
fn failure(status: u16, body: &Value) -> Failure {
    let (code, exit) = match status {
        400 => ("INVALID_ARGUMENT", 2),
        401 => match body["error"]["code"].as_str() {
            Some("ACCESS_TOKEN_EXPIRED") => ("ACCESS_TOKEN_EXPIRED", 3),
            Some("AUTHORIZATION_REVOKED") => ("AUTHORIZATION_REVOKED", 3),
            Some("AUTHORIZATION_EXPIRED") => ("AUTHORIZATION_EXPIRED", 3),
            _ => ("AUTH_REQUIRED", 3),
        },
        403 => ("AUTH_DENIED", 4),
        404 => ("RESOURCE_UNAVAILABLE", 6),
        409 => match body["error"]["code"].as_str() {
            Some("JUMP_UNAVAILABLE") => ("JUMP_UNAVAILABLE", 6),
            _ => ("RESOURCE_UNAVAILABLE", 6),
        },
        413 => ("RESULT_LIMIT", 7),
        500 => ("INTERNAL_ERROR", 1),
        429 | 503 => ("DEPENDENCY_UNAVAILABLE", 5),
        _ => ("DEPENDENCY_UNAVAILABLE", 5),
    };
    let mut f = Failure::new(code, exit, "IAM 未确认请求可用。");
    f.trace_id = safe_trace(body);
    f
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Subject {
    pub id: String,
    pub account: String,
    pub display_name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Me {
    pub subject: Subject,
    pub authorization_id: String,
    pub authorization_expires_at: String,
    pub allowed_operations: Vec<String>,
    pub checked_at: String,
}
fn safe_trace(body: &Value) -> Option<String> {
    body["meta"]["traceId"]
        .as_str()
        .filter(|s| {
            !s.is_empty() && s.len() <= 64 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        })
        .map(str::to_owned)
}
fn with_trace(mut failure: Failure, trace_id: &Option<String>) -> Failure {
    failure.trace_id = trace_id.clone();
    failure
}
pub async fn me(client: &reqwest::Client, env: &Environment, c: &Credentials) -> Result<Value> {
    let catalog = crate::catalog::Catalog::shared();
    let operation = catalog.operation("iam.profile.get")?;
    let route = operation.route.as_deref().ok_or_else(network)?;
    let method =
        reqwest::Method::from_bytes(operation.method.as_deref().ok_or_else(network)?.as_bytes())
            .map_err(|_| network())?;
    let r = client
        .request(
            method,
            format!("{}{}", env.api_origin.trim_end_matches('/'), route),
        )
        .bearer_auth(&c.access_token)
        .send()
        .await
        .map_err(|_| network())?;
    let status = r.status().as_u16();
    let body = bounded(r).await?;
    if status != 200 || body["ok"] != true {
        return Err(failure(status, &body));
    }
    if body["schemaVersion"] != "1.0"
        || body["operationId"] != "iam.profile.get"
        || !body["error"].is_null()
    {
        return Err(network());
    }
    let value = body["data"].clone();
    let parsed: Me = serde_json::from_value(value.clone()).map_err(|_| network())?;
    if parsed.subject.id.is_empty()
        || parsed.authorization_id.is_empty()
        || chrono::DateTime::parse_from_rfc3339(&parsed.checked_at).is_err()
        || chrono::DateTime::parse_from_rfc3339(&parsed.authorization_expires_at).is_err()
        || parsed.allowed_operations.iter().any(|o| {
            ![
                "iam.profile.get",
                "iam.apps.list",
                "iam.workflow.applications",
                "iam.workflow.form-types",
                "iam.workflow.list",
                "iam.workflow.open",
            ]
            .contains(&o.as_str())
        })
    {
        return Err(network());
    }
    let mut projected = json!({"subject":{"id":parsed.subject.id,"account":parsed.subject.account,"displayName":parsed.subject.display_name},"authorizationId":parsed.authorization_id,"authorizationExpiresAt":parsed.authorization_expires_at,"allowedOperations":parsed.allowed_operations,"checkedAt":parsed.checked_at});
    if let Some(trace) = safe_trace(&body) {
        projected["_traceId"] = json!(trace);
    }
    Ok(projected)
}
pub async fn operation(
    client: &reqwest::Client,
    env: &Environment,
    c: &Credentials,
    id: &str,
    params: Value,
) -> Result<Value> {
    let catalog = crate::catalog::Catalog::shared();
    let op = catalog.operation(id)?;
    let route = op.route.as_deref().ok_or_else(network)?;
    let method = reqwest::Method::from_bytes(op.method.as_deref().ok_or_else(network)?.as_bytes())
        .map_err(|_| network())?;
    let mut request = client
        .request(
            method.clone(),
            format!("{}{}", env.api_origin.trim_end_matches('/'), route),
        )
        .bearer_auth(&c.access_token);
    if method != reqwest::Method::GET {
        request = request.json(&params);
    }
    let r = request.send().await.map_err(|_| network())?;
    let status = r.status().as_u16();
    let body = bounded(r).await?;
    if status != 200 || body["ok"] != true {
        return Err(failure(status, &body));
    }
    if body["schemaVersion"] != "1.0" || body["operationId"] != id || !body["error"].is_null() {
        return Err(network());
    }
    let mut value = body["data"].clone();
    if let Some(object) = value.as_object_mut() {
        object.insert("_meta".into(), body["meta"].clone());
    } else {
        value = json!({"items":value,"_meta":body["meta"]});
    }
    Ok(value)
}
/// Byte ceilings: one page of the workflow projection, and one whole aggregation.
pub const PAGE_BYTE_LIMIT: usize = 2 * 1024 * 1024;
pub const AGGREGATE_BYTE_LIMIT: usize = 10 * 1024 * 1024;
/// Path of the CLI workflow message query; the aggregator always uses this route.
pub const WORKFLOW_QUERY_ROUTE: &str = "/cli/v1/workflow/messages/query";
pub const WORKFLOW_LIST_OPERATION: &str = "iam.workflow.list";

pub fn over_limit() -> Failure {
    Failure::new("RESULT_LIMIT", 7, "IAM 响应超过本次读取的字节上限。")
}
pub fn protocol() -> Failure {
    Failure::new("DEPENDENCY_UNAVAILABLE", 5, "IAM 响应不符合协议。")
}
pub fn pagination_unknown() -> Failure {
    Failure::new(
        "PAGINATION_UNKNOWN",
        7,
        "分页信息缺失、矛盾或异常空页；不能推断范围已取完。",
    )
}

mod launch;
mod oauth;
/// Pagination metadata with the four fields the protocol lets stay unknown.
mod workflow;
pub use launch::{Launch, launch};
pub use oauth::{Token, refresh, refresh_recoverable, revoke, token};
pub use workflow::{PageResponse, Pagination, Source, pagination_diagnostic, workflow_page};

use workflow::read_body;
