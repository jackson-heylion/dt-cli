use super::*;

pub(super) fn endpoint(env: &profile::Environment, route: &str) -> Result<String> {
    if !route.starts_with('/')
        || route.contains("..")
        || route.contains("//")
        || route.contains(['?', '#', '\\'])
    {
        return Err(invalid());
    }
    Ok(format!("{}{}", env.api_origin.trim_end_matches('/'), route))
}

/// One bounded HTTP exchange. Error bodies that are not JSON keep their status.
pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: Value,
    pub(super) retry_after: Option<u64>,
}
pub(super) async fn send(request: reqwest::RequestBuilder) -> Result<Reply> {
    send_limited(request, BODY_LIMIT).await
}
pub(super) async fn send_limited(request: reqwest::RequestBuilder, limit: usize) -> Result<Reply> {
    let mut response = request.send().await.map_err(|_| http::network())?;
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|seconds| *seconds <= 3600);
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| http::network())? {
        if bytes.len() + chunk.len() > limit {
            return Err(http::protocol());
        }
        bytes.extend_from_slice(&chunk);
    }
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(_) if (200..300).contains(&status) => return Err(http::protocol()),
            Err(_) => Value::Null,
        }
    };
    Ok(Reply {
        status,
        body,
        retry_after,
    })
}
/// A server-provided identifier that is safe to interpret or surface.
pub(super) fn safe_code(value: &Value) -> Option<&str> {
    value.as_str().filter(|v| {
        !v.is_empty()
            && v.len() <= 64
            && v.starts_with(|c: char| c.is_ascii_uppercase())
            && v.bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
    })
}
pub(super) fn trace(body: &Value) -> Option<String> {
    [&body["meta"]["traceId"], &body["traceId"]]
        .into_iter()
        .find_map(Value::as_str)
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 64
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .map(str::to_owned)
}
/// Maps a governed error to a fixed CLI code; downstream text never reaches the envelope.
pub(super) fn remote_failure(reply: &Reply) -> Failure {
    let body = &reply.body;
    let code = safe_code(&body["error"]["code"])
        .or_else(|| safe_code(&body["code"]))
        .unwrap_or("");
    let mut meta = Map::new();
    let mut failure = match reply.status {
        400 => invalid(),
        401 => match code {
            "AUTHORIZATION_REVOKED" => revoked(),
            _ => auth_required(),
        },
        403 => match code {
            "DATA_SCOPE_DENIED" => {
                Failure::new("DATA_SCOPE_DENIED", 4, "目标数据不在本人当前权限范围内。")
            }
            "AUTHORIZATION_REVOKED" => revoked(),
            "AUTHORIZATION_EXPIRED" => Failure::new(
                "AUTHORIZATION_EXPIRED",
                4,
                "个人授权已到期，请重新登录并同意授权。",
            ),
            "GRANT_EXPIRED" => Failure::new(
                "GRANT_EXPIRED",
                4,
                "管理员授予的接口权限已到期，请联系管理员；重新登录不会恢复权限。",
            ),
            "CONSENT_REQUIRED" => Failure::new(
                "CONSENT_REQUIRED",
                4,
                "该接口版本尚未获得本人同意，请重新选择授权范围。",
            ),
            _ => Failure::new("SCOPE_DENIED", 4, "当前受控访问权限不足。"),
        },
        404 => Failure::new("RESOURCE_UNAVAILABLE", 6, "受控资源不存在或不属于本人。"),
        409 => match code {
            "CONTRACT_CHANGED" => contract_changed(),
            "EXECUTOR_UNAVAILABLE" => Failure::new(
                "EXECUTOR_UNAVAILABLE",
                6,
                "服务端尚未提供该接口版本的执行实现，请联系维护者。",
            ),
            "PARTIAL_RESULT" => partial_result(),
            "IDEMPOTENCY_CONFLICT" => {
                Failure::new("IDEMPOTENCY_CONFLICT", 6, "同一幂等键已用于不同请求。")
            }
            "ALREADY_DISPATCHED" => {
                Failure::new("ALREADY_DISPATCHED", 6, "请求已派发，不能取消或重发。")
            }
            "INTENT_EXPIRED" => Failure::new("INTENT_EXPIRED", 6, "意图已过期；未发送。"),
            "INTENT_TERMINAL" | "JOB_TERMINAL" => {
                Failure::new("INTENT_TERMINAL", 6, "该请求已处于终态，不能再变更。")
            }
            "RESULT_REVIEW_REQUIRED" => Failure::new(
                "RESULT_REVIEW_REQUIRED",
                6,
                "此前结果待人工核对；不会重新发送。",
            ),
            "APPROVAL_REQUIRED" => Failure::new(
                "APPROVAL_REQUIRED",
                6,
                "意图尚未由本人在可信确认页批准；未发送。",
            ),
            _ => Failure::new("RESOURCE_UNAVAILABLE", 6, "受控操作当前不可用。"),
        },
        202 if code == "SIDE_EFFECT_UNKNOWN" => side_effect_unknown(),
        422 => {
            if let Some(downstream) = safe_code(&body["error"]["downstreamCode"]) {
                meta.insert("downstreamCode".into(), json!(downstream));
            }
            Failure::new("BUSINESS_REJECTED", 6, "目标系统拒绝了本次业务请求。")
        }
        429 => Failure::new("RATE_LIMITED", 5, "受控访问超出当前预算，请稍后再试。"),
        502 => Failure::new("UPSTREAM_CONTRACT_MISMATCH", 5, "目标系统返回不符合合同。"),
        503 => Failure::new(
            "DEPENDENCY_UNAVAILABLE",
            5,
            "受控访问依赖暂不可用或已停用。",
        ),
        _ => http::network(),
    };
    if matches!(reply.status, 429 | 503)
        && let Some(seconds) = reply.retry_after
    {
        meta.insert("retryAfterSeconds".into(), json!(seconds));
    }
    // A write intent failure carries its current state and the intent to query or reconcile.
    for key in ["state", "sideEffect"] {
        if let Some(value) = body["meta"][key].as_str().filter(|v| v.len() <= 16) {
            meta.insert(key.into(), json!(value));
        }
    }
    if let Some(view) = intent_view(&body["meta"]["recovery"]) {
        failure.recovery = Some(Box::new(view));
    }
    failure.trace_id = trace(body);
    if !meta.is_empty() {
        failure = failure.with_partial(Value::Null, Value::Object(meta));
    }
    failure
}
pub(super) fn side_effect_unknown() -> Failure {
    let mut failure = Failure::new(
        "SIDE_EFFECT_UNKNOWN",
        6,
        "写入可能已发出但结果无法确认；不会自动重发。",
    );
    failure.hint =
        "用 dt-cli intents status 查询；仍为 unknown 时在确认页人工核对，不要重新准备同一写入。";
    failure
}
/// The intent fields a failure may carry back; anything else the server sends is dropped.
pub(super) fn intent_view(value: &Value) -> Option<Value> {
    let id = value["intentId"].as_str().filter(|v| job_id(v))?;
    let mut view = Map::new();
    view.insert("intentId".into(), json!(id));
    for key in [
        "state",
        "sideEffect",
        "reason",
        "downstreamCode",
        "operationId",
        "operationVersion",
        "expiresAt",
        "completedAt",
        "evidenceSource",
        "confirmationUrl",
    ] {
        if let Some(text) = value[key].as_str().filter(|v| v.len() <= 256) {
            view.insert(key.into(), json!(text));
        }
    }
    Some(Value::Object(view))
}
pub(super) fn partial_result() -> Failure {
    Failure::new(
        "PARTIAL_RESULT",
        7,
        "任务尚未取得完整结果；不返回部分数据。",
    )
}
/// The §4 envelope of a successful governed response.
pub(super) fn envelope(reply: Reply) -> Result<Value> {
    if !(200..300).contains(&reply.status) || reply.body["ok"] == false {
        return Err(remote_failure(&reply));
    }
    let body = reply.body;
    if body["schemaVersion"] != "1.0" || body["ok"] != true || !body["error"].is_null() {
        return Err(http::protocol());
    }
    Ok(body)
}
/// An OAuth response: plain JSON on success, OAuth error plus governed code on failure.
pub(super) fn oauth(reply: Reply) -> Result<Value> {
    if reply.status != 200 {
        return Err(remote_failure(&reply));
    }
    if !reply.body.is_object() {
        return Err(http::protocol());
    }
    Ok(reply.body)
}
