use super::*;

pub struct Launch {
    pub message_id: String,
    pub application_id: String,
    pub launch_url: zeroize::Zeroizing<String>,
    pub expires_at: String,
    pub trace_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct LaunchPayload {
    pub(super) message_id: String,
    pub(super) application_id: String,
    pub(super) launch_url: String,
    pub(super) expires_at: String,
}
/// One open request. Delivery signing is never retried, so this sends exactly one request.
pub async fn launch(
    client: &reqwest::Client,
    env: &Environment,
    c: &Credentials,
    route: &str,
) -> Result<Launch> {
    let r = client
        .post(format!("{}{}", env.api_origin.trim_end_matches('/'), route))
        .bearer_auth(&c.access_token)
        .json(&json!({}))
        .send()
        .await
        .map_err(|_| network())?;
    let status = r.status().as_u16();
    let body = read_body(r, PAGE_BYTE_LIMIT).await;
    let value = body.value.ok_or_else(|| {
        body.failure
            .unwrap_or_else(|| Failure::new("DEPENDENCY_UNAVAILABLE", 5, "IAM 响应不符合协议。"))
    })?;
    let trace_id = safe_trace(&value);
    if status != 200 || value["ok"] != true {
        let mut failure = failure(status, &value);
        failure.trace_id = trace_id;
        return Err(failure);
    }
    if value["schemaVersion"] != "1.0"
        || value["operationId"] != "iam.workflow.open"
        || !value["error"].is_null()
    {
        return Err(with_trace(protocol(), &trace_id));
    }
    let payload: LaunchPayload = serde_json::from_value(value["data"].clone())
        .map_err(|_| with_trace(protocol(), &trace_id))?;
    Ok(Launch {
        message_id: payload.message_id,
        application_id: payload.application_id,
        launch_url: zeroize::Zeroizing::new(payload.launch_url),
        expires_at: payload.expires_at,
        trace_id,
    })
}
