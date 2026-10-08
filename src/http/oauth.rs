use super::*;

#[derive(Deserialize)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub scope: String,
    pub authorization_id: String,
    pub authorization_expires_at: String,
    pub server_time: String,
}
/// The internal launch payload. The URL is only ever handed to the system browser.
pub async fn token(
    client: &reqwest::Client,
    env: &Environment,
    code: &str,
    redirect: &str,
    verifier: &str,
) -> Result<Token> {
    let r = client
        .post(format!(
            "{}/cli/oauth2/token",
            env.api_origin.trim_end_matches('/')
        ))
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", "dt-cli"),
            ("code", code),
            ("redirect_uri", redirect),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .map_err(|_| network())?;
    let status = r.status();
    let body = bounded(r).await?;
    if status != reqwest::StatusCode::OK {
        if status.is_server_error()
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status.is_redirection()
        {
            return Err(Failure::new(
                "DEPENDENCY_UNAVAILABLE",
                5,
                "IAM 授权交换暂不可用；未重试交换。",
            ));
        }
        return Err(Failure::new(
            "AUTH_DENIED",
            4,
            "授权码交换被拒绝，请重新发起授权。",
        ));
    }
    serde_json::from_value(body).map_err(|_| network())
}
pub async fn refresh(client: &reqwest::Client, env: &Environment, refresh: &str) -> Result<Token> {
    let r = client
        .post(format!(
            "{}/cli/oauth2/token",
            env.api_origin.trim_end_matches('/')
        ))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", "dt-cli"),
            ("refresh_token", refresh),
        ])
        .send()
        .await
        .map_err(|_| {
            Failure::new(
                "AUTH_REFRESH_OUTCOME_UNKNOWN",
                3,
                "续期结果未知；不会重发旧刷新凭证。",
            )
        })?;
    let status = r.status();
    let body = bounded(r).await.map_err(|_| {
        Failure::new(
            "AUTH_REFRESH_OUTCOME_UNKNOWN",
            3,
            "续期结果未知；不会重发旧刷新凭证。",
        )
    })?;
    if status != reqwest::StatusCode::OK {
        if !matches!(status.as_u16(), 400 | 401 | 403) {
            return Err(Failure::new(
                "AUTH_REFRESH_OUTCOME_UNKNOWN",
                3,
                "续期结果未知；不会重发旧刷新凭证。",
            ));
        }
        return Err(Failure::new(
            "AUTH_REQUIRED",
            3,
            "授权续期被拒绝，请重新授权。",
        ));
    }
    serde_json::from_value(body).map_err(|_| {
        Failure::new(
            "AUTH_REFRESH_OUTCOME_UNKNOWN",
            3,
            "续期结果未知；不会重发旧刷新凭证。",
        )
    })
}
/// Recovery is tied to the same refresh generation. Old servers may reject the extension
/// before consuming anything; only a fresh attempt may then use the legacy exchange.
pub async fn refresh_recoverable(
    client: &reqwest::Client,
    env: &Environment,
    refresh_token: &str,
    request_id: &str,
    recovering: bool,
) -> Result<Token> {
    let unknown = || {
        Failure::new(
            "AUTH_REFRESH_OUTCOME_UNKNOWN",
            3,
            "续期结果未知；可在两分钟内重新执行以恢复同一次续期，超过窗口需重新授权。",
        )
    };
    let response = client
        .post(format!(
            "{}/cli/oauth2/token",
            env.api_origin.trim_end_matches('/')
        ))
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", "dt-cli"),
            ("refresh_token", refresh_token),
            ("request_id", request_id),
        ])
        .send()
        .await
        .map_err(|_| unknown())?;
    let status = response.status();
    let body = bounded(response).await.map_err(|_| unknown())?;
    if status.as_u16() == 400
        && matches!(
            body["error"].as_str(),
            Some("invalid_request" | "RECOVERY_UNAVAILABLE")
        )
    {
        return if recovering {
            Err(unknown())
        } else {
            refresh(client, env, refresh_token).await
        };
    }
    if status != reqwest::StatusCode::OK {
        return Err(if matches!(status.as_u16(), 400 | 401 | 403) {
            Failure::new(
                "AUTH_REQUIRED",
                3,
                "授权已失效或恢复窗口已结束，请重新授权。",
            )
        } else {
            unknown()
        });
    }
    serde_json::from_value(body).map_err(|_| unknown())
}

impl Token {
    pub(crate) fn access_expiry(&self) -> Result<String> {
        let server =
            chrono::DateTime::parse_from_rfc3339(&self.server_time).map_err(|_| protocol())?;
        let deadline = chrono::DateTime::parse_from_rfc3339(&self.authorization_expires_at)
            .map_err(|_| protocol())?;
        if self.token_type != "Bearer"
            || !self.access_token.starts_with("dtcli_a_")
            || !self.refresh_token.starts_with("dtcli_r_")
            || !(1..=43200).contains(&self.expires_in)
            || self.scope != "iam.profile.read iam.apps.read iam.workflow.read iam.workflow.open"
        {
            return Err(protocol());
        }
        let expires = server
            .checked_add_signed(chrono::Duration::seconds(self.expires_in))
            .filter(|expires| *expires <= deadline)
            .ok_or_else(protocol)?;
        Ok(expires.to_rfc3339())
    }
}
pub async fn revoke(client: &reqwest::Client, env: &Environment, token: &str) -> bool {
    client
        .post(format!(
            "{}/cli/oauth2/revoke",
            env.api_origin.trim_end_matches('/')
        ))
        .form(&[
            ("client_id", "dt-cli"),
            ("token", token),
            ("token_type_hint", "refresh_token"),
        ])
        .send()
        .await
        .is_ok_and(|r| r.status() == reqwest::StatusCode::OK)
}
