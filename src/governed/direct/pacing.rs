//! Share a conservative business request budget across tasks and CLI processes.
use super::*;

pub(super) fn default_interval() -> u64 {
    crate::constants::query::DIRECT_REQUEST_INTERVAL_MILLIS
}

pub(super) async fn send(
    rt: &Runtime,
    plan: &Plan,
    request: reqwest::RequestBuilder,
) -> Result<wire::Reply> {
    // The gateway origin is deployment owned. Sharing its lock also covers
    // ownership preflights and operations with different names on that gateway.
    let origin = url::Url::parse(&plan.base_uri)
        .map_err(|_| catalog_invalid())?
        .origin()
        .ascii_serialization();
    let digest = format!("{:x}", Sha256::digest(origin.as_bytes()));
    let path = rt
        .root
        .join("governed/request-locks")
        .join(format!("{digest}.lock"));
    let _permit = crate::tasks::journal::lock::acquire(&path).await?;
    // Waiting before each request avoids persisting wall-clock timestamps and
    // preserves the limit when a process exits or the machine clock changes.
    tokio::time::sleep(Duration::from_millis(
        plan.limits.min_request_interval_millis,
    ))
    .await;
    send_limited(request, plan.limits.page_bytes).await
}
