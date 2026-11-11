//! 本机 HTTP／代理／解析夹具验证错误分类与脱敏；不连接外部域名。
use super::*;
use std::{io, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct FailedDns;
impl reqwest::dns::Resolve for FailedDns {
    fn resolve(&self, _: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async {
            Err(Box::new(io::Error::new(
                io::ErrorKind::NotFound,
                "fixture-dns-secret",
            )) as Box<dyn std::error::Error + Send + Sync>)
        })
    }
}

fn assert_reason(error: reqwest::Error, expected: &str) {
    let failure = failure(&error, crate::http::network());
    assert_eq!(failure.code, "NETWORK_ERROR");
    assert_eq!(
        failure.details.as_ref().unwrap().reason.as_deref(),
        Some(expected)
    );
    let output = serde_json::to_string(&failure).unwrap();
    for hidden in [
        "fixture-password",
        "fixture-user",
        "fixture-code",
        "fixture-dns-secret",
        "fixture.invalid",
    ] {
        assert!(!output.contains(hidden));
    }
}

#[tokio::test]
async fn dns_failure_is_identified_without_exposing_the_request() {
    let error = reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(FailedDns))
        .build()
        .unwrap()
        .post("https://platform.fixture.invalid/token?code=fixture-code")
        .bearer_auth("fixture-password")
        .send()
        .await
        .unwrap_err();
    assert_reason(error, "dns_resolution_failed");
}

#[tokio::test]
async fn proxy_connect_rejection_is_distinct_from_platform_http_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 4096];
        let length = socket.read(&mut request).await.unwrap();
        assert!(request[..length].starts_with(b"CONNECT platform.fixture.invalid:443 HTTP/1.1"));
        socket
            .write_all(
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await
            .unwrap();
    });
    let error = reqwest::Client::builder()
        .proxy(
            reqwest::Proxy::all(proxy)
                .unwrap()
                .basic_auth("fixture-user", "fixture-password"),
        )
        .build()
        .unwrap()
        .post("https://platform.fixture.invalid/token?code=fixture-code")
        .send()
        .await
        .unwrap_err();
    server.await.unwrap();
    assert_reason(error, "proxy_connect_failed");
}

#[tokio::test]
async fn request_timeout_remains_distinct_from_connection_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let error = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(50))
        .build()
        .unwrap()
        .get(format!("http://{address}/me"))
        .send()
        .await
        .unwrap_err();
    server.abort();
    assert_reason(error, "request_timeout");
}

#[tokio::test]
async fn corrupt_tls_handshake_is_rejected_and_classified() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut hello = [0; 4096];
        socket.read(&mut hello).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let error = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("https://{address}/me"))
        .send()
        .await
        .unwrap_err();
    server.await.unwrap();
    assert_reason(error, "tls_failed");
}
