use crate::{
    Runtime,
    credentials::Credentials,
    http,
    output::{Failure, Result, invalid},
    profile::{self, Environment, Profile},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, io::IsTerminal, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub trait Browser: Send + Sync {
    fn open(&self, url: &str) -> Result<()>;
}
pub struct SystemBrowser;
impl Browser for SystemBrowser {
    fn open(&self, url: &str) -> Result<()> {
        webbrowser::open(url)
            .map_err(|_| Failure::new("BROWSER_OPEN_FAILED", 6, "浏览器未能启动。"))
    }
}
/// Progress is separate from the final stdout envelope and contains no OAuth material.
pub(crate) fn progress(stage: &'static str) {
    use std::io::Write;
    let _ = writeln!(
        std::io::stderr().lock(),
        "{}",
        serde_json::json!({"event":"auth.login.progress","stage":stage})
    );
}
pub(crate) fn preflight(rt: &Runtime, governed: bool) -> Result<()> {
    progress("checking_local_storage");
    rt.store.preflight()?;
    let metadata = if governed {
        rt.root.join("governed")
    } else {
        rt.root.clone()
    };
    crate::private_store::probe_metadata(&metadata).map_err(|e| {
        crate::private_store::context(
            e,
            if governed {
                "governed_metadata"
            } else {
                "profile"
            },
        )
    })?;
    Ok(())
}

pub(crate) fn random() -> String {
    let mut b = [0u8; 32];
    OsRng.fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}
fn cancelled() -> Failure {
    Failure::new("LOGIN_CANCELLED", 8, "登录已取消。")
}
pub(crate) async fn callback(listener: &TcpListener, state: &str, origin: &str) -> Result<String> {
    let (mut socket, _) = listener.accept().await.map_err(|_| http::network())?;
    let mut bytes = Vec::new();
    loop {
        let mut b = [0u8; 512];
        let n = socket.read(&mut b).await.map_err(|_| http::network())?;
        if n == 0 || bytes.len() + n > 8192 {
            return Err(invalid());
        }
        bytes.extend_from_slice(&b[..n]);
        if bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    let result = (|| {
        let raw = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
        let first = raw.lines().next().ok_or_else(invalid)?;
        let parts: Vec<_> = first.split(' ').collect();
        if parts.len() != 3 || parts[0] != "GET" || !parts[1].starts_with("/oauth/callback?") {
            return Err(invalid());
        }
        if !["HTTP/1.0", "HTTP/1.1"].contains(&parts[2]) {
            return Err(invalid());
        }
        let headers = raw
            .split("\r\n")
            .skip(1)
            .take_while(|line| !line.is_empty())
            .map(|line| line.split_once(':').ok_or_else(invalid))
            .collect::<Result<Vec<_>>>()?;
        let hosts = headers
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case("host"))
            .collect::<Vec<_>>();
        let expected_host = listener
            .local_addr()
            .map_err(|_| http::network())?
            .to_string();
        if hosts.len() != 1
            || hosts[0].1.trim() != expected_host
            || headers.iter().any(|(key, value)| {
                key.eq_ignore_ascii_case("transfer-encoding")
                    || (key.eq_ignore_ascii_case("content-length") && value.trim() != "0")
            })
        {
            return Err(invalid());
        }
        let url =
            url::Url::parse(&format!("http://127.0.0.1{}", parts[1])).map_err(|_| invalid())?;
        if url.path() != "/oauth/callback" || url.fragment().is_some() {
            return Err(invalid());
        }
        let mut params = BTreeMap::new();
        for (k, v) in url.query_pairs() {
            if !["state", "code", "error", "iss"].contains(&k.as_ref())
                || params.insert(k.to_string(), v.to_string()).is_some()
            {
                return Err(invalid());
            }
        }
        if params.get("state").map(String::as_str) != Some(state)
            || params.get("iss").map(String::as_str) != Some(origin)
        {
            return Err(Failure::new(
                "AUTH_DENIED",
                4,
                "回调状态或授权服务器来源不匹配。",
            ));
        }
        if params.contains_key("error") {
            return Err(Failure::new("AUTH_DENIED", 4, "员工拒绝了本次授权。"));
        }
        params
            .remove("code")
            .filter(|s| !s.is_empty() && s.len() <= 256)
            .ok_or_else(invalid)
    })();
    let accepted = result.is_ok();
    let nonce = accepted.then(random).unwrap_or_default();
    let close_script = if accepted {
        format!(
            "<script nonce=\"{nonce}\">{}</script>",
            include_str!("login/callback-close.js")
        )
    } else {
        String::new()
    };
    let body = callback_page(accepted, &nonce, &close_script);
    let scripts = if accepted {
        format!("script-src 'nonce-{nonce}'")
    } else {
        "script-src 'none'".to_owned()
    };
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; {scripts}; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.write_all(reply.as_bytes()).await;
    result
}
fn callback_page(accepted: bool, nonce: &str, close_script: &str) -> String {
    let (state, icon, title, message, next, detail) = if accepted {
        (
            "accepted",
            "m5 12 4 4 10-10",
            "身份确认已返回",
            "授权响应已收到，此页面将自动关闭。",
            "返回终端查看登录结果",
            "dt-cli 正在验证并保存登录信息。以终端结果为准。如果页面未自动关闭，请手动关闭。",
        )
    } else {
        (
            "error",
            "m6 6 12 12M6 18 18 6",
            "本次登录未完成",
            "授权请求未通过验证，或本次登录已取消。",
            "回到终端，重新发起登录",
            "查看终端中的具体原因后，再执行原登录命令。此页面可以关闭。",
        )
    };
    include_str!("login/callback.html")
        .replace("{{state}}", state)
        .replace("{{icon}}", icon)
        .replace("{{title}}", title)
        .replace("{{message}}", message)
        .replace("{{next}}", next)
        .replace("{{detail}}", detail)
        .replace("{{nonce}}", nonce)
        .replace(
            "{{close_fallback}}",
            if accepted {
                "<p id=\"close-fallback\">如果页面未自动关闭，请返回终端查看登录结果，再手动关闭此页。</p>"
            } else {
                ""
            },
        )
        .replace("{{close_script}}", close_script)
}

async fn recover(
    mut e: Failure,
    client: &reqwest::Client,
    env: &Environment,
    c: &Credentials,
    id: &str,
) -> Failure {
    let revoked = http::revoke(client, env, &c.refresh_token).await;
    e.recovery = Some(Box::new(
        json!({"authorizationId":id,"remoteRevocation":if revoked{"confirmed"}else{"unknown"},"originalProfile":"preserved","recoveryUrl":env.recovery_url}),
    ));
    e
}
pub async fn login(rt: &Runtime, name: &str, environment: Option<&str>) -> Result<Value> {
    login_with_method(rt, name, environment, None).await
}
pub(crate) fn validate_method(method: Option<&str>) -> Result<()> {
    if method.is_some_and(|value| !["password", "dingtalk", "sms"].contains(&value)) {
        return Err(invalid());
    }
    Ok(())
}
pub async fn login_with_method(
    rt: &Runtime,
    name: &str,
    environment: Option<&str>,
    method: Option<&str>,
) -> Result<Value> {
    login_with_interaction(rt, name, environment, method, None).await
}

pub(crate) fn validate_interaction(interaction: Option<&str>) -> Result<()> {
    if interaction.is_some_and(|value| !["terminal", "browser"].contains(&value)) {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) async fn login_with_interaction(
    rt: &Runtime,
    name: &str,
    environment: Option<&str>,
    method: Option<&str>,
    interaction: Option<&str>,
) -> Result<Value> {
    validate_method(method)?;
    validate_interaction(interaction)?;
    if !rt.interactive && interaction != Some("browser") {
        return Err(Failure::new(
            "INTERACTION_REQUIRED",
            8,
            "请由员工在交互终端显式执行登录。",
        ));
    }
    let cancellation = tokio::signal::ctrl_c();
    tokio::pin!(cancellation);
    let _lock = tokio::select! {biased; _=&mut cancellation=>return Err(cancelled()), r=profile::lock(&rt.root,name)=>r?};
    if crate::governed::profile_exists(&rt.root, name)? {
        return Err(Failure::new(
            "PROFILE_PROVIDER_MISMATCH",
            2,
            "此 profile 属于受控授权，不能用于个人流程登录。",
        ));
    }
    let previous = profile::read(&rt.root, name)?;
    let env_name = environment
        .or_else(|| previous.as_ref().map(|p| p.environment.as_str()))
        .ok_or_else(invalid)?;
    if previous.as_ref().is_some_and(|p| p.environment != env_name) {
        return Err(invalid());
    }
    let env = rt.environment(env_name)?;
    preflight(rt, false)?;
    let client = http::client()?;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| http::network())?;
    let redirect = format!(
        "http://127.0.0.1:{}/oauth/callback",
        listener.local_addr().map_err(|_| http::network())?.port()
    );
    let state = random();
    let verifier = zeroize::Zeroizing::new(random());
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = url::Url::parse(&format!(
        "{}/cli/oauth2/authorize",
        env.api_origin.trim_end_matches('/')
    ))
    .map_err(|_| invalid())?;
    url.query_pairs_mut().extend_pairs([
        ("response_type", "code"),
        ("client_id", "dt-cli"),
        ("redirect_uri", &redirect),
        ("state", &state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    if let Some(method) = method {
        url.query_pairs_mut().append_pair("login_method", method);
    }
    rt.browser.open(url.as_str())?;
    progress("waiting_for_browser");
    let code = tokio::select! {r=tokio::time::timeout(Duration::from_secs(900),callback(&listener,&state,&env.api_origin))=>r.map_err(|_|Failure::new("TIMEOUT",5,"等待浏览器确认超时。"))??,_=&mut cancellation=>return Err(cancelled())};
    drop(listener);
    progress("exchanging_code");
    // Do not retry or abandon an in-flight exchange: the server may already have consumed the code.
    let token = http::token(&client, env, &code, &redirect, &verifier).await?;
    let c = Credentials {
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        version: 1,
        access_expires_at: None,
        authorization_expires_at: Some(token.authorization_expires_at.clone()),
        generation: 0,
        refresh_state: "ready".into(),
        refresh_request_id: None,
    };
    let checked =
        tokio::select! {r=http::me(&client,env,&c)=>r,_=&mut cancellation=>Err(cancelled())};
    let mut new_key = None;
    let outcome = (|| {
        let me = checked?;
        let server_time = chrono::DateTime::parse_from_rfc3339(&token.server_time)
            .map_err(|_| http::network())?;
        let deadline = chrono::DateTime::parse_from_rfc3339(&token.authorization_expires_at)
            .map_err(|_| http::network())?;
        if token.token_type != "Bearer"
            || !c.access_token.starts_with("dtcli_a_")
            || !c.refresh_token.starts_with("dtcli_r_")
            || token.expires_in <= 0
            || token.expires_in > 43200
            || server_time + chrono::Duration::seconds(token.expires_in) > deadline
            || token.scope != "iam.profile.read iam.apps.read iam.workflow.read iam.workflow.open"
            || me["authorizationId"] != token.authorization_id
            || me["authorizationExpiresAt"] != token.authorization_expires_at
        {
            return Err(http::network());
        }
        let subject = me["subject"]["id"].as_str().ok_or_else(http::network)?;
        if previous.as_ref().is_some_and(|p| p.subject_id != subject) {
            return Err(Failure::new(
                "IDENTITY_MISMATCH",
                4,
                "浏览器员工与原 profile 不一致；请为另一员工创建新 profile。",
            ));
        }
        let access_expires_at =
            (server_time + chrono::Duration::seconds(token.expires_in)).to_rfc3339();
        let mut stored = c.clone();
        stored.access_expires_at = Some(access_expires_at.clone());
        let p = Profile {
            environment: env_name.to_owned(),
            subject_id: subject.into(),
            authorization_id: token.authorization_id.clone(),
            authorization_expires_at: token.authorization_expires_at,
            access_expires_at,
            checked_at: me["checkedAt"].as_str().unwrap().into(),
            credential_key: format!("{env_name}/{subject}/{}", token.authorization_id),
        };
        if previous
            .as_ref()
            .is_some_and(|old| old.authorization_id == p.authorization_id)
        {
            return Err(http::network());
        }
        progress("saving_local_session");
        new_key = Some(p.credential_key.clone());
        rt.store.write(&p.credential_key, &stored)?;
        // Read back before publishing a profile reference; leave the previous record untouched on failure.
        let saved = rt
            .store
            .read(&p.credential_key)?
            .ok_or_else(crate::output::storage)?;
        if saved.access_token != stored.access_token || saved.refresh_token != stored.refresh_token
        {
            return Err(crate::output::storage());
        }
        profile::save(&rt.root, name, &p)?;
        Ok((me, p))
    })();
    match outcome {
        Err(e) => {
            progress("saving_failed_cleanup");
            let mut failure = recover(e, &client, env, &c, &token.authorization_id).await;
            let cleanup = new_key.as_deref().map(|key| rt.store.delete(key).is_ok());
            if let Some(recovery) = failure.recovery.as_mut() {
                recovery["newCredentialCleanup"] = json!(match cleanup {
                    Some(true) => "confirmed",
                    Some(false) => "unknown",
                    None => "not-created",
                });
            }
            Err(failure)
        }
        Ok((mut me, _p)) => {
            let mut old_status = "not-applicable";
            if let Some(old) = previous {
                match rt.store.read(&old.credential_key) {
                    Ok(Some(old_c)) => {
                        if http::revoke(&client, env, &old_c.refresh_token).await {
                            old_status = if rt.store.delete(&old.credential_key).is_ok() {
                                "confirmed"
                            } else {
                                "confirmed-local-cleanup-failed"
                            };
                        } else {
                            old_status = "unknown";
                        }
                    }
                    _ => old_status = "unknown",
                }
            }
            progress("complete");
            me["previousAuthorizationRevocation"] = json!(old_status);
            Ok(me)
        }
    }
}
pub fn terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

#[cfg(test)]
mod callback_tests {
    use super::*;
    use tokio::net::TcpStream;

    async fn receive(query: &str) -> (Result<String>, String) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let request = format!("GET /oauth/callback?{query} HTTP/1.1\r\nHost: {address}\r\n\r\n");
        let client = async move {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            response
        };
        tokio::join!(
            callback(&listener, "expected-state", "https://iam.fixture.invalid"),
            client
        )
    }

    #[tokio::test]
    async fn accepted_response_is_html_without_echoing_oauth_values() {
        let (result, response) = receive(
            "state=expected-state&iss=https%3A%2F%2Fiam.fixture.invalid&code=synthetic-secret-code",
        )
        .await;
        assert_eq!(result.unwrap(), "synthetic-secret-code");
        assert!(response.contains("Content-Type: text/html; charset=utf-8"));
        assert!(response.contains("Cache-Control: no-store"));
        assert!(response.contains("Content-Security-Policy: default-src 'none'"));
        assert!(response.contains("身份确认已返回"));
        assert!(!response.contains("synthetic-secret-code"));
        assert!(!response.contains("expected-state"));
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        assert!(headers.contains(&format!("Content-Length: {}", body.len())));
    }

    #[tokio::test]
    async fn denied_or_mismatched_response_shows_failure_and_keeps_validation() {
        for query in [
            "state=wrong&iss=https%3A%2F%2Fiam.fixture.invalid&code=synthetic-code",
            "state=expected-state&iss=https%3A%2F%2Fevil.invalid&code=synthetic-code",
            "state=expected-state&iss=https%3A%2F%2Fiam.fixture.invalid&error=access_denied",
            "state=expected-state&state=expected-state&iss=https%3A%2F%2Fiam.fixture.invalid&code=synthetic-code",
        ] {
            let (result, response) = receive(query).await;
            assert!(result.is_err());
            assert!(response.contains("本次登录未完成"));
            assert!(!response.contains("synthetic-code"));
        }
    }
}
