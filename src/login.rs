//! 本机安全回调与浏览器能力；平台登录流程位于 platform。
use crate::{
    constants::identity::{CALLBACK_BUFFER_BYTES, CALLBACK_REQUEST_BYTES, CALLBACK_SOCKET_SECONDS},
    http,
    output::{Failure, Result, invalid},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::{RngCore, rngs::OsRng};
use std::{collections::BTreeMap, io::IsTerminal, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};
pub(crate) mod browser;
pub(crate) use browser::progress;
pub use browser::{Browser, SystemBrowser};
pub(crate) fn random() -> String {
    let mut b = [0u8; crate::constants::identity::RANDOM_BYTES];
    OsRng.fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}
async fn read_callback_request(socket: &mut TcpStream) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; CALLBACK_BUFFER_BYTES];
    loop {
        let size = socket.read(&mut buffer).await.ok()?;
        if size == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..size]);
        if bytes.len() > CALLBACK_REQUEST_BYTES || bytes.windows(4).any(|w| w == b"\r\n\r\n") {
            return Some(bytes);
        }
    }
}

fn targets_callback(bytes: &[u8]) -> bool {
    let line = bytes
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or_default();
    let target = line
        .split(u8::is_ascii_whitespace)
        .filter(|part| !part.is_empty())
        .nth(1);
    target.is_some_and(|target| {
        target.split(|byte| matches!(*byte, b'?' | b'#')).next() == Some(b"/oauth/callback")
    })
}

async fn callback_observed(
    listener: &TcpListener,
    state: &str,
    origin: &str,
    login: &browser::BrowserLogin,
) -> Result<String> {
    let socket_budget = Duration::from_secs(CALLBACK_SOCKET_SECONDS);
    let (mut socket, bytes) = loop {
        let (mut socket, _) = listener.accept().await.map_err(|_| http::network())?;
        login.callback_connected();
        let Ok(Some(bytes)) = timeout(socket_budget, read_callback_request(&mut socket)).await
        else {
            continue;
        };
        if targets_callback(&bytes) {
            break (socket, bytes);
        }
    };
    if bytes.len() > CALLBACK_REQUEST_BYTES {
        return Err(invalid());
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
            .filter(|s| {
                !s.is_empty() && s.len() <= crate::constants::identity::CALLBACK_ERROR_LENGTH
            })
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
    let body = callback_page(accepted, &close_script);
    let scripts = if accepted {
        format!("script-src 'nonce-{nonce}'")
    } else {
        "script-src 'none'".to_owned()
    };
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; {scripts}; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\nX-Content-Type-Options: nosniff\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    let _ = timeout(socket_budget, socket.write_all(reply.as_bytes())).await;
    result
}
fn callback_page(accepted: bool, close_script: &str) -> String {
    let (state, icon, title, message, next, detail) = if accepted {
        (
            "accepted",
            "m5 12 4 4 10-10",
            "身份确认已返回",
            "授权响应已收到，终端将继续完成登录。",
            "返回终端查看登录结果",
            "dt-cli 正在验证并保存登录信息，以终端结果为准。浏览器不允许网页关闭由命令打开的标签页，请按 ⌘W 或 Ctrl+W 关闭本页。",
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
        .replace("{{close_script}}", close_script)
}

pub fn terminal() -> bool {
    std::io::stdin().is_terminal()
}

#[cfg(test)]
mod tests;
