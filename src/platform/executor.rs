//! 普通HTTP数据面；不自动重试业务请求，不接收业务凭证。
use super::{
    catalog, contract::*, input_model::*, invocation, model::PlatformProfile, session, transport,
};
use crate::output::{PartialData, PartialMeta};
use crate::{
    Runtime,
    output::{Failure, Result, invalid},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::time::Duration;

#[derive(Clone, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExecutionResult {
    /// 原稳定接口标识。
    pub operation_id: String,
    /// 原不可变执行版本。
    pub operation_version: u64,
    /// 原契约摘要。
    pub contract_digest: String,
    /// 实际业务HTTP状态。
    pub status: u16,
    /// 固定契约明确判定的业务成功，不等同HTTP成功。
    pub business_success: bool,
    /// 是否符合固定版本的响应Schema。
    pub contract_matched: bool,
    /// 完整业务响应原文，HEAD/空响应为空。
    pub body: String,
    /// 实际Gateway审计标识，可空。
    pub invocation_id: Option<String>,
    /// 受控写入状态，读取为空。
    pub write_state: Option<String>,
    /// 同一invocationId的客户端与服务端分段毫秒观测，历史记录可无此字段。
    #[serde(default)]
    pub timing: Option<super::timing::Timing>,
}
pub(super) fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}
pub(super) async fn choose_view(
    rt: &Runtime,
    profile: &PlatformProfile,
    c: &ExecutionContract,
    requested: Option<TaskView>,
) -> Result<Option<TaskView>> {
    if c.view_types.is_empty() {
        if requested.is_some() {
            return Err(invalid());
        }
        return Ok(None);
    }
    if let Some(view) = requested {
        if view.object_id.is_empty()
            || view.object_id.len() > 128
            || !view
                .object_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
        {
            return Err(invalid());
        }
        revalidate_view(rt, profile, c, &Some(view.clone())).await?;
        return Ok(Some(view));
    }
    let candidates = catalog::views_for(rt, profile, c).await?;
    let supported: Vec<_> = candidates.iter().filter(|v| v.supported).collect();
    if supported.len() == 1 {
        return Ok(Some(TaskView {
            view_id: supported[0].view_id,
            object_id: supported[0].object_id.clone(),
        }));
    }
    Err(Failure::new(
        "VIEW_REQUIRED",
        2,
        "请核对本人可用候选，再为当前任务明确选择视角与对象。",
    )
    .with_partial(
        PartialData::Views(candidates),
        PartialMeta {
            selection_required: Some(true),
            ..Default::default()
        },
    ))
}
pub(super) async fn execute(
    rt: &Runtime,
    profile: &PlatformProfile,
    published: &Published,
    request: &WireInvocation,
    intent: Option<&str>,
    seconds: u64,
) -> Result<ExecutionResult> {
    let started = std::time::Instant::now();
    let c = &published.contract;
    if c.effect == Effect::Write && intent.is_none() {
        return Err(Failure::new(
            "WRITE_INTENT_REQUIRED",
            4,
            "写入必须先准备并明确确认完整参数。",
        ));
    }
    if c.effect == Effect::Read && intent.is_some() {
        return Err(invalid());
    }
    let credentials = session::access(rt, profile).await?;
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Configuration {
        /// 平台部署方固定的专用Gateway基址。
        gateway_base_url: String,
    }
    let gateway_origin = match super::metadata::runtime(profile) {
        Some(origin) => origin,
        None => {
            let config: Configuration = transport::request(
                transport::client()?
                    .get(format!("{}/platform/cli/catalog/runtime", profile.origin))
                    .bearer_auth(&credentials.access_token),
            )
            .await?;
            transport::origin(&config.gateway_base_url)?;
            super::metadata::remember_runtime(profile, config.gateway_base_url.clone());
            config.gateway_base_url
        }
    };
    let url = format!(
        "{}/platform-gateway/{}/{}/{}{}{}",
        gateway_origin,
        c.environment,
        c.operation_id,
        c.operation_version,
        request.path,
        if request.query.is_empty() {
            String::new()
        } else {
            format!("?{}", request.query)
        }
    );
    let method = reqwest::Method::from_bytes(c.method.text().as_bytes()).map_err(|_| invalid())?;
    let mut outgoing = transport::client()?
        .request(method, url)
        .timeout(Duration::from_secs(seconds.clamp(1, 40)))
        .bearer_auth(&credentials.access_token);
    for header in &request.headers {
        outgoing = outgoing.header(&header.name, &header.value);
    }
    if !request.body.is_empty() {
        outgoing = outgoing
            .header(
                "Content-Type",
                match c.media {
                    Media::Json => "application/json",
                    Media::Form => "application/x-www-form-urlencoded",
                    Media::None => return Err(invalid()),
                },
            )
            .body(request.body.clone());
    }
    if let Some(view) = &request.view {
        outgoing = outgoing
            .header("X-Platform-View-Id", view.view_id)
            .header("X-Platform-View-Object", &view.object_id);
    }
    if let Some(id) = intent {
        outgoing = outgoing.header("X-Platform-Intent-Id", id);
    }
    let preparation_millis = super::timing::millis(started);
    let gateway_started = std::time::Instant::now();
    let mut response = outgoing.send().await.map_err(|_| {
        if c.effect == Effect::Read {
            super::errors::read_retry(transport::network())
        } else {
            transport::network()
        }
    })?;
    let status = response.status().as_u16();
    if response.headers().contains_key("X-Platform-Error-Code") {
        super::metadata::invalidate(profile);
        let header = |name: &str| response.headers().get(name).and_then(|v| v.to_str().ok());
        let failure = super::errors::gateway(
            status,
            header("X-Platform-Error-Code"),
            header("X-Platform-Error-Stage"),
            header("X-Platform-Trace-Id").or_else(|| header("X-Trace-Id")),
        );
        return Err(if c.effect == Effect::Read {
            super::errors::read_retry(failure)
        } else {
            failure
        });
    }
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .filter(|v| v.len() < 128)
            .map(str::to_owned)
    };
    let gateway_timing = super::timing::parse(
        response
            .headers()
            .get("Server-Timing")
            .and_then(|v| v.to_str().ok()),
    );
    let invocation_id = header("X-Platform-Invocation");
    let write_state = header("X-Platform-Write-State");
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| transport::network())? {
        if bytes.len() + chunk.len() > 2097152
            || bytes.len() as u64 + chunk.len() as u64 > c.limits.max_bytes
        {
            return Err(Failure::new(
                "RESPONSE_LIMIT",
                6,
                "业务响应超过当前执行限额。",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let gateway_request_millis = super::timing::millis(gateway_started);
    let body = String::from_utf8(bytes).map_err(|_| crate::http::protocol())?;
    let accepted = c.success.status_codes.contains(&status);
    let mut contract_matched = true;
    if accepted && !body.is_empty() {
        if let Some(schema) = &c.response_schema {
            contract_matched = invocation::validate(schema, &body).is_ok();
        } else {
            contract_matched = false;
        }
    }
    if accepted
        && body.is_empty()
        && c.method != Method::Head
        && status != 204
        && c.response_schema.is_some()
    {
        contract_matched = false;
    }
    let mut business_success = accepted && contract_matched;
    if let (Some(pointer), Some(expected)) = (&c.success.body_pointer, &c.success.expected_json) {
        // JSON树只用于已声明Pointer的传输结果判定，不进入领域模型。
        let actual = serde_json::from_str::<serde_json::Value>(&body);
        let expected = serde_json::from_str::<serde_json::Value>(expected);
        business_success &= match (actual, expected) {
            (Ok(actual), Ok(expected)) => actual.pointer(pointer) == Some(&expected),
            _ => false,
        };
    }
    if business_success {
        if let Some(view) = &request.view {
            let _ = super::view_memory::remember(rt, profile, &c.system_id, view);
        }
    }
    Ok(ExecutionResult {
        operation_id: c.operation_id.clone(),
        operation_version: c.operation_version,
        contract_digest: published.digest.clone(),
        status,
        business_success,
        contract_matched,
        body,
        invocation_id,
        write_state,
        timing: Some(super::timing::Timing {
            client_elapsed_millis: super::timing::millis(started),
            preparation_millis,
            gateway_request_millis,
            gateway: gateway_timing,
        }),
    })
}
pub(super) async fn read_input(
    rt: &Runtime,
    profile: &PlatformProfile,
    operation: &str,
    version: u64,
    input: InvocationInput,
) -> Result<ExecutionResult> {
    let started = std::time::Instant::now();
    let published = catalog::contract(rt, profile, operation, version).await?;
    if published.contract.effect != Effect::Read {
        return Err(Failure::new(
            "WRITE_INTENT_REQUIRED",
            4,
            "写入不能通过读取调用绕过准备与明确确认。",
        ));
    }
    let mut request = invocation::encode(&published.contract, &input)?;
    request.view = choose_view(rt, profile, &published.contract, request.view).await?;
    let preparation = super::timing::millis(started);
    let mut result = execute(
        rt,
        profile,
        &published,
        &request,
        None,
        published.contract.limits.max_seconds.into(),
    )
    .await?;
    if let Some(timing) = &mut result.timing {
        timing.preparation_millis += preparation;
        timing.client_elapsed_millis = super::timing::millis(started);
    }
    Ok(result)
}

pub(super) fn present(result: ExecutionResult) -> Result<serde_json::Value> {
    let success = result.business_success;
    let matched = result.contract_matched;
    let data = serde_json::to_value(&result).map_err(|_| crate::http::protocol())?;
    if success {
        Ok(data)
    } else {
        Err(Failure::new(
            if matched {
                "BUSINESS_FAILURE"
            } else {
                "EXECUTION_CONTRACT_MISMATCH"
            },
            6,
            if matched {
                "业务未满足固定契约的成功条件，请核对原始结果。"
            } else {
                "上游响应不再符合固定版本，已停止执行；请核对原结果，未切换新版本。"
            },
        )
        .with_partial(
            PartialData::Execution(result),
            PartialMeta {
                complete: Some(false),
                ..Default::default()
            },
        ))
    }
}

/// 恢复时重新查询当前候选，不能仅凭本地原视角继续。
pub(super) async fn revalidate_view(
    rt: &Runtime,
    profile: &PlatformProfile,
    c: &ExecutionContract,
    view: &Option<TaskView>,
) -> Result<()> {
    if c.view_types.is_empty() && view.is_none() {
        return Ok(());
    }
    let candidates = catalog::views(rt, profile, &c.operation_id, c.operation_version).await?;
    if view.as_ref().is_some_and(|selected| {
        candidates.iter().any(|candidate| {
            candidate.supported
                && candidate.view_id == selected.view_id
                && candidate.object_id == selected.object_id
        })
    }) {
        return Ok(());
    }
    Err(Failure::new(
        "TASK_VIEW_UNAVAILABLE",
        4,
        "原任务视角或对象权限已不可用，原进度保留。",
    ))
}
