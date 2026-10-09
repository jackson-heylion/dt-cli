use super::*;

pub(super) async fn call_operation(
    rt: &Runtime,
    name: &str,
    id: &str,
    version: Option<&str>,
    raw: &str,
    job: bool,
    preview: bool,
) -> Result<Value> {
    let p = require_profile(rt, name)?;
    let env = rt.environment(&p.environment)?;
    let cache = read_cache(&rt.root, name, &p)?.ok_or_else(not_synced)?;
    let operation = select(&cache, id, version)?;
    usable(operation, "read")?;
    // Arguments are validated before any credential read or network request.
    let mut params = crate::input::strict_json(raw)?;
    delivery_context::apply(rt, name, &p, operation, &mut params)?;
    let params = validate_params(&params.to_string(), &cache.validators(operation)?.input)?;
    if preview {
        return Ok(json!({
            "preview": true,
            "provider": PROVIDER,
            "operationId": id,
            "operationVersion": operation["operationVersion"],
            "contractDigest": operation["contractDigest"],
            "effect": operation["effect"],
            "mode": if job { "job" } else { "invoke" },
            "environment": p.environment,
            "systemId": p.system_id,
            "permissionVerified": false,
            "authentication": "unresolved",
            "credentialReads": 0,
            "httpRequests": 0,
            "browserOpened": false,
        }));
    }
    let client = http::client()?;
    call_with(
        rt,
        &p,
        env,
        &client,
        &cache,
        ReadRequest {
            operation,
            params: &params,
            job,
        },
    )
    .await
}

pub(super) struct ReadRequest<'a> {
    pub operation: &'a Value,
    pub params: &'a Value,
    pub job: bool,
}

pub(super) async fn call_with(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    cache: &Cache,
    request: ReadRequest<'_>,
) -> Result<Value> {
    let ReadRequest {
        operation,
        params,
        job,
    } = request;
    let id = operation["operationId"]
        .as_str()
        .ok_or_else(http::protocol)?;
    let current = operation;
    let body = json!({
        "contract": {
            "operationId": id,
            "operationVersion": current["operationVersion"],
            "contractDigest": current["contractDigest"],
        },
        "arguments": params,
    });
    let route = if job {
        JOBS_ROUTE.to_owned()
    } else {
        format!("/cli-api/v1/systems/{}/invoke", p.system_id)
    };
    let request = || async {
        let system_token = exchange(rt, p, env, client, current).await?;
        envelope(
            send(
                client
                    .post(endpoint(env, &route)?)
                    .bearer_auth(system_token.as_str())
                    .json(&body),
            )
            .await?,
        )
    };
    // Submit is a single dispatch. Only direct reads can repeat on an explicit server 429.
    let result = if job {
        request().await?
    } else {
        read_retry::run(request).await?
    };
    let data = &result["data"];
    if job {
        let job = data["jobId"]
            .as_str()
            .filter(|v| job_id(v))
            .ok_or_else(http::protocol)?;
        if !matches!(data["state"].as_str(), Some("queued" | "running")) {
            return Err(http::protocol());
        }
        return Ok(json!({
            "jobId": job,
            "state": data["state"],
            "expiresAt": data["expiresAt"],
            "complete": false,
            "operationId": id,
            "operationVersion": current["operationVersion"],
            "_meta": job_meta(&result),
        }));
    }
    let meta = &result["meta"];
    if result["operationId"] != id
        || meta["contractVersion"] != current["operationVersion"]
        || meta["contractDigest"] != current["contractDigest"]
    {
        return Err(contract_changed());
    }
    let validator = &cache.validators(current)?.output;
    if meta["state"] != "succeeded" || !validator.is_valid(data) {
        return Err(Failure::new(
            "UPSTREAM_CONTRACT_MISMATCH",
            5,
            "目标响应与输出合同不符；未返回数据。",
        ));
    }
    Ok(json!({
        "items": data,
        "permissionVerified": true,
        "onlineVerified": true,
        "_meta": project_meta(meta),
    }))
}

pub(super) async fn job_action(
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
    job_action_with(rt, &p, env, &client, operation, id).await
}

pub(super) async fn job_action_with(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    operation: &str,
    id: &str,
) -> Result<Value> {
    let result = operation == "jobs.result";
    // Only the result download gets the batch contract's size and time budget.
    let route = format!(
        "{JOBS_ROUTE}/{id}{}",
        match operation {
            "jobs.result" => "/result",
            "jobs.cancel" => "/cancel",
            _ => "",
        }
    );
    let cancel = operation == "jobs.cancel";
    let limit = if result {
        RESULT_BODY_LIMIT
    } else {
        BODY_LIMIT
    };
    let request = || async {
        let reply = authorized_limited(rt, p, env, client, limit, |token| {
            let url = endpoint(env, &route)?;
            Ok(if cancel {
                client.post(url).bearer_auth(token).json(&json!({}))
            } else {
                let request = client.get(url).bearer_auth(token);
                if result {
                    request.timeout(RESULT_TIMEOUT)
                } else {
                    request
                }
            })
        })
        .await?;
        let retry_after = reply.retry_after;
        Ok((envelope(reply)?, retry_after))
    };
    let (body, retry_after) = if cancel {
        request().await?
    } else {
        read_retry::run(request).await?
    };
    let data = &body["data"];
    if data["jobId"] != id || !data["state"].is_string() {
        return Err(http::protocol());
    }
    // Only a complete, succeeded job yields items; anything else is never shown as a result.
    if result
        && (data["state"] != "succeeded" || data["complete"] != true || !data["items"].is_array())
    {
        return Err(partial_result());
    }
    let mut value = data.clone();
    value["_meta"] = job_meta(&body);
    if let Some(seconds) = retry_after {
        value["_meta"]["retryAfterSeconds"] = json!(seconds);
    }
    Ok(value)
}

fn job_meta(body: &Value) -> Value {
    let mut meta = json!({"traceId":trace(body)});
    for key in [
        "operationId",
        "contractVersion",
        "contractDigest",
        "argumentsDigest",
    ] {
        if let Some(value) = body["meta"].get(key) {
            meta[key] = value.clone();
        }
    }
    meta
}
