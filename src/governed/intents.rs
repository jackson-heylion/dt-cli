use super::*;

pub(super) fn idempotency_key(value: Option<&str>) -> Result<String> {
    match value {
        Some(key)
            if (16..=128).contains(&key.len())
                && key
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') =>
        {
            Ok(key.to_owned())
        }
        Some(_) => Err(invalid()),
        None => Ok(crate::login::random()),
    }
}
/// The one confirmation page this environment's IAM serves for the intent; nothing else is shown.
pub(super) fn confirmation_url(env: &profile::Environment, id: &str) -> String {
    format!("{}{INTENT_PAGE}{id}", env.api_origin)
}
/// An intent view from IAM, checked field by field; the page URL must be IAM's own page.
pub(super) fn intent_data(
    body: &Value,
    env: &profile::Environment,
    id: Option<&str>,
    operation: Option<&str>,
) -> Result<Value> {
    let data = &body["data"];
    let intent = data["intentId"]
        .as_str()
        .filter(|v| job_id(v) && id.is_none_or(|id| id == *v))
        .ok_or_else(http::protocol)?;
    let states = [
        "prepared",
        "approved",
        "dispatching",
        "succeeded",
        "unknown",
        "rejected",
        "expired",
        "cancelled",
    ];
    if !data["state"].as_str().is_some_and(|v| states.contains(&v))
        || !matches!(
            data["sideEffect"].as_str(),
            Some("none" | "confirmed" | "unknown")
        )
        || operation.is_some_and(|op| data["operationId"] != op)
    {
        return Err(http::protocol());
    }
    let url = confirmation_url(env, intent);
    if !(data["confirmationUrl"].is_null() || data["confirmationUrl"] == url.as_str()) {
        return Err(http::protocol());
    }
    let mut view = intent_view(data).ok_or_else(http::protocol)?;
    for key in [
        "argumentsDigest",
        "contractDigest",
        "approvedAt",
        "dispatchedAt",
        "reviewedAt",
        "reviewOutcome",
    ] {
        if let Some(text) = data[key].as_str().filter(|v| v.len() <= 128) {
            view[key] = json!(text);
        }
    }
    // HRMP returns no per-request record id; none is ever invented.
    view["downstreamRecordId"] = Value::Null;
    view["_meta"] = json!({"traceId": trace(body)});
    Ok(view)
}

/// Creates a write intent (nothing is sent); the agent authorizes the exact digests via CLI.
pub(super) async fn prepare_intent(
    rt: &Runtime,
    name: &str,
    id: &str,
    version: Option<&str>,
    raw: &str,
    key: Option<&str>,
    preview: bool,
) -> Result<Value> {
    let p = require_profile(rt, name)?;
    let env = rt.environment(&p.environment)?;
    let cache = read_cache(&rt.root, name, &p)?.ok_or_else(not_synced)?;
    let operation = select(&cache, id, version)?;
    usable(operation, "write")?;
    if !matches!(
        operation["confirmation"]["channel"].as_str(),
        Some("agent-cli" | "backend-grant")
    ) {
        return Err(contract_changed());
    }
    // Arguments and the key are validated before any credential read or network request.
    let params = validate_params(raw, &cache.validators(operation)?.input)?;
    let key = idempotency_key(key)?;
    if preview {
        return Ok(json!({
            "preview": true,
            "provider": PROVIDER,
            "operationId": id,
            "operationVersion": operation["operationVersion"],
            "contractDigest": operation["contractDigest"],
            "effect": "write",
            "mode": "intent",
            "environment": p.environment,
            "systemId": p.system_id,
            "permissionVerified": false,
            "credentialReads": 0,
            "httpRequests": 0,
            "browserOpened": false,
        }));
    }
    let client = http::client()?;
    prepare_with(rt, &p, env, &client, operation, &params, &key).await
}

