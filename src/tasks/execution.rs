use super::{compare, definitions, journal::Journal, planning};
use crate::{
    Runtime,
    governed::tasks::TaskSession,
    output::{Failure, Result},
    profiles::Binding,
};
use serde_json::{Value, json};

fn api_params(id: &str, params: &Value) -> Value {
    let mut params = params.clone();
    if id == "order-config.compare" {
        params.as_object_mut().unwrap().remove("shopCodes");
    }
    params
}
fn session<'a>(rt: &'a Runtime, j: &Journal, params: Option<&Value>) -> Result<TaskSession<'a>> {
    let session = TaskSession::new(
        rt,
        &j.record.binding,
        &j.record.operation_id,
        Some(&j.record.operation_version),
        if j.record.task_id == "like.send" {
            "write"
        } else {
            "read"
        },
        params,
    )?;
    if session.operation["contractDigest"] != j.record.contract_digest {
        return Err(Failure::new(
            "CONTRACT_CHANGED",
            6,
            "原执行合同已变化；未继续执行。",
        ));
    }
    Ok(session)
}
fn result(j: &Journal, mut remote: Value, permission_verified: bool) -> Value {
    let original_meta = remote
        .as_object_mut()
        .and_then(|o| o.remove("_meta"))
        .unwrap_or_else(|| json!({}));
    let complete = if j.record.task_id == "like.send" {
        remote["state"] == "succeeded" && remote["sideEffect"] == "confirmed"
    } else {
        remote["complete"] != false
    };
    let mut source = json!({"operationId":j.record.operation_id,"operationVersion":j.record.operation_version,"contractDigest":j.record.contract_digest,"traceId":original_meta["traceId"]});
    source[if j.record.task_id == "like.send" {
        "intentId"
    } else {
        "jobId"
    }] = json!(j.record.remote_id);
    let mut meta = original_meta;
    meta["taskSchemaVersion"] = json!(1);
    meta["recipeVersion"] = json!(j.record.recipe_version);
    meta["effect"] = json!(if j.record.task_id == "like.send" {
        "write"
    } else {
        "read"
    });
    meta["complete"] = json!(complete);
    meta["onlineVerified"] = json!(true);
    meta["permissionVerified"] = json!(permission_verified);
    meta["availability"] = json!({"state":if permission_verified && complete{"ready"}else{"unknown"},"reason":if permission_verified && complete{"verified-current-permissions"}else{"status-does-not-verify-execution-permission"}});
    meta["observedAt"] = json!(chrono::Utc::now().to_rfc3339());
    meta["sources"] = json!([source]);
    meta["binding"] = json!(j.record.binding);
    let mut actions = vec![];
    if !complete
        || matches!(
            remote["state"].as_str(),
            Some("unknown" | "dispatching" | "queued" | "running")
        )
    {
        actions.push(json!({"id":"inspect-run","actor":"employee","kind":"command","argv":["dt-cli","tasks","status",j.record.run_id],"requiresInteraction":false,"reason":"查询原任务或意图，不重新派发"}));
    }
    if j.record.task_id == "like.send"
        && remote["state"] == "unknown"
        && let Some(id) = &j.record.remote_id
    {
        actions.push(json!({"id":"reconcile-intent","actor":"employee","kind":"browser","url":format!("{}/cli-governed-intent.html?intentId={id}",j.record.binding.issuer),"requiresInteraction":true,"reason":"在可信IAM页面核对原意图结果；不会再次发送"}));
    }
    if remote["resultRecovery"]["requiresOriginalParameters"] == true {
        actions.push(json!({"id":"recover-selection","actor":"employee","kind":"manual","requiresInteraction":true,"reason":"提供原参数文件，用 tasks run order-config.compare --run-id 恢复原job结果及原门店选择"}));
    }
    meta["actions"] = json!(actions);
    if let Some(state) = remote.get("state") {
        meta["state"] = state.clone();
    }
    if let Some(effect) = remote.get("sideEffect") {
        meta["sideEffect"] = effect.clone();
    }
    json!({"taskId":j.record.task_id,"runId":j.record.run_id,"result":remote,"_meta":meta})
}
fn failure(j: &Journal, failure: Failure) -> Failure {
    let data = failure
        .partial_data
        .as_deref()
        .cloned()
        .unwrap_or_else(|| json!({"complete":false}));
    let mut meta = failure
        .partial_meta
        .as_deref()
        .cloned()
        .unwrap_or_else(|| json!({}));
    meta["complete"] = json!(false);
    meta["binding"] = json!(j.record.binding);
    meta["taskSchemaVersion"] = json!(1);
    meta["recipeVersion"] = json!(j.record.recipe_version);
    meta["observedAt"] = json!(chrono::Utc::now().to_rfc3339());
    j.recovery(failure).with_partial(
        json!({"taskId":j.record.task_id,"runId":j.record.run_id,"result":data}),
        meta,
    )
}
/// Preserve a verified server observation when only the final local save fails.
fn persist_observation(j: &mut Journal, server: &Value, presentation: &Value) -> Result<()> {
    j.observe(server).map_err(|mut error| {
        if error.code != "LOCAL_STATE_UNAVAILABLE" {
            return error;
        }
        let mut payload = presentation.clone();
        payload.as_object_mut().map(|o| o.remove("_meta"));
        let mut meta = server["_meta"]
            .as_object()
            .cloned()
            .map(Value::Object)
            .unwrap_or_else(|| json!({}));
        meta["recordPersistence"] = json!("unconfirmed");
        for key in ["state", "sideEffect"] {
            if let Some(v) = server.get(key) {
                meta[key] = v.clone();
            }
        }
        error.message = "已取得服务端结果，但本地保存失败；请查询原ID，不要重发。";
        error.with_partial(payload, meta)
    })
}
pub(super) async fn run(
    rt: &Runtime,
    id: &str,
    binding: Binding,
    version: Option<&str>,
    params: &Value,
    run_id: Option<&str>,
) -> Result<Value> {
    let api = api_params(id, params);
    if let Some(run_id) = run_id {
        let mut journal = Journal::load(rt, run_id).await?;
        if journal.record.task_id != id
            || journal.record.binding != binding
            || version.is_some_and(|v| v != journal.record.operation_version)
        {
            return Err(failure(&journal, crate::output::invalid()));
        }
        journal
            .params_match(params)
            .map_err(|e| failure(&journal, e))?;
        let s = session(rt, &journal, Some(&api)).map_err(|e| failure(&journal, e))?;
        let value = resume(&s, &mut journal, &api, Some(params)).await;
        return value
            .map(|v| result(&journal, v, id == "order-config.compare"))
            .map_err(|e| failure(&journal, e));
    }
    let task = definitions::get(id)?;
    let s = TaskSession::new(
        rt,
        &binding,
        &task.operation_id,
        version,
        &task.effect,
        Some(&api),
    )?;
    planning::compatible(task, &s.operation)?;
    let mut journal = Journal::create(rt, id, binding, &s.operation, params, &api).await?;
    if rt.interactive {
        eprintln!("执行记录：{}（恢复时查询原记录）", journal.record.run_id);
    }
    let value = if id == "like.send" {
        send(&s, &mut journal, &api).await
    } else {
        submit(&s, &mut journal, &api, params).await
    };
    value
        .map(|v| result(&journal, v, true))
        .map_err(|e| failure(&journal, e))
}
async fn submit(
    s: &TaskSession<'_>,
    j: &mut Journal,
    api: &Value,
    params: &Value,
) -> Result<Value> {
    let remote = s.submit(api).await?;
    j.attach(&remote)?;
    finish_compare(s, j, params).await
}
async fn finish_compare(s: &TaskSession<'_>, j: &mut Journal, params: &Value) -> Result<Value> {
    let remote = s
        .wait(
            j.record
                .remote_id
                .as_deref()
                .ok_or_else(crate::http::protocol)?,
            60,
        )
        .await?;
    j.check_remote(&remote)?;
    s.validate_result(&remote)?;
    let mut compared = compare::compare(params, &remote)?;
    compared["_meta"] = remote["_meta"].clone();
    persist_observation(j, &remote, &compared)?;
    Ok(compared)
}
async fn send(s: &TaskSession<'_>, j: &mut Journal, params: &Value) -> Result<Value> {
    let remote = s
        .prepare(
            params,
            j.record
                .idempotency_key
                .as_deref()
                .ok_or_else(crate::http::protocol)?,
        )
        .await?;
    j.attach(&remote)?;
    if !matches!(remote["state"].as_str(), Some("prepared" | "approved"))
        || remote["sideEffect"] != "none"
    {
        return Err(crate::http::protocol());
    }
    crate::governed::require_current_intent(&remote)?;
    let id = j
        .record
        .remote_id
        .clone()
        .ok_or_else(crate::http::protocol)?;
    if remote["state"] == "prepared" {
        if s.operation["confirmation"]["channel"] == "backend-grant" {
            return Err(crate::http::protocol());
        }
        j.step("authorize-attempted")?;
        let authorized = s
            .authorize(&id, &j.record.arguments_digest, &j.record.contract_digest)
            .await?;
        j.check_remote(&authorized)?;
    }
    j.step("dispatch-attempted")?;
    let mut invoked = s.invoke(&id, &j.record.arguments_digest).await?;
    // Invoke already validated the exact contract and succeeded/confirmed. Keep those proven
    // digests so observation can mark a terminal receipt without an unnecessary extra request.
    for (key, value) in [
        ("operationVersion", &j.record.operation_version),
        ("contractDigest", &j.record.contract_digest),
        ("argumentsDigest", &j.record.arguments_digest),
    ] {
        invoked[key] = json!(value);
    }
    persist_observation(j, &invoked, &invoked)?;
    Ok(invoked)
}
async fn resume(
    s: &TaskSession<'_>,
    j: &mut Journal,
    api: &Value,
    params: Option<&Value>,
) -> Result<Value> {
    if j.record.remote_id.is_none() {
        if j.record.task_id == "like.send" && j.record.stage == "prepare-attempted" {
            // Same key and same fixed arguments can locate the original preparation. This path
            // never authorizes or invokes, even if IAM creates a previously lost preparation.
            let remote = s
                .prepare(
                    api,
                    j.record
                        .idempotency_key
                        .as_deref()
                        .ok_or_else(crate::http::protocol)?,
                )
                .await?;
            j.attach(&remote)?;
        } else {
            return Err(Failure::new(
                "RUN_UNLOCATED",
                6,
                "原请求尚未定位；不会重新提交或派发。",
            ));
        }
    }
    if j.record.task_id == "like.send" {
        let remote = s.intent(j.record.remote_id.as_deref().unwrap()).await?;
        persist_observation(j, &remote, &remote)?;
        return Ok(remote);
    }
    if let Some(params) = params {
        return finish_compare(s, j, params).await;
    }
    let remote = s.job(j.record.remote_id.as_deref().unwrap()).await?;
    persist_observation(j, &remote, &remote)?;
    Ok(remote)
}
pub(super) async fn status(rt: &Runtime, id: &str) -> Result<Value> {
    let mut journal = Journal::load(rt, id).await?;
    if journal.record.remote_id.is_none() {
        return Err(failure(
            &journal,
            Failure::new(
                "RUN_UNLOCATED",
                6,
                "原请求尚未定位；请保留本记录，不要创建相同写入。",
            ),
        ));
    }
    let s = session(rt, &journal, None).map_err(|e| failure(&journal, e))?;
    let remote = if journal.record.task_id == "like.send" {
        s.intent(journal.record.remote_id.as_deref().unwrap()).await
    } else {
        s.job(journal.record.remote_id.as_deref().unwrap()).await
    };
    let value = match remote {
        Ok(mut remote) => {
            persist_observation(&mut journal, &remote, &remote)
                .map_err(|e| failure(&journal, e))?;
            if journal.record.task_id == "order-config.compare" && remote["state"] == "succeeded" {
                remote["resultRecovery"] = json!({"requiresOriginalParameters":true,"reason":"selection-not-cached","command":"tasks run order-config.compare","runId":id});
            }
            result(&journal, remote, false)
        }
        Err(e) => return Err(failure(&journal, e)),
    };
    Ok(value)
}
