//! 同一配置目录共享发送间隔。等待期间释放文件锁。
use super::{catalog, contract::Published, executor::digest, metadata, model::PlatformProfile};
use crate::{
    Runtime,
    output::{Details, Failure, Result},
    private_store, record_lock,
};
use std::{
    fs::File,
    path::PathBuf,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(super) struct Slot {
    /// 本机发送锁，记录发送时间后才释放。
    _lock: File,
    /// 同一接口最近发送时间的私有文件。
    stamp: PathBuf,
    /// 本次发送间隔设置的有效截止时间。
    pub(super) expires: Instant,
}

impl Slot {
    pub(super) fn sent(&self) -> Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| private_store::unavailable())?
            .as_millis() as u64;
        private_store::write(&self.stamp, &now)
    }
}

pub(super) fn remaining(deadline: Instant) -> Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(budget_exhausted());
    }
    Ok(remaining)
}

pub(super) async fn reserve(
    rt: &Runtime,
    profile: &PlatformProfile,
    published: &Published,
    deadline: Instant,
) -> Result<Slot> {
    let contract = &published.contract;
    let origin = url::Url::parse(&profile.origin).map_err(|_| crate::output::invalid())?;
    let key = digest(
        format!(
            "{}\n{}\n{}",
            origin.origin().ascii_serialization(),
            contract.environment,
            contract.operation_id
        )
        .as_bytes(),
    );
    let dir = rt.root.join("send-intervals");
    let lock_path = dir.join(format!("{key}.lock"));
    let stamp = dir.join(format!("{key}.json"));
    let operation = &contract.operation_id;
    let version = contract.operation_version;
    loop {
        remaining(deadline)?;
        let mut settings = metadata::interval(profile, operation, version);
        if settings.is_none() {
            let current = tokio::time::timeout_at(
                deadline.into(),
                catalog::refresh_contract(rt, profile, operation, version),
            )
            .await
            .map_err(|_| unavailable())?
            .map_err(|failure| {
                if matches!(failure.exit, 3 | 4 | 8) {
                    failure
                } else {
                    unavailable()
                }
            })?;
            if current.digest != published.digest {
                return Err(Failure::new(
                    "EXECUTION_CONTRACT_MISMATCH",
                    4,
                    "原执行契约已变化，未发送。",
                ));
            }
            settings = metadata::interval(profile, operation, version);
        }
        let (millis, expires) = settings.ok_or_else(unavailable)?;
        let Some(lock) = record_lock::try_acquire(&lock_path)? else {
            wait(Duration::from_millis(10), deadline).await?;
            continue;
        };
        if Instant::now() >= expires {
            continue;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| private_store::unavailable())?
            .as_millis() as u64;
        let last = private_store::read::<u64>(&stamp)?.unwrap_or(0);
        let pending = last.saturating_add(millis).saturating_sub(now).min(millis);
        if pending == 0 && Instant::now() < deadline {
            return Ok(Slot {
                _lock: lock,
                stamp,
                expires,
            });
        }
        drop(lock);
        wait(
            Duration::from_millis(pending).min(expires.saturating_duration_since(Instant::now())),
            deadline,
        )
        .await?;
    }
}

async fn wait(delay: Duration, deadline: Instant) -> Result<()> {
    if delay >= deadline.saturating_duration_since(Instant::now()) {
        let mut failure = budget_exhausted();
        failure.details = Some(Box::new(Details {
            retry_after_seconds: Some((delay.as_millis() as u64).div_ceil(1000)),
            ..Default::default()
        }));
        return Err(failure);
    }
    tokio::time::sleep(delay).await;
    Ok(())
}

fn budget_exhausted() -> Failure {
    let mut failure = Failure::new("CLI_INTERVAL_WAIT", 5, "本次时间预算不足，未发送请求。");
    failure.retryable = true;
    failure
}

fn unavailable() -> Failure {
    let mut failure = Failure::new(
        "CLI_INTERVAL_UNAVAILABLE",
        5,
        "未能读取当前发送间隔，已暂停发送。请稍后继续原任务。",
    );
    failure.retryable = true;
    failure
}
