use super::*;
use crate::output::{PartialData, PartialMeta};

pub(super) async fn login_platform(
    rt: &Runtime,
    name: &str,
    endpoint: &endpoints::PlatformEndpoint,
    system: &str,
    no_browser: bool,
    purpose: SessionPurpose,
) -> Result<SessionView> {
    let origin = endpoint.origin();
    let environment = endpoint.environment();
    let namespace = purpose.namespace();
    let path = profile_path_for(rt, name, purpose);
    for value in [environment, system] {
        if value.is_empty()
            || value.len() > 100
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-".contains(&b))
        {
            return Err(invalid());
        }
    }
    let _lock = profile::lock(&rt.root, &format!("{}-{name}", purpose.directory())).await?;
    let previous = if path.exists() {
        let previous = read_profile_for(rt, name, purpose)?;
        if previous.origin != origin || previous.environment != environment {
            return Err(Failure::new(
                "PROFILE_EXISTS",
                2,
                "此记录属于其他平台或环境，请明确选择对应记录。",
            ));
        }
        match session::me(rt, &previous).await {
            Ok(me) => return Ok(me),
            Err(failure) if failure.code == "AUTH_REQUIRED" => {}
            Err(failure) => return Err(failure),
        }
        metadata::invalidate(&previous);
        Some(previous)
    } else {
        None
    };
    login::progress("checking_local_storage");
    rt.store.preflight()?;
    private_store::probe_metadata(&rt.root.join(purpose.directory()))?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| transport::network())?;
    let redirect = format!(
        "http://127.0.0.1:{}/oauth/callback",
        listener
            .local_addr()
            .map_err(|_| transport::network())?
            .port()
    );
    let state = login::random();
    let verifier = zeroize::Zeroizing::new(login::random());
    let challenge = transport::challenge(&verifier);
    let mut url =
        url::Url::parse(&format!("{origin}{namespace}/oauth2/authorize")).map_err(|_| invalid())?;
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", "dt-cli"),
        ("redirect_uri", redirect.as_str()),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("system_id", system),
        ("environment", environment),
    ]);
    let browser_login = login::browser::BrowserLogin::begin(
        rt.browser.as_ref(),
        url.as_str(),
        listener.local_addr().map_err(|_| transport::network())?,
        no_browser,
    );
    let code = tokio::select! {
        result = browser_login.wait(&listener, &state, origin) => result?,
        _ = tokio::signal::ctrl_c() => return Err(Failure::new("LOGIN_CANCELLED", 8, "登录已取消。")),
    };
    drop(listener);
    login::progress("exchanging_code");
    let token: TokenReply = transport::request(
        transport::client()?
            .post(format!("{origin}{namespace}/oauth2/token"))
            .form(&[
                ("client_id", "dt-cli"),
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", redirect.as_str()),
                ("code_verifier", verifier.as_str()),
            ]),
    )
    .await?;
    session::validate_token_for(&token, environment, system, None, purpose)?;
    login::progress("checking_session");
    let me: SessionView = transport::request(
        transport::client()?
            .get(format!("{origin}{namespace}/me"))
            .bearer_auth(&token.access_token),
    )
    .await?;
    if me.session_id != token.authorization_id
        || me.purpose != purpose.text()
        || me.revoked
        || me.expires_at != token.authorization_expires_at
        || me.domain != IdentityDomain::Employee
    {
        return Err(crate::http::protocol());
    }
    let key = format!(
        "{}:{}:{name}:{}",
        purpose.directory(),
        transport::challenge(origin),
        login::random()
    );
    let stored = PlatformProfile {
        purpose,
        schema_version: 1,
        origin: origin.into(),
        environment: environment.into(),
        system_id: token.system_id.clone(),
        subject_id: me.subject_id.clone(),
        domain: me.domain.clone(),
        session_id: me.session_id.clone(),
        expires_at: me.expires_at.clone(),
        credential_key: key.clone(),
    };
    let credentials = session::credentials(token, 0);
    login::progress("saving_local_session");
    if let Err(error) = rt.store.write(&key, &credentials) {
        let revoked = transport::empty(
            transport::client()?
                .post(format!("{origin}{namespace}/oauth2/revoke"))
                .form(&[
                    ("client_id", "dt-cli"),
                    ("token", credentials.refresh_token.as_str()),
                ]),
        )
        .await
        .is_ok();
        return Err(error.with_partial(
            PartialData::Empty,
            PartialMeta {
                remote_revocation: Some(if revoked { "confirmed" } else { "unknown" }),
                ..Default::default()
            },
        ));
    }
    if let Err(error) = private_store::write(&path, &stored) {
        // 撤销确认前保留安全存储内容，以便明确报告清理边界。
        let _ = transport::empty(
            transport::client()?
                .post(format!("{origin}{namespace}/oauth2/revoke"))
                .form(&[
                    ("client_id", "dt-cli"),
                    ("token", credentials.refresh_token.as_str()),
                ]),
        )
        .await;
        rt.store.delete(&key)?;
        return Err(error);
    }
    if let Some(previous) = previous {
        rt.store.delete(&previous.credential_key)?;
    }
    login::progress("complete");
    Ok(me)
}
