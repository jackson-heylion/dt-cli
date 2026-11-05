use super::*;

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
    let _lock = profile::authorization_lock(&rt.root, &stored.session_id).await?;
    let credential = rt
        .store
        .read(&stored.credential_key)?
        .ok_or_else(required)?;
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
    .await?;
    rt.store.delete(&stored.credential_key)?;
    std::fs::remove_file(profile_path_for(rt, name, stored.purpose))
        .map_err(|_| crate::output::storage())?;
    Ok(LogoutView {
        remote_revocation: "confirmed",
        local_cleanup: "confirmed",
    })
}
