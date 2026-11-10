//! 本地写入记录先落盘；恢复只核对原意图，不自动重发。
use crate::output::{PartialData, PartialMeta};
mod fingerprint;
pub(crate) mod model;
#[cfg(test)]
mod recovery_tests;
use super::{
    catalog, contract::Effect, executor, input_model::InvocationInput, invocation,
    model::PlatformProfile, records, session, transport,
};
use crate::{
    Runtime,
    output::{Failure, Result, invalid},
};
use model::*;
use serde::Serialize;
const LIMIT: u64 = 8388608;
async fn load(rt: &Runtime, id: &str) -> Result<Record> {
    let path = records::path(rt, "writes", id)?;
    let row: Record = records::read(rt, &path, LIMIT)
        .await?
        .ok_or_else(|| Failure::new("WRITE_RECORD_NOT_FOUND", 6, "本地写入记录不存在。"))?;
    if row.schema_version != 1 || row.id != id {
        return Err(crate::private_store::unavailable());
    }
    Ok(row)
}
async fn save(rt: &Runtime, record: &Record) -> Result<()> {
    records::write(rt, &records::path(rt, "writes", &record.id)?, record, LIMIT).await
}
fn view(row: Record, message: &str) -> View {
    View {
        record_id: row.id,
        stage: row.stage,
        intent: row.intent,
        result: row.result,
        message: message.into(),
    }
}
fn value(row: Record, message: &str) -> Result<serde_json::Value> {
    super::serialize(view(row, message))
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Prepare<'a> {
    /// 原持久化业务指令标识。
    client_request_id: &'a str,
    /// 用户明确业务指令。
    instruction: &'a str,
    /// 原固定环境。
    environment: &'a str,
    /// 原稳定接口标识。
    operation_id: &'a str,
    /// 原执行版本。
    operation_version: u64,
    /// 原契约摘要。
    contract_digest: &'a str,
    /// 原完整业务参数，仅供验证后形成摘要。
    invocation: &'a super::input_model::WireInvocation,
}
async fn recover_prepare(rt: &Runtime, row: &mut Record) -> Result<()> {
    let credential = session::access(rt, &row.profile).await?;
    let intent: Intent = transport::request(
        transport::client()?
            .post(format!("{}/platform/cli/writes", row.profile.origin))
            .bearer_auth(&credential.access_token)
            .json(&Prepare {
                client_request_id: &row.id,
                instruction: &row.instruction,
                environment: &row.profile.environment,
                operation_id: &row.published.contract.operation_id,
                operation_version: row.published.contract.operation_version,
                contract_digest: &row.published.digest,
                invocation: &row.request,
            }),
    )
    .await?;
    fingerprint::validate(row, &intent)?;
    row.stage = stage(&intent.state);
    row.intent = Some(intent);
    save(rt, row).await
}
fn stage(state: &WriteState) -> Stage {
    match state {
        WriteState::Prepared => Stage::Prepared,
        WriteState::Authorized => Stage::Authorized,
        WriteState::Succeeded | WriteState::Failed => Stage::Finished,
        _ => Stage::ResultUnknown,
    }
}
async fn observe(rt: &Runtime, row: &mut Record) -> Result<()> {
    if row.intent.is_none() {
        return recover_prepare(rt, row).await;
    }
    let profile = row.profile.clone();
    observe_with_profile(rt, row, &profile).await
}
async fn observe_with_profile(
    rt: &Runtime,
    row: &mut Record,
    profile: &PlatformProfile,
) -> Result<()> {
    let credential = session::access(rt, profile).await?;
    let id = &row.intent.as_ref().unwrap().id;
    let intent: Intent = transport::request(
        transport::client()?
            .get(format!("{}/platform/cli/writes/{id}", profile.origin))
            .bearer_auth(&credential.access_token),
    )
    .await?;
    fingerprint::validate(row, &intent)?;
    row.stage = match (&row.stage, &intent.state) {
        (
            Stage::DispatchAttempted | Stage::ResultUnknown,
            WriteState::Prepared
            | WriteState::Authorized
            | WriteState::Dispatched
            | WriteState::ResultUnknown,
        ) => Stage::ResultUnknown,
        (Stage::Finished, _) => Stage::Finished,
        _ => stage(&intent.state),
    };
    row.intent = Some(intent);
    save(rt, row).await
}
pub(super) async fn prepare(
    rt: &Runtime,
    name: &str,
    profile: &PlatformProfile,
    operation: &str,
    version: u64,
    input: InvocationInput,
    instruction: &str,
) -> Result<serde_json::Value> {
    if instruction.trim().is_empty() || instruction.chars().count() > 1000 {
        return Err(invalid());
    }
    // 固定契约与本人会话均须远端核验；相互独立的读取并发，结果仍逐项检查。
    let (published, identity) = tokio::join!(
        catalog::contract(rt, profile, operation, version),
        session::me(rt, profile),
    );
    let published = published?;
    if published.contract.effect != Effect::Write {
        return Err(Failure::new(
            "READ_OPERATION",
            2,
            "此接口是读取，请使用 call 或本机分页任务。",
        ));
    }
    let mut request = invocation::encode(&published.contract, &input)?;
    request.view = executor::choose_view(rt, profile, &published.contract, request.view).await?;
    identity?;
    let id = crate::login::random();
    let _lock = records::acquire(rt, "writes", &id).await?;
    let mut row = Record {
        schema_version: 1,
        id,
        profile_name: name.into(),
        profile: profile.clone(),
        published,
        request,
        instruction: instruction.into(),
        stage: Stage::Preparing,
        intent: None,
        result: None,
    };
    save(rt, &row).await?;
    records::created("platform.write.created", &row.id)?;
    if let Err(failure) = recover_prepare(rt, &mut row).await {
        let local_id = row.id.clone();
        return Err(failure.with_partial(
            PartialData::Write(view(
                row,
                "准备响应未确认；本地记录已保存，查询原记录恢复。",
            )),
            PartialMeta {
                record_id: Some(local_id),
                business_dispatch: Some(false),
                ..Default::default()
            },
        ));
    }
    value(
        row,
        "尚未派发。请 inspect 核对完整参数，并用 confirm 明确确认原指纹。",
    )
}
pub(super) async fn status(
    rt: &Runtime,
    profile: &PlatformProfile,
    id: &str,
) -> Result<serde_json::Value> {
    let _lock = records::acquire(rt, "writes", id).await?;
    let mut row = load(rt, id).await?;
    records::same_profile(&row.profile, profile)?;
    super::metadata::invalidate(profile);
    // 不缓存或省略恢复许可；三项只读核验全部结束后才能回读、持久化原意图。
    let (identity, current, view) = tokio::join!(
        session::me(rt, profile),
        catalog::contract(
            rt,
            profile,
            &row.published.contract.operation_id,
            row.published.contract.operation_version,
        ),
        executor::revalidate_view(rt, profile, &row.published.contract, &row.request.view),
    );
    identity?;
    let current = current?;
    if current.digest != row.published.digest {
        return Err(Failure::new(
            "EXECUTION_CONTRACT_MISMATCH",
            4,
            "原写入版本或摘要不可用，只保留本地记录。",
        ));
    }
    view?;
    // 新登录仅用于核对本人原意图，绝不恢复旧许可或再次派发。
    if row.intent.is_none() && row.profile.session_id != profile.session_id {
        return Err(Failure::new(
            "WRITE_PREPARATION_UNCONFIRMED",
            4,
            "原准备结果未确认，重新登录后不自动重建意图。",
        ));
    }
    if row.intent.is_none() {
        recover_prepare(rt, &mut row).await?;
        return value(row, "已核对原准备请求，未派发业务请求。");
    }
    observe_with_profile(rt, &mut row, profile).await?;
    value(row, "已核对原意图状态，未派发业务请求。")
}
pub(super) async fn inspect(rt: &Runtime, id: &str) -> Result<serde_json::Value> {
    let row = load(rt, id).await?;
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Review<'a> {
        /// 本地记录标识。
        record_id: &'a str,
        /// 用户明确的原业务指令。
        instruction: &'a str,
        /// 固定接口标识。
        operation_id: &'a str,
        /// 固定执行版本。
        operation_version: u64,
        /// 固定契约摘要。
        contract_digest: &'a str,
        /// 待明确确认的完整参数与身份指纹。
        fingerprint: String,
        /// 原完整业务参数，绝无受服务端管理的业务token。
        request: &'a super::input_model::WireInvocation,
    }
    super::serialize(Review {
        record_id: &row.id,
        instruction: &row.instruction,
        operation_id: &row.published.contract.operation_id,
        operation_version: row.published.contract.operation_version,
        contract_digest: &row.published.digest,
        fingerprint: fingerprint::compute(&row)?,
        request: &row.request,
    })
}
pub(super) async fn confirm(
    rt: &Runtime,
    profile: &PlatformProfile,
    id: &str,
    expected: &str,
) -> Result<serde_json::Value> {
    let _lock = records::acquire(rt, "writes", id).await?;
    let mut row = load(rt, id).await?;
    records::same_profile(&row.profile, profile)?;
    same_session(&row, profile)?;
    if expected != fingerprint::compute(&row)? {
        return Err(Failure::new(
            "WRITE_CONFIRMATION_MISMATCH",
            4,
            "确认指纹与原完整参数不同，未请求写入许可。",
        ));
    }
    if matches!(
        row.stage,
        Stage::DispatchAttempted | Stage::ResultUnknown | Stage::Finished
    ) {
        return value(row, "已记录派发尝试，不能再次确认；仅核对原结果。");
    }
    if row.intent.is_none() {
        recover_prepare(rt, &mut row).await?;
    }
    let intent = row.intent.as_ref().unwrap();
    if !matches!(intent.state, WriteState::Prepared | WriteState::Authorized) {
        return value(row, "原意图已派发或结果待核对，未再次授权。");
    }
    if intent.session_id != profile.session_id {
        return Err(Failure::new(
            "WRITE_SESSION_CHANGED",
            4,
            "原意图属于另一个会话，只能核对结果。",
        ));
    }
    row.stage = Stage::Authorizing;
    save(rt, &row).await?;
    #[derive(Serialize)]
    struct Confirmation<'a> {
        /// 用户明确确认的原参数指纹。
        fingerprint: &'a str,
    }
    let credentials = session::access(rt, profile).await?;
    let updated: Intent = transport::request(
        transport::client()?
            .post(format!(
                "{}/platform/cli/writes/{}/confirm",
                profile.origin,
                row.intent.as_ref().unwrap().id
            ))
            .bearer_auth(&credentials.access_token)
            .json(&Confirmation {
                fingerprint: expected,
            }),
    )
    .await?;
    fingerprint::validate(&row, &updated)?;
    row.stage = stage(&updated.state);
    row.intent = Some(updated);
    save(rt, &row).await?;
    value(row, "已明确确认原参数；尚未派发业务请求。")
}
pub(super) async fn dispatch(
    rt: &Runtime,
    profile: &PlatformProfile,
    id: &str,
) -> Result<serde_json::Value> {
    let _lock = records::acquire(rt, "writes", id).await?;
    let mut row = load(rt, id).await?;
    records::same_profile(&row.profile, profile)?;
    same_session(&row, profile)?;
    if matches!(
        row.stage,
        Stage::DispatchAttempted | Stage::ResultUnknown | Stage::Finished
    ) {
        observe(rt, &mut row).await?;
        return value(
            row,
            "仅核对原意图，未再次派发。结果不明时请核对真实业务状态。",
        );
    }
    observe(rt, &mut row).await?;
    let intent = row.intent.as_ref().unwrap();
    if intent.state != WriteState::Authorized || intent.session_id != profile.session_id {
        return value(row, "此原意图当前不允许派发，未发出业务请求。");
    }
    let current = catalog::contract(
        rt,
        profile,
        &row.published.contract.operation_id,
        row.published.contract.operation_version,
    )
    .await?;
    if current.digest != row.published.digest {
        return Err(Failure::new(
            "EXECUTION_CONTRACT_MISMATCH",
            4,
            "原执行定义不一致，未派发。",
        ));
    }
    let remote_id = intent.id.clone();
    row.stage = Stage::DispatchAttempted;
    save(rt, &row).await?;
    match executor::execute(
        rt,
        profile,
        &row.published,
        &row.request,
        Some(&remote_id),
        row.published.contract.limits.max_seconds.into(),
    )
    .await
    {
        Ok(result) => {
            row.stage = match result.write_state.as_deref() {
                Some("SUCCEEDED" | "FAILED") => Stage::Finished,
                _ => Stage::ResultUnknown,
            };
            row.result = Some(result);
            save(rt, &row).await?;
            if observe(rt, &mut row).await.is_err() {
                row.stage = Stage::ResultUnknown;
                save(rt, &row).await?;
            }
            value(row, "本地保留原业务响应；后续只核对原意图，不自动重发。")
        }
        Err(mut failure) => {
            failure.retryable = false;
            row.stage = Stage::ResultUnknown;
            save(rt, &row).await?;
            let record_id = row.id.clone();
            Err(failure.with_partial(
                PartialData::Write(view(row, "已记录派发尝试，结果待核对，禁止自动重发。")),
                PartialMeta {
                    record_id: Some(record_id),
                    retry_business_request: Some(false),
                    ..Default::default()
                },
            ))
        }
    }
}

