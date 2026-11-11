//! 浏览器授权前验证 CLI 实际网络与匿名平台协议，复用随后换码的客户端。
use super::{model::RemoteError, transport};
use crate::output::{Details, Failure, Result};

pub(super) async fn check(client: &reqwest::Client, origin: &str, namespace: &str) -> Result<()> {
    let reply = transport::receive(client.get(format!("{origin}{namespace}/me"))).await?;
    if reply.status == 401 && reply.json {
        let rejection: RemoteError =
            serde_json::from_slice(&reply.body).map_err(|_| unexpected(reply.status))?;
        if rejection.code.as_deref() == Some("AUTH_REQUIRED") {
            return Ok(());
        }
    }
    Err(unexpected(reply.status))
}

fn unexpected(status: u16) -> Failure {
    let mut failure = Failure::new(
        "PLATFORM_NETWORK_PREFLIGHT_FAILED",
        5,
        "平台匿名网络检查未通过，尚未打开浏览器。",
    );
    failure.hint = "核对当前代理、DNS及平台接口响应，修复后沿原环境登录。";
    failure.details = Some(Box::new(Details {
        reason: Some("unexpected_response".into()),
        http_status: Some(status),
        ..Default::default()
    }));
    failure
}
