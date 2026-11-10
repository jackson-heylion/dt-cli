use super::*;
use crate::output::{PartialData, PartialMeta};

pub(super) fn validate_token(
    token: &TokenReply,
    environment: &str,
    system: &str,
    stored: Option<&PlatformProfile>,
) -> Result<()> {
    validate_token_for(
        token,
        environment,
        system,
        stored,
        stored.map(|p| p.purpose).unwrap_or_default(),
    )
}

pub(super) fn validate_token_for(
    token: &TokenReply,
    environment: &str,
    system: &str,
    stored: Option<&PlatformProfile>,
    purpose: SessionPurpose,
) -> Result<()> {
    let deadline = chrono::DateTime::parse_from_rfc3339(&token.authorization_expires_at)
        .map_err(|_| crate::http::protocol())?;
    let access_deadline = chrono::DateTime::parse_from_rfc3339(&token.access_expires_at)
        .map_err(|_| crate::http::protocol())?;
    let issued = chrono::DateTime::parse_from_rfc3339(&token.server_time)
        .map_err(|_| crate::http::protocol())?;
    if access_deadline > deadline
        || access_deadline - issued != chrono::Duration::seconds(token.expires_in)
    {
        return Err(crate::http::protocol());
    }
    if token.token_type != "Bearer"
        || token.environment != environment
        || (token.system_id != "platform" && token.system_id != system)
        || !handle(&token.authorization_id)
        || !(1..=43200).contains(&token.expires_in)
        || !token
            .access_token
            .strip_prefix(purpose.access_prefix())
            .is_some_and(handle)
        || !token
            .refresh_token
            .strip_prefix(purpose.refresh_prefix())
            .is_some_and(handle)
        || access_deadline < chrono::Utc::now() - chrono::Duration::seconds(30)
        || stored.is_some_and(|p| {
            p.session_id != token.authorization_id || p.expires_at != token.authorization_expires_at
        })
    {
        return Err(crate::http::protocol());
    }
    Ok(())
}

pub(super) fn credentials(token: TokenReply, generation: u64) -> Credentials {
    Credentials {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        version: 1,
        access_expires_at: Some(token.access_expires_at),
        authorization_expires_at: Some(token.authorization_expires_at),
        generation,
        refresh_state: "ready".into(),
        refresh_request_id: None,
    }
}

pub(super) async fn access(rt: &Runtime, stored: &PlatformProfile) -> Result<Credentials> {
    let _lock = profile::authorization_lock(&rt.root, &stored.session_id).await?;
    let prefix = format!(
        "{}:{}:",
        stored.purpose.directory(),
        transport::challenge(&stored.origin)
    );
    let valid = stored
        .credential_key
        .strip_prefix(&prefix)
        .and_then(|tail| tail.rsplit_once(':'))
        .is_some_and(|(name, nonce)| profile::validate_name(name).is_ok() && handle(nonce));
    if !valid {
        return Err(required());
    }
    let mut current = rt
        .store
        .read(&stored.credential_key)?
        .ok_or_else(required)?;
    if !current
        .access_token
        .strip_prefix(stored.purpose.access_prefix())
        .is_some_and(handle)
        || !current
            .refresh_token
            .strip_prefix(stored.purpose.refresh_prefix())
            .is_some_and(handle)
    {
        return Err(required());
    }
    if chrono::DateTime::parse_from_rfc3339(&stored.expires_at)
        .map_err(|_| crate::http::protocol())?
        <= chrono::Utc::now()
    {
        return Err(required());
    }
    let valid = current
        .access_expires_at
        .as_deref()
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
        .is_some_and(|v| v > chrono::Utc::now() + chrono::Duration::seconds(30));
    if current.refresh_state != "pending" && valid {
        return Ok(current);
    }
    let request_id = if current.refresh_state == "pending" {
        current
            .refresh_request_id
            .clone()
            .filter(|v| handle(v))
            .ok_or_else(required)?
    } else {
        login::random()
    };
    current.refresh_state = "pending".into();
    current.refresh_request_id = Some(request_id.clone());
    rt.store.write(&stored.credential_key, &current)?;
    // 发送前持久化请求标识；网络异常保留 pending，只用相同请求标识恢复。
    let token: TokenReply = transport::request(
        transport::client()?
            .post(format!(
                "{}{}/oauth2/token",
                stored.origin,
                stored.purpose.namespace()
            ))
            .form(&[
                ("client_id", "dt-cli"),
                ("grant_type", "refresh_token"),
                ("refresh_token", current.refresh_token.as_str()),
                ("request_id", request_id.as_str()),
            ]),
    )
    .await?;
    validate_token(&token, &stored.environment, &stored.system_id, Some(stored))?;
    let generation = current
        .generation
        .checked_add(1)
        .ok_or_else(crate::http::protocol)?;
    super::metadata::invalidate(stored);
    let next = credentials(token, generation);
    rt.store.write(&stored.credential_key, &next)?;
    Ok(next)
}
fn required() -> Failure {
    Failure::new("AUTH_REQUIRED", 4, "平台会话已失效，请重新登录。")
}