pub(super) async fn list(rt: &Runtime, page: u32) -> Result<serde_json::Value> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Item {
        /// 本地写入记录标识。
        record_id: String,
        /// 固定接口标识。
        operation_id: String,
        /// 固定执行版本。
        operation_version: u64,
        /// 本地阶段，未冒充远端实时状态。
        stage: Stage,
        /// 原服务端意图标识，可空。
        intent_id: Option<String>,
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Listing {
        /// 当前页本地写入记录。
        items: Vec<Item>,
        /// 记录总数。
        total: usize,
        /// 当前页码。
        page: u32,
        /// 每页数量。
        page_size: u32,
    }
    let index = records::index(rt, "writes", page)?;
    let mut items = Vec::new();
    for id in index.ids {
        let row = load(rt, &id).await?;
        items.push(Item {
            record_id: row.id,
            operation_id: row.published.contract.operation_id,
            operation_version: row.published.contract.operation_version,
            stage: row.stage,
            intent_id: row.intent.map(|intent| intent.id),
        });
    }
    super::serialize(Listing {
        items,
        total: index.total,
        page: index.page,
        page_size: 20,
    })
}

fn same_session(row: &Record, profile: &PlatformProfile) -> Result<()> {
    if row.profile.session_id != profile.session_id {
        return Err(Failure::new(
            "WRITE_SESSION_CHANGED",
            4,
            "新会话只能查询原写入状态，不能恢复旧许可或再次派发。",
        ));
    }
    Ok(())
}

pub(super) async fn recovery(rt: &Runtime, id: &str) -> Result<crate::output::Recovery> {
    let row = load(rt, id).await?;
    let mut recovery = super::recovery::bound(&row.profile_name, &row.profile);
    recovery.record_id = Some(row.id.clone());
    recovery.actions.push(crate::output::RecoveryAction {
        id: "inspect-original-writes",
        argv: vec![
            "dt-cli".into(),
            "writes".into(),
            "status".into(),
            "--id".into(),
            id.into(),
            format!("--profile={}", row.profile_name),
        ],
        requires_interaction: false,
        reason: "仅核对原记录与检查点，不派发新的业务请求。",
    });
    Ok(recovery)
}
