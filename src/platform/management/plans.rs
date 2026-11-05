//! 原计划与请求号的本地恢复；未知结果只回读，不盲目换号重发。
use super::super::{handle, model::PlatformProfile, serialize, session, transport};
use super::{http, model::*};
use crate::{
    Runtime,
    output::{Failure, Result, invalid},
    private_store,
};
use std::path::PathBuf;

fn location(rt: &Runtime, id: &str) -> Result<PathBuf> {
    if !handle(id) {
        return Err(invalid());
    }
    Ok(rt
        .root
        .join("platform-management-executions")
        .join(format!("{id}.json")))
}
fn validate(stored: &PlatformProfile, record: &LocalExecution) -> Result<()> {
    if record.schema_version != 1
        || record.origin != stored.origin
        || record.subject_id != stored.subject_id
        || record.environment != stored.environment
        || !handle(&record.request_id)
    {
        return Err(Failure::new(
            "MANAGEMENT_CONTEXT_MISMATCH",
            4,
            "原计划记录属于其他身份或环境。",
        ));
    }
    Ok(())
}
pub(super) async fn plan(rt: &Runtime, stored: &PlatformProfile, file: &str) -> Result<Plan> {
    let bytes = std::fs::read(file).map_err(|_| invalid())?;
    if bytes.len() > 3145728 {
        return Err(invalid());
    }
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|_| {
        Failure::new(
            "MANAGEMENT_INPUT_INVALID",
            2,
            "清单有未知字段或类型错误，请按管理协议填写。",
        )
    })?;
    if manifest.protocol_version != 1
        || manifest.environment != stored.environment
        || manifest.system.environment != stored.environment
        || manifest.selections.is_empty()
        || manifest.selections.len() > 100
    {
        return Err(invalid());
    }
    let credential = session::access(rt, stored).await?;
    // 发送原始传输字节，使服务端严格解码继续拒绝嵌套未知字段，不在重序列化中丢弃。
    let planned: Plan = http::request(
        transport::client()?
            .post(format!("{}/platform/management/plans", stored.origin))
            .bearer_auth(&credential.access_token)
            .header("Content-Type", "application/json")
            .body(bytes),
    )
    .await?;
    validate_plan(stored, &planned)?;
    Ok(planned)
}
fn validate_plan(stored: &PlatformProfile, plan: &Plan) -> Result<()> {
    if plan.protocol_version != 1
        || !handle(&plan.plan_id)
        || plan.digest.len() != 64
        || plan.environment != stored.environment
        || plan.actor_id != format!("EMPLOYEE:{}", stored.subject_id)
        || plan.snapshot.system.environment != stored.environment
    {
        return Err(crate::http::protocol());
    }
    Ok(())
}
pub(super) async fn inspect(rt: &Runtime, stored: &PlatformProfile, id: &str) -> Result<Plan> {
    location(rt, id)?;
    let credential = session::access(rt, stored).await?;
    let plan: Plan = http::request(
        transport::client()?
            .get(format!("{}/platform/management/plans/{id}", stored.origin))
            .bearer_auth(&credential.access_token),
    )
    .await?;
    validate_plan(stored, &plan)?;
    if plan.plan_id != id {
        return Err(crate::http::protocol());
    }
    Ok(plan)
}
fn validate_result(
    stored: &PlatformProfile,
    id: &str,
    digest: Option<&str>,
    result: &Application,
) -> Result<()> {
    if result.protocol_version != 1
        || result.plan_id != id
        || digest.is_some_and(|d| d != result.digest)
        || result
            .published
            .iter()
            .any(|p| p.contract.environment != stored.environment)
    {
        return Err(crate::http::protocol());
    }
    Ok(())
}
async fn remote_status(
    stored: &PlatformProfile,
    id: &str,
    credential: &str,
) -> Result<Application> {
    http::request(
        transport::client()?
            .get(format!(
                "{}/platform/management/plans/{id}/status",
                stored.origin
            ))
            .bearer_auth(credential),
    )
    .await
}
pub(super) async fn status(
    rt: &Runtime,
    stored: &PlatformProfile,
    id: &str,
) -> Result<Application> {
    let path = location(rt, id)?;
    let _lock = crate::profile::lock(&rt.root, &format!("management-plan-{id}")).await?;
    let mut local: Option<LocalExecution> = private_store::read(&path)?;
    if let Some(record) = &local {
        validate(stored, record)?;
    }
    let credential = session::access(rt, stored).await?;
    let result = remote_status(stored, id, &credential.access_token).await?;
    validate_result(
        stored,
        id,
        local.as_ref().map(|r| r.digest.as_str()),
        &result,
    )?;
    if let Some(record) = &mut local {
        reconcile_request(record, &result)?;
        record.state = result.state.clone();
        private_store::write(&path, record)?;
    }
    Ok(result)
}
pub(super) async fn apply(
    rt: &Runtime,
    stored: &PlatformProfile,
    id: &str,
    digest: &str,
    request_id: Option<&str>,
    retry: bool,
) -> Result<Application> {
    let path = location(rt, id)?;
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let _lock = crate::profile::lock(&rt.root, &format!("management-plan-{id}")).await?;
    let existing: Option<LocalExecution> = private_store::read(&path)?;
    let credential = session::access(rt, stored).await?;
    let mut record = if let Some(record) = existing {
        validate(stored, &record)?;
        if record.digest != digest || request_id.is_some_and(|r| r != record.request_id) {
            return Err(Failure::new(
                "EXECUTION_REQUEST_CONFLICT",
                2,
                "已保存原执行内容，请回读原计划。",
            ));
        }
        let authority = remote_status(stored, id, &credential.access_token).await?;
        validate_result(stored, id, Some(digest), &authority)?;
        if authority.state == "APPLIED" {
            return Ok(authority);
        }
        if !retry
            || !["RETRYABLE", "PROCESSING", "READY"].contains(&authority.state.as_str())
            || (authority.state == "PROCESSING"
                && authority.next_action != "RETRY_ORIGINAL_REQUEST")
        {
            return Ok(authority);
        }
        record
    } else {
        let plan = inspect(rt, stored, id).await?;
        if plan.digest != digest || plan.state != "READY" {
            return Err(Failure::new(
                "PLAN_NOT_READY",
                2,
                "计划摘要不符、过期或仍有缺项，未应用。",
            ));
        }
        let request_id = request_id
            .map(str::to_owned)
            .unwrap_or_else(crate::login::random);
        if !handle(&request_id) {
            return Err(invalid());
        }
        LocalExecution {
            schema_version: 1,
            origin: stored.origin.clone(),
            subject_id: stored.subject_id.clone(),
            environment: stored.environment.clone(),
            plan_id: id.into(),
            digest: digest.into(),
            request_id,
            state: "UNKNOWN".into(),
        }
    };
    record.state = "UNKNOWN".into();
    private_store::write(&path, &record)?;
    let response: Result<Application> = http::request(
        transport::client()?
            .post(format!(
                "{}/platform/management/plans/{id}/apply",
                stored.origin
            ))
            .bearer_auth(&credential.access_token)
            .json(&Apply {
                digest,
                request_id: &record.request_id,
            }),
    )
    .await;
    let result = match response {
        Ok(result) => result,
        Err(error) if ["NETWORK_ERROR", "PROTOCOL_ERROR"].contains(&error.code) => {
            if let Ok(authority) = remote_status(stored, id, &credential.access_token).await
                && authority.state == "APPLIED"
            {
                authority
            } else {
                let mut unknown = Failure::new(
                    "APPLICATION_RESULT_UNKNOWN",
                    5,
                    "应用结果尚未确认，请用原计划编号查询状态。",
                );
                unknown.details = Some(serialize(&record)?);
                return Err(unknown);
            }
        }
        Err(mut error) => {
            error.recovery = Some(Box::new(serialize(&record)?));
            return Err(error);
        }
    };
    validate_result(stored, id, Some(digest), &result)?;
    reconcile_request(&mut record, &result)?;
    record.state = result.state.clone();
    if let Err(mut error) = private_store::write(&path, &record) {
        error.partial_data = Some(Box::new(serialize(&result)?));
        error.recovery = Some(Box::new(serialize(&record)?));
        return Err(error);
    }
    Ok(result)
}

// 同一计划已由另一个请求完成时，回读并保存权威原请求号，不生成另一份主数据。
fn reconcile_request(record: &mut LocalExecution, result: &Application) -> Result<()> {
    if let Some(request) = &result.request_id {
        if !handle(request) {
            return Err(crate::http::protocol());
        }
        if request != &record.request_id {
            if result.state != "APPLIED" {
                return Err(crate::http::protocol());
            }
            record.request_id = request.clone();
        }
    }
    Ok(())
}
