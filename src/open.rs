//! `workflow open`: hand one explicitly chosen message to the system browser.
//!
//! The dedicated command and `api call iam.workflow.open` both end here, so the generic
//! channel cannot surface the internal launch URL that the sensitive-result adapter removes.
use crate::{
    Runtime, http,
    output::{Failure, Result, invalid},
    profile,
};
use serde_json::{Value, json};

fn jump_unavailable() -> Failure {
    Failure::new("JUMP_UNAVAILABLE", 6, "IAM 未提供可信的打开入口。")
}
fn with_trace(mut failure: Failure, trace_id: &Option<String>) -> Failure {
    failure.trace_id = trace_id.clone();
    failure
}
/// Only a canonical positive message identifier is accepted, never a URL or a path.
pub fn canonical_id(raw: &str) -> Result<String> {
    if raw.is_empty()
        || raw.len() > 19
        || raw.starts_with('0')
        || !raw.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    Ok(raw.to_owned())
}
/// The internal launch URL must be the fixed IAM entry with one delivery ticket.
fn authorized_url(raw: &str, portal_origin: &str, launch_path: &str) -> Result<()> {
    let url = url::Url::parse(raw).map_err(|_| jump_unavailable())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
        || url.origin().ascii_serialization() != portal_origin
        || url.path() != launch_path
    {
        return Err(jump_unavailable());
    }
    let mut pairs = url.query_pairs();
    let ticket = pairs.next().ok_or_else(jump_unavailable)?;
    if ticket.0 != "ticket" || ticket.1.is_empty() || pairs.next().is_some() {
        return Err(jump_unavailable());
    }
    Ok(())
}
/// Requests one delivery credential and passes only the validated entry to the browser.
pub async fn open(rt: &Runtime, name: &str, raw_id: &str) -> Result<Value> {
    let id = canonical_id(raw_id)?;
    let catalog = crate::catalog::Catalog::shared();
    let operation = catalog.schema("iam.workflow.open")?;
    let route = operation["route"].as_str().ok_or_else(invalid)?;
    let route = route.replace("{messageId}", &id);
    let p = profile::read(&rt.root, name)?
        .ok_or_else(|| Failure::new("PROFILE_NOT_CONFIGURED", 2, "profile 尚未配置。"))?;
    let env = rt.environment(&p.environment)?;
    let client = http::client()?;
    let credentials = rt.credentials(&p, env, &client).await?;
    let launch = http::launch(&client, env, &credentials, &route).await?;
    let trace_id = launch.trace_id.clone();
    if launch.message_id != id
        || launch.application_id.is_empty()
        || !launch.application_id.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(with_trace(http::protocol(), &trace_id));
    }
    let expires = chrono::DateTime::parse_from_rfc3339(&launch.expires_at)
        .map_err(|_| with_trace(http::protocol(), &trace_id))?;
    if expires <= chrono::Utc::now() {
        return Err(with_trace(
            Failure::new(
                "RESOURCE_UNAVAILABLE",
                6,
                "本次打开凭据已过期；请重新执行。",
            ),
            &trace_id,
        ));
    }
    authorized_url(&launch.launch_url, &env.portal_origin, &env.launch_path)
        .map_err(|failure| with_trace(failure, &trace_id))?;
    rt.browser
        .open(&launch.launch_url)
        .map_err(|failure| with_trace(failure, &trace_id))?;
    // The URL is dropped here and never enters data, error, meta, debug output or logs.
    Ok(json!({"messageId":id,"applicationId":launch.application_id,
        "launchStatus":"browser_requested","browserConsumption":"unknown",
        "_traceId":trace_id}))
}
