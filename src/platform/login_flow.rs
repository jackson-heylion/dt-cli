use super::*;
use crate::output::{CredentialCleanup, LoginSave, PartialData, PartialMeta, RecoveryAction};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct LoginResult {
    /// 已验证且保存的会话，沿用既有登录输出字段。
    #[serde(flatten)]
    session: SessionView,
    /// 登录已完成后仍需处理的本机清理告警，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    warning: Option<LoginWarning>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum LoginWarning {
    PreviousCredentialCleanupFailed,
}

struct LoginIo<'a> {
    /// 预检、换码、会话确认及补偿使用的同一 HTTP 客户端。
    client: &'a reqwest::Client,
    /// 原账号锁内原子保存本地账号记录的入口。
    write_profile: fn(&std::path::Path, &PlatformProfile) -> Result<()>,
}

impl<'a> LoginIo<'a> {
    fn new(client: &'a reqwest::Client) -> Self {
        Self {
            client,
            write_profile: private_store::write,
        }
    }
}

pub(super) async fn login_platform(
    rt: &Runtime,
    name: &str,
    endpoint: &endpoints::PlatformEndpoint,
    system: &str,
    no_browser: bool,
    purpose: SessionPurpose,
) -> Result<LoginResult> {
    let client = transport::client().map_err(|error| at_stage(error, "checking_network"))?;
    login_with_client(
        rt,
        name,
        endpoint,
        system,
        no_browser,
        purpose,
        LoginIo::new(&client),
    )
    .await
}

async fn login_with_client(
    rt: &Runtime,
    name: &str,
    endpoint: &endpoints::PlatformEndpoint,
    system: &str,
    no_browser: bool,
    purpose: SessionPurpose,
    io: LoginIo<'_>,
) -> Result<LoginResult> {
    let client = io.client;
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
            Ok(session) => {
                return Ok(LoginResult {
                    session,
                    warning: None,
                });
            }
            Err(failure) if failure.code == "AUTH_REQUIRED" => {}
            Err(failure) => return Err(at_stage(failure, "checking_existing_session")),
        }
        metadata::invalidate(&previous);
        Some(previous)
    } else {
        None
    };
    login::progress("checking_local_storage");
    rt.store.preflight()?;
    private_store::probe(&rt.root.join(purpose.directory()))?;
    login::progress("checking_network");
    super::login_preflight::check(client, origin, namespace)
        .await
        .map_err(|error| at_stage(error, "checking_network"))?;
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
        client
            .post(format!("{origin}{namespace}/oauth2/token"))
            .form(&[
                ("client_id", "dt-cli"),
                ("grant_type", "authorization_code"),
                ("code", code.as_str()),
                ("redirect_uri", redirect.as_str()),
                ("code_verifier", verifier.as_str()),
            ]),
    )
    .await
    .map_err(|error| {
        let mut error = browser_login.validated_failure(error, "exchanging_code");
        error
            .details
            .get_or_insert_with(Default::default)
            .authorization_code_status = Some("unknown".into());
        error.hint = "尚未确认换码结果，请勿重放原授权码；排除网络或接口错误后重新发起登录。";
        error
    })?;
    session::validate_token_for(&token, environment, system, None, purpose)
        .map_err(|error| browser_login.validated_failure(error, "checking_token"))?;
    login::progress("checking_session");
    let me: SessionView = transport::request(
        client
            .get(format!("{origin}{namespace}/me"))
            .bearer_auth(&token.access_token),
    )
    .await
    .map_err(|error| browser_login.validated_failure(error, "checking_session"))?;
    if me.session_id != token.authorization_id
        || me.purpose != purpose.text()
        || me.revoked
        || me.expires_at != token.authorization_expires_at
        || me.domain != IdentityDomain::Employee
    {
        return Err(browser_login.validated_failure(crate::http::protocol(), "checking_session"));
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
        let outcome = Box::pin(compensate_save(rt, client, &stored, &credentials)).await;
        return Err(browser_login
            .validated_failure(error, "saving_local_session")
            .with_partial(PartialData::Empty, outcome));
    }
    if let Err(error) = (io.write_profile)(&path, &stored) {
        let error = browser_login.validated_failure(error, "saving_local_session");
        // 替换文件后仍可能报错。原账号锁内确认未切换，才处置本次新会话。
        let not_switched = match private_store::read::<PlatformProfile>(&path) {
            Ok(None) => true,
            Ok(Some(actual)) => previous.as_ref().is_some_and(|old| {
                old.credential_key == actual.credential_key && old.session_id == actual.session_id
            }),
            Err(_) => false,
        };
        if not_switched {
            let outcome = Box::pin(compensate_save(rt, client, &stored, &credentials)).await;
            return Err(error.with_partial(PartialData::Empty, outcome));
        }
        return Err(unconfirmed_save(error, name, &stored));
    }
    let warning = previous
        .filter(|previous| rt.store.delete(&previous.credential_key).is_err())
        .map(|_| LoginWarning::PreviousCredentialCleanupFailed);
    login::progress("complete");
    Ok(LoginResult {
        session: me,
        warning,
    })
}

async fn compensate_save(
    rt: &Runtime,
    client: &reqwest::Client,
    stored: &PlatformProfile,
    credentials: &Credentials,
) -> PartialMeta {
    let revoked = transport::empty(
        client
            .post(format!(
                "{}{}/oauth2/revoke",
                stored.origin,
                stored.purpose.namespace()
            ))
            .form(&[
                ("client_id", "dt-cli"),
                ("token", credentials.refresh_token.as_str()),
            ]),
    )
    .await
    .is_ok();
    let cleanup = if rt.store.delete(&stored.credential_key).is_ok() {
        CredentialCleanup::Confirmed
    } else {
        CredentialCleanup::Failed
    };
    PartialMeta {
        login_save: Some(LoginSave::NotSaved),
        remote_revocation: Some(if revoked { "confirmed" } else { "unknown" }),
        credential_cleanup: Some(cleanup),
        ..Default::default()
    }
}

fn unconfirmed_save(mut error: Failure, name: &str, stored: &PlatformProfile) -> Failure {
    let mut recovery = super::recovery::bound(name, stored);
    let mut argv = vec!["dt-cli".into()];
    if stored.purpose == SessionPurpose::Management {
        argv.push("manage".into());
    }
    argv.extend(["check".into(), format!("--profile={name}")]);
    recovery.actions.push(RecoveryAction {
        id: "check-login-save",
        argv,
        reason: "核对原账号记录和对应会话，暂不重新登录。",
        requires_interaction: false,
    });
    error.recovery = Some(recovery);
    error.hint = "登录保存待核对，已保留相关凭证；请使用对应账号的 check 核对。";
    error.with_partial(
        PartialData::Empty,
        PartialMeta {
            login_save: Some(LoginSave::Unconfirmed),
            remote_revocation: Some("not-attempted"),
            credential_cleanup: Some(CredentialCleanup::NotAttempted),
            ..Default::default()
        },
    )
}

fn at_stage(mut failure: Failure, stage: &'static str) -> Failure {
    let details = failure.details.get_or_insert_with(Default::default);
    details.login_stage = Some(stage.into());
    details.process_id = Some(std::process::id());
    failure.retryable = false;
    failure
}

#[cfg(test)]
mod tests;
