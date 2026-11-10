//! 有界读取循环，完整页先提交，失败保留原错误和检查点。
use super::*;
use std::time::Instant;
pub(super) async fn run(rt: &Runtime, job: &mut Job, mut retry_failed: bool) -> Result<()> {
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
            Err(failure) => {
                return paused_failure(rt, job, "AUTHORITY_OR_VERSION_UNAVAILABLE", failure).await;
            }
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
                        incomplete(rt, job, "RECORD_OR_BYTE_LIMIT").await?;
                        return Err(failure.with_partial(
                            PartialData::Job(summary_model(job)),
                            PartialMeta {
                                complete: Some(false),
                                ..Default::default()
                            },
                        ));
                    }
                    let reason = if failure.code == "INTERFACE_PERMISSION_REQUIRED" {
                        "INTERFACE_PERMISSION_REQUIRED"
                    } else {
                        "READ_REQUEST_UNCONFIRMED"
                    };
                    return paused_failure(rt, job, reason, failure).await;
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
async fn paused_failure(rt: &Runtime, job: &mut Job, reason: &str, failure: Failure) -> Result<()> {
    pause(rt, job, reason).await?;
    Err(failure.with_partial(
        PartialData::Job(summary_model(job)),
        PartialMeta {
            complete: Some(false),
            ..Default::default()
        },
    ))
}