pub(super) async fn me(rt: &Runtime, stored: &PlatformProfile) -> Result<SessionView> {
    let credential = access(rt, stored).await?;
    let me: SessionView = transport::request(
        transport::client()?
            .get(format!(
                "{}{}/me",
                stored.origin,
                stored.purpose.namespace()
            ))
            .bearer_auth(&credential.access_token),
    )
    .await?;
    if me.session_id != stored.session_id
        || me.subject_id != stored.subject_id
        || me.domain != stored.domain
        || me.purpose != stored.purpose.text()
        || me.revoked
        || me.expires_at != stored.expires_at
    {
        return Err(crate::http::protocol());
    }
    Ok(me)
}
pub(super) async fn list(rt: &Runtime, stored: &PlatformProfile, page: u32) -> Result<SessionPage> {
    let credential = access(rt, stored).await?;
    transport::request(
        transport::client()?
            .get(format!(
                "{}/platform/cli/sessions?page={page}&pageSize=20",
                stored.origin
            ))
            .bearer_auth(&credential.access_token),
    )
    .await
}
pub(super) async fn revoke(rt: &Runtime, stored: &PlatformProfile, id: &str) -> Result<()> {
    let credential = access(rt, stored).await?;
    transport::empty(
        transport::client()?
            .delete(format!("{}/platform/cli/sessions/{id}", stored.origin))
            .bearer_auth(&credential.access_token),
    )
    .await
}
pub(super) async fn logout(
    rt: &Runtime,
    name: &str,
    stored: &PlatformProfile,
) -> Result<LogoutView> {
    // 与登录采用同一名称/用途锁，等待后重读，绝不删除并发新会话。
    let _profile_lock =
        profile::lock(&rt.root, &format!("{}-{name}", stored.purpose.directory())).await?;
    let path = profile_path_for(rt, name, stored.purpose);
    let Some(current) = private_store::read::<PlatformProfile>(&path)? else {
        return Ok(absent_logout());
    };
    if current.session_id != stored.session_id || current.credential_key != stored.credential_key {
        return Err(logout_failure(
            stored,
            name,
            LogoutView {
                remote_revocation: "not-attempted",
                local_cleanup: "preserved-new-session",
                credential_cleanup: "preserved",
                profile_cleanup: "preserved",
                context_cleanup: "preserved",
            },
            None,
        ));
    }
    let _lock = profile::authorization_lock(&rt.root, &stored.session_id).await?;
    let credential = rt.store.read(&stored.credential_key);
    let mut local_error = None;
    let remote = match &credential {
        Ok(Some(credential)) => {
            let result = async {
                transport::empty(
                    transport::client()?
                        .post(format!(
                            "{}{}/oauth2/revoke",
                            stored.origin,
                            stored.purpose.namespace()
                        ))
                        .form(&[
                            ("client_id", "dt-cli"),
                            ("token", credential.refresh_token.as_str()),
                        ]),
                )
                .await
            }
            .await;
            if result.is_ok() {
                "confirmed"
            } else {
                "unknown"
            }
        }
        _ => "not-locatable",
    };
    let credential_cleanup = match credential {
        Ok(None) => "not-present",
        Ok(Some(_)) => match rt.store.delete(&stored.credential_key) {
            Ok(()) => "confirmed",
            Err(e) => {
                local_error = Some(e);
                "failed"
            }
        },
        Err(e) => {
            local_error = Some(e);
            "failed"
        }
    };
    // 删除失败时保留本人记录作为明确的清理定位，不保留后台补偿凭证。
    let profile_cleanup = if credential_cleanup == "failed" {
        "preserved"
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => "confirmed",
            Err(e) => {
                local_error = Some(private_store::io_failure("delete_profile", e));
                "failed"
            }
        }
    };
    let context_cleanup = match context::clear_session(rt, name, stored) {
        Ok(state) => state,
        Err(e) => {
            local_error = Some(e);
            "failed"
        }
    };
    metadata::invalidate(stored);
    let view = LogoutView {
        remote_revocation: remote,
        local_cleanup: if local_error.is_none() {
            "confirmed"
        } else {
            "failed"
        },
        credential_cleanup,
        profile_cleanup,
        context_cleanup,
    };
    if remote != "confirmed" || local_error.is_some() {
        return Err(logout_failure(stored, name, view, local_error));
    }
    Ok(view)
}
pub(super) fn absent_logout() -> LogoutView {
    LogoutView {
        remote_revocation: "not-locatable",
        local_cleanup: "not-present",
        credential_cleanup: "not-present",
        profile_cleanup: "not-present",
        context_cleanup: "not-present",
    }
}
fn logout_failure(
    stored: &PlatformProfile,
    name: &str,
    view: LogoutView,
    cause: Option<Failure>,
) -> Failure {
    let mut error = Failure::new(
        "LOGOUT_PARTIAL",
        7,
        "本机清理与远端撤销未全部确认，请分别核对结果。",
    );
    error.details = cause.and_then(|e| e.details);
    let mut argv = vec!["dt-cli".into()];
    if stored.purpose == SessionPurpose::Management {
        argv.push("manage".into());
    }
    argv.extend([
        "login".into(),
        format!("--profile={name}"),
        "--environment".into(),
        stored.environment.clone(),
    ]);
    error.recovery = Some(crate::output::Recovery {
        profile: Some(name.into()),
        environment: Some(stored.environment.clone()),
        purpose: if stored.purpose == SessionPurpose::Management {
            "management"
        } else {
            "business"
        }
        .into(),
        actions: vec![crate::output::RecoveryAction {
            id: "open-session-management",
            argv,
            requires_interaction: true,
            reason: "沿用原本人及用途建立有效会话，核对远端会话撤销；不重复业务写入。",
        }],
        ..Default::default()
    });
    error.with_partial(PartialData::Logout(view), PartialMeta::default())
}
