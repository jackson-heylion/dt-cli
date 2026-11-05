use super::super::{contract::Location, executor, input_model::TaskView, model::IdentityDomain};
use super::model::{Intent, Record};
use crate::output::{Failure, Result};
use serde::Serialize;
#[derive(Serialize)]
struct HeaderValue {
    /// 已声明业务头小写名称。
    name: String,
    /// 实际发送的业务头值。
    value: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RequestBinding<'a> {
    /// 原规范实际路径。
    path: &'a str,
    /// 原编码查询串。
    query: &'a str,
    /// 按声明业务头构建的摘要。
    headers_digest: String,
    /// 原请求体字节摘要。
    body_digest: String,
    /// 原任务固定视角，可空。
    view: &'a Option<TaskView>,
}
#[derive(Serialize)]
struct Context<'a> {
    /// 原IAM人员标识。
    subject: &'a str,
    /// 原身份域。
    domain: &'a IdentityDomain,
    /// 原系统。
    system: &'a str,
    /// 原环境。
    environment: &'a str,
    /// 原接口标识。
    operation: &'a str,
    /// 原执行版本。
    version: u64,
    /// 原契约摘要。
    digest: &'a str,
    /// 原完整参数绑定。
    request: RequestBinding<'a>,
}
pub(super) fn compute(record: &Record) -> Result<String> {
    let c = &record.published.contract;
    let mut headers: Vec<_> = record
        .request
        .headers
        .iter()
        .filter(|h| {
            c.parameters
                .iter()
                .any(|p| p.location == Location::Header && p.name.eq_ignore_ascii_case(&h.name))
        })
        .map(|h| HeaderValue {
            name: h.name.to_ascii_lowercase(),
            value: h.value.clone(),
        })
        .collect();
    headers.sort_by(|a, b| a.name.cmp(&b.name));
    let headers_digest =
        executor::digest(&serde_json::to_vec(&headers).map_err(|_| crate::http::protocol())?);
    let request = RequestBinding {
        path: &record.request.path,
        query: &record.request.query,
        headers_digest,
        body_digest: executor::digest(record.request.body.as_bytes()),
        view: &record.request.view,
    };
    let context = Context {
        subject: &record.profile.subject_id,
        domain: &record.profile.domain,
        system: &c.system_id,
        environment: &c.environment,
        operation: &c.operation_id,
        version: c.operation_version,
        digest: &record.published.digest,
        request,
    };
    Ok(executor::digest(
        &serde_json::to_vec(&context).map_err(|_| crate::http::protocol())?,
    ))
}
pub(super) fn validate(record: &Record, intent: &Intent) -> Result<()> {
    let c = &record.published.contract;
    if intent.subject_id != record.profile.subject_id
        || intent.system_id != c.system_id
        || intent.environment != c.environment
        || intent.operation_id != c.operation_id
        || intent.operation_version != c.operation_version
        || intent.contract_digest != record.published.digest
        || intent.fingerprint != compute(record)?
        || intent.view != record.request.view
    {
        return Err(Failure::new(
            "WRITE_CONTEXT_MISMATCH",
            4,
            "原写入意图与本地固定身份、版本或完整参数不一致，未派发。",
        ));
    }
    Ok(())
}
