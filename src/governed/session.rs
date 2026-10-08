use super::*;

pub(super) async fn revoke(
    client: &reqwest::Client,
    env: &profile::Environment,
    token: &str,
) -> bool {
    let Ok(url) = endpoint(env, REVOKE_ROUTE) else {
        return false;
    };
    client
        .post(url)
        .form(&[("client_id", CLIENT_ID), ("token", token)])
        .send()
        .await
        .is_ok_and(|r| r.status() == reqwest::StatusCode::OK)
}

/// A new-format credential: fixed prefix plus 256 random bits in base64url.
pub(super) fn issued_token<'a>(value: &'a Value, prefix: &str) -> Option<&'a str> {
    value.as_str().filter(|v| {
        v.len() == prefix.len() + 43
            && v.starts_with(prefix)
            && v[prefix.len()..]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}
pub(super) fn token_fields(token: &Value, system: &str, environment: &str) -> Result<()> {
    let expires_in = token["expires_in"]
        .as_i64()
        .filter(|v| (1..=43_200).contains(v));
    let deadline = token["authorizationExpiresAt"].as_str().and_then(rfc3339);
    let (Some(expires_in), Some(deadline)) = (expires_in, deadline) else {
        return Err(http::protocol());
    };
    // Access never outlives its parent authorization; allow for clock skew only.
    if token["token_type"] != "Bearer"
        || token["systemId"] != system
        || token["environment"] != environment
        || issued_token(&token["access_token"], "dtcli_g_a_").is_none()
        || issued_token(&token["refresh_token"], "dtcli_g_r_").is_none()
        || !token["authorizationId"].as_str().is_some_and(valid_handle)
        || chrono::Utc::now() + chrono::Duration::seconds(expires_in)
            > deadline + chrono::Duration::seconds(120)
    {
        return Err(http::protocol());
    }
    Ok(())
}
pub(super) fn expired(credentials: &Credentials) -> bool {
    credentials
        .access_expires_at
        .as_deref()
        .and_then(rfc3339)
        .is_none_or(|v| v <= chrono::Utc::now() + chrono::Duration::seconds(30))
}
/// Rotates the refresh token once under the authorization lock. `stale` is the record the
/// caller found unusable; a newer generation written meanwhile is reused instead.
pub(super) async fn refresh(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    stale: &Credentials,
) -> Result<Credentials> {
    let _lock = profile::authorization_lock(&rt.root, &p.authorization_id).await?;
    let mut credentials = rt
        .store
        .read(&p.credential_key)?
        .ok_or_else(auth_required)?;
    if credentials.generation != stale.generation && !expired(&credentials) {
        return Ok(credentials);
    }
    let recovering = credentials.refresh_state == "pending";
    let request_id = Zeroizing::new(if recovering {
        credentials.refresh_request_id.clone().ok_or_else(|| {
            Failure::new(
                "AUTH_REFRESH_OUTCOME_UNKNOWN",
                4,
                "旧版本的续期结果未知，没有可恢复的请求记录，请重新授权。",
            )
        })?
    } else {
        crate::login::random()
    });
    credentials.refresh_request_id = Some(request_id.to_string());
    credentials.refresh_state = "pending".into();
    rt.store.write(&p.credential_key, &credentials)?;
    let mut reply = send(client.post(endpoint(env, TOKEN_ROUTE)?).form(&[
        ("client_id", CLIENT_ID),
        ("grant_type", "refresh_token"),
        ("refresh_token", credentials.refresh_token.as_str()),
        ("request_id", request_id.as_str()),
    ]))
    .await?;
    if reply.status == 400 && reply.body["code"] == "INVALID_ARGUMENT" {
        if recovering {
            return Err(Failure::new(
                "AUTH_REFRESH_OUTCOME_UNKNOWN",
                4,
                "旧服务端不支持续期恢复，请重新授权。",
            ));
        }
        reply = send(client.post(endpoint(env, TOKEN_ROUTE)?).form(&[
            ("client_id", CLIENT_ID),
            ("grant_type", "refresh_token"),
            ("refresh_token", credentials.refresh_token.as_str()),
        ]))
        .await?;
    }
    if reply.status != 200 {
        let failure = remote_failure(&reply);
        // An explicit rejection is a known outcome; transport or server failures stay pending.
        if matches!(reply.status, 400 | 401 | 403) {
            credentials.refresh_state = "ready".into();
            credentials.refresh_request_id = None;
            rt.store.write(&p.credential_key, &credentials)?;
        }
        return Err(failure);
    }
    let token = reply.body;
    token_fields(&token, &p.system_id, &p.environment)?;
    if token["authorizationId"] != p.authorization_id
        || token["authorizationExpiresAt"] != p.authorization_expires_at
    {
        return Err(identity_mismatch());
    }
    credentials.access_token = token["access_token"].as_str().unwrap_or_default().into();
    credentials.refresh_token = token["refresh_token"].as_str().unwrap_or_default().into();
    credentials.generation = credentials
        .generation
        .checked_add(1)
        .ok_or_else(http::protocol)?;
    credentials.refresh_state = "ready".into();
    credentials.refresh_request_id = None;
    credentials.access_expires_at = Some(
        (chrono::Utc::now() + chrono::Duration::seconds(token["expires_in"].as_i64().unwrap_or(0)))
            .to_rfc3339(),
    );
    rt.store.write(&p.credential_key, &credentials)?;
    Ok(credentials)
}
pub(super) async fn access(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
) -> Result<Credentials> {
    if p.issuer != env.api_origin {
        return Err(profile_invalid());
    }
    if rfc3339(&p.authorization_expires_at).is_none_or(|v| v <= chrono::Utc::now()) {
        return Err(Failure::new(
            "AUTHORIZATION_EXPIRED",
            4,
            "受控访问授权已到期，请重新登录。",
        ));
    }
    let credentials = rt
        .store
        .read(&p.credential_key)?
        .ok_or_else(auth_required)?;
    if expired(&credentials) || credentials.refresh_state == "pending" {
        refresh(rt, p, env, client, &credentials).await
    } else {
        Ok(credentials)
    }
}
/// Sends a request with the governed access token; an explicit 401 allows one refresh.
pub(super) async fn authorized<F>(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    build: F,
) -> Result<Reply>
where
    F: Fn(&str) -> Result<reqwest::RequestBuilder>,
{
    authorized_limited(rt, p, env, client, BODY_LIMIT, build).await
}
pub(super) async fn authorized_limited<F>(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    limit: usize,
    build: F,
) -> Result<Reply>
where
    F: Fn(&str) -> Result<reqwest::RequestBuilder>,
{
    let credentials = access(rt, p, env, client).await?;
    let reply = send_limited(build(&credentials.access_token)?, limit).await?;
    let code = safe_code(&reply.body["error"]["code"]).or_else(|| safe_code(&reply.body["code"]));
    if reply.status != 401 || code == Some("AUTHORIZATION_REVOKED") {
        return Ok(reply);
    }
    let refreshed = refresh(rt, p, env, client, &credentials).await?;
    send_limited(build(&refreshed.access_token)?, limit).await
}

pub(super) async fn exchange(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    contract: &Value,
) -> Result<Zeroizing<String>> {
    let key = contract_key(contract)?;
    let contracts = serde_json::to_string(&[key.as_str()]).map_err(|_| invalid())?;
    let reply = authorized(rt, p, env, client, |subject| {
        Ok(client.post(endpoint(env, EXCHANGE_ROUTE)?).form(&[
            ("grant_type", TOKEN_EXCHANGE),
            ("subject_token", subject),
            ("subject_token_type", ACCESS_TOKEN_TYPE),
            ("requested_token_type", ACCESS_TOKEN_TYPE),
            ("audience", p.system_id.as_str()),
            ("operation_contracts", contracts.as_str()),
        ]))
    })
    .await?;
    let body = oauth(reply)?;
    let token = issued_token(&body["access_token"], "dtcli_g_s_").ok_or_else(http::protocol)?;
    if body["audience"] != p.system_id
        || body["token_type"] != "Bearer"
        || body["expires_in"]
            .as_i64()
            .is_none_or(|v| !(1..=300).contains(&v))
        || body["operation_contracts"] != json!([key])
    {
        return Err(http::protocol());
    }
    Ok(Zeroizing::new(token.to_owned()))
}
pub(super) async fn logout(rt: &Runtime, name: &str) -> Result<Value> {
    let _profile_lock = profile::lock(&rt.root, name).await?;
    let p = require_profile(rt, name)?;
    let env = rt.environment(&p.environment)?;
    let _lock = profile::authorization_lock(&rt.root, &p.authorization_id).await?;
    let credentials = rt.store.read(&p.credential_key)?;
    let remote = match credentials.as_ref() {
        Some(c) => {
            if revoke(&http::client()?, env, &c.refresh_token).await {
                "confirmed"
            } else {
                "unknown"
            }
        }
        None => "not-locatable",
    };
    if credentials.is_some() {
        rt.store.delete(&p.credential_key)?;
    }
    remove(&profile_path(&rt.root, name)?)?;
    remove(&cache_path(&rt.root, name)?)?;
    let result = json!({
        "provider": PROVIDER,
        "authorizationId": p.authorization_id,
        "remoteRevocation": remote,
        "localCleanup": "confirmed",
    });
    if remote == "unknown" {
        let mut failure = Failure::new(
            "LOGOUT_PARTIAL",
            7,
            "本地受控凭证已清理，远端撤销结果未知。",
        );
        failure.recovery = Some(Box::new(result));
        return Err(failure);
    }
    Ok(result)
}
