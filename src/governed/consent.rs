use super::*;

pub(super) async fn login_profile(
    rt: &Runtime,
    name: &str,
    environment: Option<&str>,
    system: Option<&str>,
    method: Option<&str>,
    interaction: Option<&str>,
) -> Result<Value> {
    login::validate_method(method)?;
    login::validate_interaction(interaction)?;
    if !rt.interactive && interaction != Some("browser") {
        return Err(Failure::new(
            "INTERACTION_REQUIRED",
            8,
            "请由员工在交互终端显式执行登录。",
        ));
    }
    let cancellation = tokio::signal::ctrl_c();
    tokio::pin!(cancellation);
    let cancelled = || Failure::new("LOGIN_CANCELLED", 8, "登录已取消。");
    let _lock = tokio::select! {
        biased;
        _ = &mut cancellation => return Err(cancelled()),
        lock = profile::lock(&rt.root, name) => lock?,
    };
    if profile::read(&rt.root, name)?.is_some() {
        return Err(provider_mismatch());
    }
    let previous = read_profile(&rt.root, name)?;
    let env_name = environment
        .or_else(|| previous.as_ref().map(|p| p.environment.as_str()))
        .ok_or_else(invalid)?;
    let system = system
        .or_else(|| previous.as_ref().map(|p| p.system_id.as_str()))
        .ok_or_else(invalid)?;
    if !valid_id(env_name)
        || !valid_id(system)
        || previous
            .as_ref()
            .is_some_and(|p| p.environment != env_name || p.system_id != system)
    {
        return Err(invalid());
    }
    let env = rt.environment(env_name)?;
    let client = http::client()?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| http::network())?;
    let redirect = format!(
        "http://127.0.0.1:{}/oauth/callback",
        listener.local_addr().map_err(|_| http::network())?.port()
    );
    let state = login::random();
    let verifier = Zeroizing::new(login::random());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = url::Url::parse(&endpoint(env, AUTHORIZE_ROUTE)?).map_err(|_| invalid())?;
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", redirect.as_str()),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("systemId", system),
        ("environment", env_name),
    ]);
    if let Some(method) = method {
        url.query_pairs_mut().append_pair("login_method", method);
    }
    rt.browser.open(url.as_str())?;
    let code = tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(300),
            login::callback(&listener, &state, &env.api_origin)) =>
            result.map_err(|_| Failure::new("TIMEOUT", 5, "等待浏览器确认超时。"))??,
        _ = &mut cancellation => return Err(cancelled()),
    };
    let code = Zeroizing::new(code);
    drop(listener);
    // The authorization code is single-use: the exchange is never retried.
    let token = oauth(
        send(client.post(endpoint(env, TOKEN_ROUTE)?).form(&[
            ("grant_type", "authorization_code"),
            ("client_id", CLIENT_ID),
            ("code", code.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("code_verifier", verifier.as_str()),
        ]))
        .await?,
    )?;
    token_fields(&token, system, env_name)?;
    let expires_in = token["expires_in"].as_i64().unwrap_or(0);
    let authorization_id = token["authorizationId"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let credentials = Credentials {
        access_token: token["access_token"].as_str().unwrap_or_default().into(),
        refresh_token: token["refresh_token"].as_str().unwrap_or_default().into(),
        version: 1,
        access_expires_at: Some(
            (chrono::Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339(),
        ),
        authorization_expires_at: token["authorizationExpiresAt"].as_str().map(str::to_owned),
        generation: 0,
        refresh_state: "ready".into(),
        refresh_request_id: None,
    };
    let mut written = None;
    let mut cache_written = false;
    let outcome = async {
        if previous
            .as_ref()
            .is_some_and(|p| p.authorization_id == authorization_id)
        {
            return Err(http::protocol());
        }
        let body = envelope(
            send(
                client
                    .get(endpoint(env, CATALOG_ROUTE)?)
                    .bearer_auth(&credentials.access_token),
            )
            .await?,
        )?;
        let data = &body["data"];
        let subject = data["subject"]["id"]
            .as_str()
            .filter(|v| valid_id(v))
            .ok_or_else(http::protocol)?;
        if previous.as_ref().is_some_and(|p| p.subject_id != subject) {
            return Err(Failure::new(
                "IDENTITY_MISMATCH",
                4,
                "浏览器员工与原受控 profile 不一致；请为另一员工创建新 profile。",
            ));
        }
        let mut p = GovernedProfile {
            provider: PROVIDER.into(),
            issuer: env.api_origin.clone(),
            environment: env_name.into(),
            system_id: system.into(),
            subject_id: subject.into(),
            authorization_id: authorization_id.clone(),
            authorization_expires_at: token["authorizationExpiresAt"]
                .as_str()
                .unwrap_or_default()
                .into(),
            access_expires_at: credentials.access_expires_at.clone().unwrap_or_default(),
            credential_key: String::new(),
        };
        p.credential_key = identity_key(&p);
        let cache = cache_from_data(data, &p)?;
        written = Some(p.credential_key.clone());
        rt.store.write(&p.credential_key, &credentials)?;
        // Read back before publishing a profile that refers to the record.
        let stored = rt.store.read(&p.credential_key)?.ok_or_else(storage)?;
        if stored.access_token != credentials.access_token
            || stored.refresh_token != credentials.refresh_token
        {
            return Err(storage());
        }
        cache_written = true;
        save_private(&cache_path(&rt.root, name)?, &cache)?;
        // The profile file is the commit point.
        save_private(&profile_path(&rt.root, name)?, &p)?;
        Ok::<(GovernedProfile, Cache), Failure>((p, cache))
    };
    let outcome = tokio::select! {
        result = outcome => result,
        _ = &mut cancellation => Err(cancelled()),
    };
    let (p, cache) = match outcome {
        Ok(value) => value,
        Err(mut failure) => {
            let remote = revoke(&client, env, &credentials.refresh_token).await;
            let cleanup = match written.as_deref() {
                Some(key) if rt.store.delete(key).is_ok() => "confirmed",
                Some(_) => "unknown",
                None => "not-created",
            };
            if cache_written {
                // The previous cache, if any, was replaced; the next sync rebuilds it.
                let _ = cache_path(&rt.root, name).map(|path| remove(&path));
            }
            failure.recovery = Some(Box::new(json!({
                "authorizationId": authorization_id,
                "remoteRevocation": if remote { "confirmed" } else { "unknown" },
                "newCredentialCleanup": cleanup,
                "originalProfile": "preserved",
            })));
            return Err(failure);
        }
    };
    let mut previous_revocation = "not-applicable";
    if let Some(old) = previous.filter(|old| old.authorization_id != p.authorization_id) {
        previous_revocation = match rt.store.read(&old.credential_key) {
            Ok(Some(old_credentials)) => {
                if revoke(&client, env, &old_credentials.refresh_token).await {
                    if rt.store.delete(&old.credential_key).is_ok() {
                        "confirmed"
                    } else {
                        "confirmed-local-cleanup-failed"
                    }
                } else {
                    "unknown"
                }
            }
            Ok(None) => "not-locatable",
            Err(_) => "unknown",
        };
    }
    Ok(json!({
        "provider": PROVIDER,
        "environment": p.environment,
        "systemId": p.system_id,
        "subjectId": p.subject_id,
        "authorizationId": p.authorization_id,
        "authorizationExpiresAt": p.authorization_expires_at,
        "catalogRevision": cache.catalog_revision,
        "operationCount": cache.operations.len(),
        "consentedCount": consented(&cache),
        "previousAuthorizationRevocation": previous_revocation,
        "permissionVerified": true,
        "onlineVerified": true,
    }))
}
