//! 通过原生命令与隔离 HTTP 核对分项退出及旧资料隔离。
#![cfg(feature = "local-dev")]
use dt_cli::credentials::{CredentialStore, Credentials, FileStore};
use serde_json::{Value, json};
use std::{path::Path, process::Command};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
fn private(path: &Path, value: &Value) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    privacy::protect(path.parent().unwrap());
    privacy::protect(&path);
}
fn setup(root: &Path, origin: &str, purpose: &str, name: &str, session: char) -> String {
    let directory = if purpose == "CLI" {
        "platform"
    } else {
        "platform-management"
    };
    let prefix = if purpose == "CLI" {
        "dtp_cli"
    } else {
        "dtp_mgmt"
    };
    let expires = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use sha2::{Digest, Sha256};
    let key = format!(
        "{directory}:{}:{name}:{}",
        URL_SAFE_NO_PAD.encode(Sha256::digest(origin.as_bytes())),
        session.to_string().repeat(43)
    );
    private(
        &root.join(directory).join(format!("{name}.json")),
        &json!({"purpose":purpose,"schemaVersion":1,"origin":origin,"environment":"fixture","systemId":"platform","subjectId":"1024","domain":"EMPLOYEE","sessionId":session.to_string().repeat(43),"expiresAt":expires,"credentialKey":key}),
    );
    FileStore::new(root)
        .write(
            &key,
            &Credentials {
                access_token: format!("{prefix}_at_{}", "A".repeat(43)),
                refresh_token: format!("{prefix}_rt_{}", "R".repeat(43)),
                version: 1,
                access_expires_at: Some(expires.clone()),
                authorization_expires_at: Some(expires),
                generation: 0,
                refresh_state: "ready".into(),
                refresh_request_id: None,
            },
        )
        .unwrap();
    key
}
fn run(root: &Path, args: &[&str]) -> (Value, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_dt-cli"))
        .args(args)
        .env("DT_CLI_LOCAL_TEST_CONFIG", root)
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains("dtp_cli_rt_"));
    let value = serde_json::from_slice(&out.stdout).unwrap_or_else(|error| {
        panic!(
            "原生命令 {args:?} 未输出 JSON: {error}; status={}; stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        )
    });
    (value, out.status.code().unwrap())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logout_cleans_local_after_remote_failure_and_preserves_other_purpose() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let key = setup(root.path(), &origin, "CLI", "employee", 'S');
    let other = setup(root.path(), &origin, "MANAGEMENT", "employee", 'M');
    let old = root.path().join("governed/history.json");
    private(&old, &json!({"historical":"evidence"}));
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0u8; 8192];
        let size = socket.read(&mut bytes).await.unwrap();
        assert!(
            String::from_utf8_lossy(&bytes[..size])
                .starts_with("POST /platform/cli/oauth2/revoke ")
        );
        let body = r#"{"code":"DEPENDENCY_UNAVAILABLE","stage":"AUTHENTICATION","traceId":"trace-logout"}"#;
        socket.write_all(format!("HTTP/1.1 503 Unavailable\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
    });
    let (result, code) = run(root.path(), &["logout", "--profile", "employee"]);
    server.await.unwrap();
    assert_eq!(code, 7, "{result}");
    assert_eq!(result["error"]["code"], "LOGOUT_PARTIAL");
    assert_eq!(result["data"]["localCleanup"], "confirmed");
    assert_eq!(result["data"]["remoteRevocation"], "unknown");
    assert!(FileStore::new(root.path()).read(&key).unwrap().is_none());
    assert!(FileStore::new(root.path()).read(&other).unwrap().is_some());
    assert!(old.exists());
    let (again, code) = run(root.path(), &["logout", "--profile", "employee"]);
    assert_eq!(code, 0, "{again}");
    assert_eq!(again["data"]["localCleanup"], "not-present");
}

#[path = "platform_outcomes/writes.rs"]
mod writes;

#[path = "common/privacy.rs"]
mod privacy;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn management_failures_keep_bounded_diagnostics_and_original_purpose() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    setup(root.path(), &origin, "MANAGEMENT", "manager", 'M');
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut input = [0; 8192];
        let size = socket.read(&mut input).await.unwrap();
        assert!(
            String::from_utf8_lossy(&input[..size])
                .starts_with("GET /platform/management/systems?")
        );
        let body = json!({"code":"SOURCE_UNAVAILABLE","message":"dtp_cli_rt_sensitive-secret","stage":"MANAGEMENT","traceId":"trace-management","planId":"dtp_cli_at_sensitive-secret","requestId":"request-001"});
        let body = body.to_string();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 503 Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
    });
    let (result, code) = run(root.path(), &["manage", "systems", "--profile", "manager"]);
    server.await.unwrap();
    assert_eq!(code, 5, "{result}");
    assert_eq!(result["error"]["category"], "dependency");
    assert_eq!(result["error"]["retryable"], false);
    assert_eq!(result["error"]["details"]["code"], "SOURCE_UNAVAILABLE");
    assert_eq!(result["error"]["details"]["httpStatus"], 503);
    assert_eq!(result["error"]["details"]["traceId"], "trace-management");
    assert!(!result.to_string().contains("sensitive-secret"));
    assert_eq!(result["meta"]["recovery"]["purpose"], "management");
    assert_eq!(result["meta"]["recovery"]["profile"], "manager");
}

#[path = "platform_outcomes/jobs.rs"]
mod jobs;
