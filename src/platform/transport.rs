use super::model::RemoteError;
use crate::output::{Failure, Result, invalid};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

pub(super) fn challenge(value: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(value.as_bytes()))
}

pub(super) fn origin(value: &str) -> Result<()> {
    // 先核对原始 origin，避免 URL 规范化抹去空用户信息、路径或两侧空白。
    let authority = value
        .split_once("://")
        .map(|(_, rest)| rest)
        .ok_or_else(invalid)?;
    if value.chars().any(|c| c.is_whitespace() || c.is_control())
        || authority.contains(['/', '\\', '@', '?', '#'])
    {
        return Err(invalid());
    }
    let parsed = url::Url::parse(value).map_err(|_| invalid())?;
    let local = (cfg!(test) || cfg!(feature = "local-dev"))
        && parsed.scheme() == "http"
        && parsed.host_str() == Some("127.0.0.1");
    if (parsed.scheme() != "https" && !local)
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
        || value.ends_with('/')
    {
        return Err(invalid());
    }
    Ok(())
}

async fn response(request: reqwest::RequestBuilder) -> Result<Vec<u8>> {
    let (client, built) = request.build_split();
    let built = built.map_err(|_| crate::http::protocol())?;
    let safe_read =
        built.method() == reqwest::Method::GET || built.method() == reqwest::Method::HEAD;
    let mut response = client.execute(built).await.map_err(|_| {
        if safe_read {
            super::errors::read_retry(network())
        } else {
            network()
        }
    })?;
    let status = response.status();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| network())? {
        if bytes.len() + chunk.len() > 2097152 {
            return Err(crate::http::protocol());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        let error = serde_json::from_slice::<RemoteError>(&bytes).ok();
        let mut failure = match (
            status.as_u16(),
            error.as_ref().and_then(|e| e.code.as_deref()),
        ) {
            (401, _) => Failure::new("AUTH_REQUIRED", 4, "平台会话无效或已撤销，请重新登录。"),
            (403, Some("INTERFACE_PERMISSION_REQUIRED")) => Failure::new(
                "INTERFACE_PERMISSION_REQUIRED",
                4,
                "当前接口缺少执行许可，请联系平台管理员在接口工作区核对内部授权。",
            ),
            (403, _) => Failure::new("AUTH_DENIED", 4, "身份、应用准入或会话权限已被拒绝。"),
            (404, _) => Failure::new(
                "RESOURCE_UNAVAILABLE",
                6,
                "本人会话不存在或当前服务尚未提供该能力。",
            ),
            (429, _) => Failure::new("RATE_LIMITED", 5, "请求过于频繁，请稍后重试。"),
            (503, _) => Failure::new(
                "DEPENDENCY_UNAVAILABLE",
                5,
                "平台身份依赖暂不可用，请稍后重试。",
            ),
            (400, _) => Failure::new(
                "INVALID_ARGUMENT",
                2,
                "平台输入格式或参数无效，请核对固定协议。",
            ),
            _ => network(),
        };
        let fields = super::errors::details(
            status.as_u16(),
            error.as_ref().and_then(|e| e.code.as_deref()),
            error.as_ref().and_then(|e| e.stage.as_deref()),
            error.as_ref().and_then(|e| e.trace_id.as_deref()),
        );
        failure.trace_id = fields.trace_id.clone();
        failure.details = Some(fields);
        return Err(if safe_read {
            super::errors::read_retry(failure)
        } else {
            failure
        });
    }
    Ok(bytes)
}

pub(super) async fn request<T: DeserializeOwned>(request: reqwest::RequestBuilder) -> Result<T> {
    serde_json::from_slice(&response(request).await?).map_err(|_| crate::http::protocol())
}
pub(super) async fn empty(request: reqwest::RequestBuilder) -> Result<()> {
    response(request).await.map(|_| ())
}

pub(super) fn network() -> Failure {
    Failure::new(
        "NETWORK_ERROR",
        5,
        "网络请求未完成，请保留当前记录；写入只核对原意图结果。",
    )
}

/// 同进程共享连接池，认证头始终由每个请求单独设置。
pub(super) fn client() -> Result<reqwest::Client> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = crate::http::client()?;
    let _ = CLIENT.set(client.clone());
    Ok(CLIENT.get().cloned().unwrap_or(client))
}
