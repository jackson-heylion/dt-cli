use super::*;
use std::future::Future;

// One bounded budget across read retries. Unknown outcomes/network errors are not replayed.
pub(super) async fn run<T, F, Fut>(request: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut retries = 0;
    loop {
        let result = tokio::time::timeout_at(deadline, request())
            .await
            .unwrap_or_else(|_| {
                Err(Failure::new(
                    "TIMEOUT",
                    5,
                    "读取及限流等待的总预算已用完，请稍后继续原读取。",
                ))
            });
        match result {
            Err(failure) if failure.code == "RATE_LIMITED" && retries < 2 => {
                let Some(seconds) = failure
                    .partial_meta
                    .as_deref()
                    .and_then(|m| m["retryAfterSeconds"].as_u64())
                else {
                    return Err(failure);
                };
                let delay = Duration::from_secs(seconds.max(1));
                if tokio::time::Instant::now() + delay >= deadline {
                    return Err(failure);
                }
                tokio::select! {
                    _ = tokio::time::sleep(delay) => {},
                    _ = tokio::signal::ctrl_c() => return Err(failure),
                }
                retries += 1;
            }
            value => return value,
        }
    }
}