pub(super) async fn prepare_with(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    operation: &Value,
    params: &Value,
    key: &str,
) -> Result<Value> {
    let id = operation["operationId"]
        .as_str()
        .ok_or_else(http::protocol)?;
    let current = operation;
    let system_token = exchange(rt, p, env, client, current).await?;
    let body = envelope(
        send(
            client
                .post(endpoint(env, INTENTS_ROUTE)?)
                .bearer_auth(system_token.as_str())
                .header("Idempotency-Key", key)
                .json(&json!({
                    "contract": {
                        "operationId": id,
                        "operationVersion": current["operationVersion"],
                        "contractDigest": current["contractDigest"],
                    },
                    "arguments": params,
                })),
        )
        .await?,
    )?;
    let mut view = intent_data(&body, env, None, Some(id))?;
    view["idempotencyKey"] = json!(key);
    if current["confirmation"]["channel"] == "backend-grant" && view["state"] == "prepared" {
        return Err(http::protocol());
    }
    view["next"] = if view["state"] == "approved" {
        json!("后台权限已核验；执行 dt-cli intents invoke 完成本次派发。approved 不代表已发送。")
    } else {
        json!(
            "旧合同需执行 dt-cli intents authorize，再执行 intents invoke；新任务请选择 backend-grant 合同。"
        )
    };
    Ok(view)
}

/// Wait only for recorded authorization; this never opens an approval page, then dispatch once.
pub(super) async fn wait_and_invoke(rt: &Runtime, name: &str, mut view: Value) -> Result<Value> {
    let id = view["intentId"]
        .as_str()
        .ok_or_else(http::protocol)?
        .to_owned();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(300);
    loop {
        match view["state"].as_str() {
            Some("approved") => return invoke_intent(rt, name, &id).await,
            Some("prepared") => {}
            _ => return Ok(view),
        }
        if tokio::time::Instant::now() >= deadline {
            view["next"] = json!("等待已结束；请查询本次 intentId 状态，勿重新创建相同写入。");
            return Ok(view);
        }
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(3)) => {},
            _ = tokio::signal::ctrl_c() => {
                view["next"] = json!("已停止等待；未主动派发，请查询或取消本次 intentId。");
                return Ok(view);
            }
        }
        view = intent_action(rt, name, "intents.status", &id)
            .await
            .map_err(|mut failure| {
                failure.recovery = Some(Box::new(view.clone()));
                failure
            })?;
    }
}

/// Authorize the exact prepared payload on the user's instruction. No business write is sent.
pub(super) async fn authorize_intent(
    rt: &Runtime,
    name: &str,
    id: &str,
    arguments: &str,
    contract: &str,
) -> Result<Value> {
    if !job_id(id) || !digest_like(&json!(arguments)) || !digest_like(&json!(contract)) {
        return Err(invalid());
    }
    let p = require_profile(rt, name)?;
    let env = rt.environment(&p.environment)?;
    let client = http::client()?;
    authorize_with(rt, &p, env, &client, id, arguments, contract).await
}

pub(super) async fn authorize_with(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    id: &str,
    arguments: &str,
    contract: &str,
) -> Result<Value> {
    let route = format!("{INTENTS_ROUTE}/{id}/authorize");
    let reply = authorized(rt, p, env, client, |token| {
        Ok(client
            .post(endpoint(env, &route)?)
            .bearer_auth(token)
            .json(&json!({
                "argumentsDigest": arguments, "contractDigest": contract,
            })))
    })
    .await?;
    let view = intent_data(&envelope(reply)?, env, Some(id), None)?;
    if view["state"] != "approved"
        || view["argumentsDigest"] != arguments
        || view["contractDigest"] != contract
    {
        return Err(http::protocol());
    }
    Ok(view)
}

/// The owner's status or cancellation of an intent; neither ever sends the write.
pub(super) async fn intent_action(
    rt: &Runtime,
    name: &str,
    operation: &str,
    id: &str,
) -> Result<Value> {
    if !job_id(id) {
        return Err(invalid());
    }
    let p = require_profile(rt, name)?;
    let env = rt.environment(&p.environment)?;
    let client = http::client()?;
    intent_action_with(rt, &p, env, &client, operation, id).await
}

pub(super) async fn intent_action_with(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    operation: &str,
    id: &str,
) -> Result<Value> {
    let cancel = operation == "intents.cancel";
    let route = format!(
        "{INTENTS_ROUTE}/{id}{}",
        if cancel { "/cancel" } else { "" }
    );
    let reply = authorized(rt, p, env, client, |token| {
        let url = endpoint(env, &route)?;
        Ok(if cancel {
            client.post(url).bearer_auth(token).json(&json!({}))
        } else {
            client.get(url).bearer_auth(token)
        })
    })
    .await?;
    intent_data(&envelope(reply)?, env, Some(id), None)
}

