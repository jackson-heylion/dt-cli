//! HTTP 接收层拆分后保持状态映射与 GET／POST 重发边界。
use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test]
async fn http_errors_preserve_status_and_only_safe_reads_can_be_retried() {
    for (status, code, expected, read_retryable) in [
        (401, "AUTH_REQUIRED", "AUTH_REQUIRED", false),
        (
            403,
            "INTERFACE_PERMISSION_REQUIRED",
            "INTERFACE_PERMISSION_REQUIRED",
            false,
        ),
        (400, "INVALID_ARGUMENT", "INVALID_ARGUMENT", false),
        (429, "RATE_LIMITED", "RATE_LIMITED", true),
        (502, "UNRECOGNIZED", "NETWORK_ERROR", true),
        (
            503,
            "DEPENDENCY_UNAVAILABLE",
            "DEPENDENCY_UNAVAILABLE",
            true,
        ),
    ] {
        for method in [reqwest::Method::GET, reqwest::Method::POST] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                socket.read(&mut request).await.unwrap();
                let body = format!("{{\"code\":\"{code}\",\"message\":\"fixture-secret-body\"}}");
                let response = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.unwrap();
            });
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let failure =
                empty(client.request(method.clone(), format!("http://{address}/fixture")))
                    .await
                    .unwrap_err();
            server.await.unwrap();
            assert_eq!(failure.code, expected);
            assert_eq!(failure.details.as_ref().unwrap().http_status, Some(status));
            assert_eq!(
                failure.retryable,
                method == reqwest::Method::GET && read_retryable
            );
            assert!(
                !serde_json::to_string(&failure)
                    .unwrap()
                    .contains("fixture-secret-body")
            );
        }
    }
}

#[tokio::test]
async fn connection_diagnostics_keep_safe_read_retryability_and_never_allow_post_replay() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    for method in [reqwest::Method::GET, reqwest::Method::POST] {
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let failure = empty(client.request(method.clone(), format!("http://{address}/fixture")))
            .await
            .unwrap_err();
        assert_eq!(failure.code, "NETWORK_ERROR");
        assert_eq!(
            failure.details.as_ref().unwrap().reason.as_deref(),
            Some("connection_failed")
        );
        assert_eq!(failure.retryable, method == reqwest::Method::GET);
    }
}
