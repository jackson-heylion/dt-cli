//! 本机读取工作包：完整页先签名落盘，再推进检查点；没有服务端业务token。
mod model;
mod paging;
#[cfg(test)]
mod permission_tests;
use super::{
    catalog, contract::*, executor, input_model::InvocationInput, invocation,
    model::PlatformProfile, records, session,
};
use crate::{
    Runtime,
    output::{Failure, Result},
};
use model::*;
use std::time::Instant;
const JOB_LIMIT: u64 = 2097152;
const PAGE_LIMIT: u64 = 8388608;
fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
async fn load(rt: &Runtime, id: &str) -> Result<Job> {
    let job: Job = records::read(rt, &records::path(rt, "jobs", id)?, JOB_LIMIT)
        .await?
        .ok_or_else(|| Failure::new("JOB_NOT_FOUND", 6, "本机任务不存在。"))?;
    if job.schema_version != 1 || job.id != id {
        return Err(crate::private_store::unavailable());
    }
    Ok(job)
}
async fn save(rt: &Runtime, job: &mut Job) -> Result<()> {
    job.updated_at = now();
    records::write(rt, &records::path(rt, "jobs", &job.id)?, job, JOB_LIMIT).await
}
fn page_path(rt: &Runtime, id: &str, index: u32) -> Result<std::path::PathBuf> {
    records::id(id)?;
    Ok(rt
        .root
        .join("platform/jobs")
        .join(id)
        .join(format!("{index:08}.json")))
}
fn summary(job: &Job) -> Result<serde_json::Value> {
    let message = if job.reason.as_deref() == Some("INTERFACE_PERMISSION_REQUIRED") {
        "当前接口缺少执行许可，请联系平台管理员在接口工作区核对内部授权；已完成页保留。"
    } else {
        match job.state {
            JobState::Succeeded => "任务已按固定规则完成。分页结果分时取得，不承诺同一时点快照。",
            JobState::Incomplete => "任务不完整，已完成页保留。限额或契约问题未解除前不继续。",
            JobState::Paused => {
                "任务已暂停，已完成页保留。恢复前重新核对原身份、视角、授权和版本。"
            }
            JobState::Running => "任务运行中；可核对已签名落盘的页与检查点。",
        }
    };
    super::serialize(Summary {
        job_id: &job.id,
        operation_id: &job.published.contract.operation_id,
        operation_version: job.published.contract.operation_version,
        contract_digest: &job.published.digest,
        subject_id: &job.profile.subject_id,
        system_id: &job.published.contract.system_id,
        environment: &job.profile.environment,
        state: &job.state,
        progress: &job.progress,
        reason: &job.reason,
        complete: job.state == JobState::Succeeded,
        point_in_time_snapshot: false,
        message,
    })
}
pub(super) async fn start(
    rt: &Runtime,
    name: &str,
    profile: &PlatformProfile,
    operation: &str,
    version: u64,
    input: InvocationInput,
) -> Result<serde_json::Value> {
    let published = catalog::contract(rt, profile, operation, version).await?;
    if published.contract.effect != Effect::Read {
        return Err(Failure::new(
            "WRITE_INTENT_REQUIRED",
            4,
            "本机分页任务只执行明确读取，不接受写入接口。",
        ));
    }
    let mut request = invocation::encode(&published.contract, &input)?;
    request.view = executor::choose_view(rt, profile, &published.contract, request.view).await?;
    let progress = paging::initial(&published.contract, &request)?;
    session::me(rt, profile).await?;
    let id = crate::login::random();
    let _lock = records::acquire(rt, "jobs", &id).await?;
    let mut job = Job {
        schema_version: 1,
        id,
        profile_name: name.into(),
        profile: profile.clone(),
        published,
        request,
        progress,
        state: JobState::Running,
        reason: None,
        created_at: now(),
        updated_at: now(),
    };
    save(rt, &mut job).await?;
    records::created("platform.job.created", &job.id)?;
    run(rt, &mut job, false).await?;
    summary(&job)
}
pub(super) async fn resume(
    rt: &Runtime,
    profile: &PlatformProfile,
    id: &str,
) -> Result<serde_json::Value> {
    let _lock = records::acquire(rt, "jobs", id).await?;
    let mut job = load(rt, id).await?;
    records::same_profile(&job.profile, profile)?;
    if matches!(job.state, JobState::Succeeded | JobState::Incomplete) {
        return summary(&job);
    }
    // 每次恢复以服务器核验原版本，不从其他profile或新版本借用执行上下文。
    if let Err(failure) = session::me(rt, profile).await {
        pause(rt, &mut job, "AUTHORITY_CHECK_FAILED").await?;
        return Err(failure.with_partial(summary(&job)?, serde_json::json!({"complete":false})));
    }
    super::metadata::invalidate(profile);
    let current = match catalog::contract(
        rt,
        profile,
        &job.published.contract.operation_id,
        job.published.contract.operation_version,
    )
    .await
    {
        Ok(current) => current,
        Err(failure) => {
            pause(rt, &mut job, failure.code).await?;
            return summary(&job);
        }
    };
    if current.digest != job.published.digest {
        pause(rt, &mut job, "EXECUTION_CONTRACT_MISMATCH").await?;
        return Err(Failure::new(
            "EXECUTION_CONTRACT_MISMATCH",
            4,
            "原执行版本或摘要不可用，已完成页保留。",
        )
        .with_partial(summary(&job)?, serde_json::json!({"complete":false})));
    }
    if let Err(failure) =
        executor::revalidate_view(rt, profile, &job.published.contract, &job.request.view).await
    {
        pause(rt, &mut job, failure.code).await?;
        return Err(failure.with_partial(summary(&job)?, serde_json::json!({"complete":false})));
    }
    // 原进度保留，只替换经本人核验的新会话凭证引用。
    job.profile = profile.clone();
    let retry_failed = job.reason.as_deref() == Some("BUSINESS_FAILURE");
    job.state = JobState::Running;
    job.reason = None;
    save(rt, &mut job).await?;
    run(rt, &mut job, retry_failed).await?;
    summary(&job)
}
async fn pause(rt: &Runtime, job: &mut Job, reason: &str) -> Result<()> {
    job.state = JobState::Paused;
    job.reason = Some(reason.into());
    save(rt, job).await
}
async fn incomplete(rt: &Runtime, job: &mut Job, reason: &str) -> Result<()> {
    job.state = JobState::Incomplete;
    job.reason = Some(reason.into());
    save(rt, job).await
}
async fn run(rt: &Runtime, job: &mut Job, mut retry_failed: bool) -> Result<()> {
    let turn = Instant::now();
    loop {
        let limits = job.published.contract.limits.clone();
        if job.progress.pages >= limits.max_pages {
            return incomplete(rt, job, "PAGE_LIMIT").await;
        }
        if job.progress.records >= limits.max_records {
            return incomplete(rt, job, "RECORD_LIMIT").await;
        }
        if job.progress.bytes >= limits.max_bytes {
            return incomplete(rt, job, "BYTE_LIMIT").await;
        }
        if job.progress.active_millis >= u64::from(limits.max_seconds) * 1000 {
            return incomplete(rt, job, "TIME_LIMIT").await;
        }
        if turn.elapsed() >= rt.aggregate_budget {
            return pause(rt, job, "LOCAL_TIME_SLICE").await;
        }
        let current = match catalog::contract(
            rt,
            &job.profile,
            &job.published.contract.operation_id,
            job.published.contract.operation_version,
        )
        .await
        {
            Ok(value) => value,
            Err(_) => return pause(rt, job, "AUTHORITY_OR_VERSION_UNAVAILABLE").await,
        };
        if current.digest != job.published.digest {
            return incomplete(rt, job, "EXECUTION_CONTRACT_MISMATCH").await;
        }
        let request = match paging::request(&job.published.contract, &job.request, &job.progress) {
            Ok(value) => value,
            Err(failure) => return incomplete(rt, job, failure.code).await,
        };
        let request_digest =
            executor::digest(&serde_json::to_vec(&request).map_err(|_| crate::http::protocol())?);
        let index = job.progress.pages + 1;
        let path = page_path(rt, &job.id, index)?;
        let mut existing: Option<Page> = records::read(rt, &path, PAGE_LIMIT).await?;
        if retry_failed
            && existing
                .as_ref()
                .is_some_and(|page| !page.result.business_success)
        {
            let archived =
                path.with_file_name(format!("{index:08}-failed-{}.json", crate::login::random()));
            std::fs::rename(&path, archived).map_err(|_| crate::private_store::unavailable())?;
            existing = None;
        }
        retry_failed = false;
        let page = if let Some(page) = existing {
            if page.job_id != job.id
                || page.index != index
                || page.request_digest != request_digest
                || page.contract_digest != job.published.digest
            {
                return incomplete(rt, job, "LOCAL_PAGE_CONTEXT_MISMATCH").await;
            }
            if let Some(reserved) = job.progress.pending_budget_millis.take() {
                job.progress.active_millis =
                    job.progress.active_millis.saturating_sub(reserved) + page.elapsed_millis;
            }
            page
        } else {
            // 进程中断且没有完整页时，原预留预算继续计入，不因重启归零。
            job.progress.pending_budget_millis = None;
            let remaining = u64::from(limits.max_seconds) * 1000 - job.progress.active_millis;
            let turn_left = rt
                .aggregate_budget
                .saturating_sub(turn.elapsed())
                .as_millis()
                .min(u64::MAX as u128) as u64;
            let reserved = remaining.min(turn_left).min(40000);
            if reserved < 1000 {
                return if remaining < 1000 {
                    incomplete(rt, job, "TIME_LIMIT").await
                } else {
                    pause(rt, job, "LOCAL_TIME_SLICE").await
                };
            }
            job.progress.pending_budget_millis = Some(reserved);
            job.progress.active_millis += reserved;
            save(rt, job).await?;
            let started = Instant::now();
            let fetched = tokio::select! {
                result=executor::execute(rt,&job.profile,&job.published,&request,None,reserved/1000)=>result,
                _=tokio::signal::ctrl_c()=>return pause(rt,job,"PROCESS_INTERRUPTED").await,
            };
            let elapsed = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
            job.progress.active_millis =
                job.progress.active_millis.saturating_sub(reserved) + elapsed;
            job.progress.pending_budget_millis = None;
            let result = match fetched {
                Ok(value) => value,
                Err(failure) => {
                    // 已知响应限额使用任务不完整状态，保留原检查点。
                    if failure.code == "RESPONSE_LIMIT" {
                        return incomplete(rt, job, "RECORD_OR_BYTE_LIMIT").await;
                    }
                    let reason = if failure.code == "INTERFACE_PERMISSION_REQUIRED" {
                        "INTERFACE_PERMISSION_REQUIRED"
                    } else {
                        "READ_REQUEST_UNCONFIRMED"
                    };
                    return pause(rt, job, reason).await;
                }
            };
            let page = Page {
                job_id: job.id.clone(),
                index,
                contract_digest: job.published.digest.clone(),
                request_digest,
                result,
                elapsed_millis: elapsed,
                observed_at: now(),
            };
            records::write(rt, &path, &page, PAGE_LIMIT).await?;
            page
        };
        if !page.result.contract_matched {
            return incomplete(rt, job, "EXECUTION_CONTRACT_MISMATCH").await;
        }
        if !page.result.business_success {
            return pause(rt, job, "BUSINESS_FAILURE").await;
        }
        let observation =
            match paging::observe(&job.published.contract, &page.result.body, &job.progress) {
                Ok(value) => value,
                Err(_) => return incomplete(rt, job, "PAGINATION_CONTRACT_MISMATCH").await,
            };
        if job.progress.records + observation.records > limits.max_records
            || job.progress.bytes + page.result.body.len() as u64 > limits.max_bytes
        {
            return incomplete(rt, job, "RECORD_OR_BYTE_LIMIT").await;
        }
        job.progress.pages += 1;
        job.progress.records += observation.records;
        job.progress.bytes += page.result.body.len() as u64;
        if let Some(cursor) = &job.progress.next_cursor {
            job.progress
                .cursor_digests
                .push(executor::digest(cursor.as_bytes()));
        }
        job.progress.next_page = observation.next_page;
        job.progress.next_cursor = observation.next_cursor;
        if observation.ended {
            job.state = JobState::Succeeded;
            job.reason = None;
            return save(rt, job).await;
        }
        save(rt, job).await?;
    }
}
pub(super) async fn status(rt: &Runtime, id: &str) -> Result<serde_json::Value> {
    let mut job = load(rt, id).await?;
    let path = records::path(rt, "jobs", id)?.with_extension("lock");
    if let Some(_lock) = crate::tasks::journal::lock::try_acquire(&path)? {
        if job.state == JobState::Running {
            pause(rt, &mut job, "PROCESS_INTERRUPTED").await?;
        }
    }
    summary(&job)
}
pub(super) async fn result(rt: &Runtime, id: &str, index: u32) -> Result<serde_json::Value> {
    let job = load(rt, id).await?;
    let index = if index == 0 {
        job.progress.pages.max(1)
    } else {
        index
    };
    if index > job.progress.pages + 1 {
        return Err(Failure::new(
            "JOB_PAGE_NOT_FOUND",
            6,
            "此页没有已取得的本地结果。",
        ));
    }
    let page: Page = records::read(rt, &page_path(rt, id, index)?, PAGE_LIMIT)
        .await?
        .ok_or_else(|| Failure::new("JOB_PAGE_NOT_FOUND", 6, "此页尚未取得完整响应。"))?;
    if page.job_id != job.id || page.index != index || page.contract_digest != job.published.digest
    {
        return Err(crate::private_store::unavailable());
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct ResultPage {
        /// 已验证且计入原任务检查点。
        checkpoint_committed: bool,
        /// 原完整页结果与取得时刻。
        page: Page,
        /// 不把不同时间取得的页称为同一时点快照。
        point_in_time_snapshot: bool,
    }
    super::serialize(ResultPage {
        checkpoint_committed: index <= job.progress.pages,
        page,
        point_in_time_snapshot: false,
    })
}

pub(super) async fn list(rt: &Runtime, page: u32) -> Result<serde_json::Value> {
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Item {
        /// 本地任务标识。
        job_id: String,
        /// 稳定接口标识。
        operation_id: String,
        /// 固定执行版本。
        operation_version: u64,
        /// 原IAM人员标识。
        subject_id: String,
        /// 固定环境。
        environment: String,
        /// 当前持久化任务状态。
        state: JobState,
        /// 已完成页数。
        pages: u32,
        /// 最近检查点时刻。
        updated_at: String,
    }
    #[derive(serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Listing {
        /// 当前页任务摘要。
        items: Vec<Item>,
        /// 本地记录总数。
        total: usize,
        /// 当前页码。
        page: u32,
        /// 每页条数。
        page_size: u32,
    }
    let index = records::index(rt, "jobs", page)?;
    let mut items = Vec::new();
    for id in index.ids {
        let row = load(rt, &id).await?;
        items.push(Item {
            job_id: row.id,
            operation_id: row.published.contract.operation_id,
            operation_version: row.published.contract.operation_version,
            subject_id: row.profile.subject_id,
            environment: row.profile.environment,
            state: row.state,
            pages: row.progress.pages,
            updated_at: row.updated_at,
        });
    }
    super::serialize(Listing {
        items,
        total: index.total,
        page: index.page,
        page_size: 20,
    })
}