/// Sends an approved intent at most once. IAM owns the single dispatch; a lost answer or an
/// interrupt here is reported as an unknown outcome, never as "not executed", and never resent.
pub(super) async fn invoke_intent(rt: &Runtime, name: &str, id: &str) -> Result<Value> {
    if !job_id(id) {
        return Err(invalid());
    }
    let p = require_profile(rt, name)?;
    let env = rt.environment(&p.environment)?;
    let cache = read_cache(&rt.root, name, &p)?.ok_or_else(not_synced)?;
    let client = http::client()?;
    invoke_with(rt, &p, env, &client, &cache, id, None).await
}

pub(super) async fn invoke_with(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    cache: &Cache,
    id: &str,
    expected_arguments: Option<&str>,
) -> Result<Value> {
    let intent = intent_action_with(rt, p, env, client, "intents.status", id).await?;
    if let Some(arguments) = expected_arguments {
        if intent["argumentsDigest"] != arguments {
            return Err(contract_changed());
        }
        match intent["state"].as_str() {
            Some("approved") if intent["sideEffect"] == "none" => {}
            Some("expired") => {
                return Err(Failure::new("INTENT_EXPIRED", 6, "原意图已到期；未派发。"));
            }
            Some("prepared") => {
                return Err(Failure::new(
                    "APPROVAL_REQUIRED",
                    6,
                    "原意图尚未授权；未派发。",
                ));
            }
            Some("dispatching" | "unknown") => return Err(side_effect_unknown()),
            _ => {
                return Err(Failure::new(
                    "ALREADY_DISPATCHED",
                    6,
                    "原意图不能再次派发；请查询原状态。",
                ));
            }
        }
        require_current_intent(&intent)?;
    }
    let operation = intent["operationId"].as_str().ok_or_else(http::protocol)?;
    let cached = select(cache, operation, intent["operationVersion"].as_str())?;
    if cached["contractDigest"] != intent["contractDigest"] {
        return Err(contract_changed());
    }
    usable(cached, "write")?;
    let current = cached;
    let system_token = exchange(rt, p, env, client, current).await?;
    let request = client
        .post(endpoint(
            env,
            &format!("/cli-api/v1/systems/{}/invoke", p.system_id),
        )?)
        .bearer_auth(system_token.as_str())
        .json(&json!({ "intentId": id }));
    let unknown = || {
        let mut failure = side_effect_unknown();
        failure.recovery = Some(Box::new(json!({
            "intentId": id,
            "state": "unknown",
            "sideEffect": "unknown",
            "confirmationUrl": confirmation_url(env, id),
        })));
        failure
    };
    let reply = tokio::select! {
        reply = send(request) => reply.map_err(|_| unknown())?,
        _ = tokio::signal::ctrl_c() => return Err(unknown()),
    };
    // A gateway/server error or malformed envelope cannot establish whether dispatch happened.
    if reply.status >= 500
        || reply.body["schemaVersion"] != "1.0"
        || !reply.body["ok"].is_boolean()
        || (reply.body["ok"] == true && !reply.body["error"].is_null())
    {
        return Err(unknown());
    }
    let body = envelope(reply).map_err(|mut failure| {
        if matches!(failure.code, "NETWORK_ERROR" | "DEPENDENCY_UNAVAILABLE") {
            return unknown();
        }
        // A write is never retryable: repeat only by querying this intent.
        failure.retryable = false;
        let recovery = failure.recovery.get_or_insert_with(|| Box::new(json!({})));
        recovery["intentId"] = json!(id);
        recovery["confirmationUrl"] = json!(confirmation_url(env, id));
        failure
    })?;
    let meta = &body["meta"];
    if body["operationId"] != operation
        || meta["contractVersion"] != current["operationVersion"]
        || meta["contractDigest"] != current["contractDigest"]
    {
        return Err(unknown());
    }
    let data = &body["data"];
    if data["intentId"] != id || data["state"] != "succeeded" || data["sideEffect"] != "confirmed" {
        return Err(unknown());
    }
    Ok(json!({
        "intentId": id,
        "state": "succeeded",
        "sideEffect": "confirmed",
        "downstreamRecordId": Value::Null,
        "operationId": operation,
        "_meta": project_meta(meta),
    }))
}

/// Stop a stale intent before the next action; a timestamp cannot authorize execution.
pub(crate) fn require_current_intent(view: &Value) -> Result<()> {
    let expires = view["expiresAt"]
        .as_str()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .ok_or_else(http::protocol)?;
    if expires <= chrono::Utc::now() {
        return Err(Failure::new(
            "INTENT_EXPIRED",
            6,
            "原意图已到期；未继续授权或派发。",
        ));
    }
    Ok(())
}
