use dt_cli::{credentials::Credentials, http, profile::Environment};
use serde_json::json;

fn endpoint(status: u16, body: String) -> Environment {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let mut data = Vec::new();
        let mut chunk = [0; 1024];
        loop {
            let n = stream.read(&mut chunk).unwrap();
            data.extend_from_slice(&chunk[..n]);
            if data.windows(4).any(|x| x == b"\r\n\r\n") {
                break;
            }
        }
        let headers = String::from_utf8_lossy(&data);
        let length = headers
            .lines()
            .find_map(|l| {
                l.to_lowercase()
                    .strip_prefix("content-length:")
                    .map(|n| n.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        let head_end = data.windows(4).position(|x| x == b"\r\n\r\n").unwrap() + 4;
        while data.len() < head_end + length {
            let n = stream.read(&mut chunk).unwrap();
            data.extend_from_slice(&chunk[..n]);
        }
        let response = format!(
            "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).unwrap();
    });
    Environment {
        api_origin: origin.clone(),
        portal_origin: origin.clone(),
        recovery_url: origin,
        launch_path: "/cli/launch".into(),
    }
}
#[tokio::test]
async fn http_failures_preserve_error_categories_and_trace() {
    let credentials = Credentials {
        access_token: "synthetic-access".into(),
        refresh_token: "synthetic-refresh".into(),
        version: 1,
        access_expires_at: None,
        authorization_expires_at: None,
        generation: 0,
        refresh_state: "ready".into(),
        refresh_request_id: None,
    };
    for (status, expected) in [(200, 5), (401, 3), (403, 4), (503, 5), (500, 1)] {
        let env = endpoint(
            status,
            json!({"ok":false,"error":{"code":"AUTH_REQUIRED"},"meta":{"traceId":"01234567-abcd"}})
                .to_string(),
        );
        let failure = http::me(&http::client().unwrap(), &env, &credentials)
            .await
            .unwrap_err();
        assert_eq!(failure.exit, expected);
        assert_eq!(failure.trace_id.as_deref(), Some("01234567-abcd"));
    }
}
#[tokio::test]
async fn oauth_dependency_failure_is_not_a_user_denial() {
    let env = endpoint(503, json!({"error":"temporarily_unavailable"}).to_string());
    let result = http::token(
        &http::client().unwrap(),
        &env,
        "synthetic-code",
        "http://127.0.0.1:12345/oauth/callback",
        "synthetic-verifier",
    )
    .await;
    assert!(matches!(result,Err(e) if e.exit==5 && e.code=="DEPENDENCY_UNAVAILABLE"));
}
#[tokio::test]
async fn revocation_requires_completed_http_success() {
    for (status, confirmed) in [(200, true), (202, false), (503, false)] {
        let env = endpoint(status, "{}".into());
        assert_eq!(
            http::revoke(&http::client().unwrap(), &env, "synthetic-refresh").await,
            confirmed
        );
    }
}

#[tokio::test]
async fn refresh_only_explicit_rejections_allow_a_known_outcome() {
    for status in [202, 302, 429, 500, 502, 503] {
        let env = endpoint(status, "{}".into());
        let failure = http::refresh(&http::client().unwrap(), &env, "synthetic-refresh")
            .await
            .err()
            .unwrap();
        assert_eq!(failure.code, "AUTH_REFRESH_OUTCOME_UNKNOWN", "{status}");
        assert!(!failure.retryable);
    }
    for status in [400, 401, 403] {
        let env = endpoint(status, "{}".into());
        let failure = http::refresh(&http::client().unwrap(), &env, "synthetic-refresh")
            .await
            .err()
            .unwrap();
        assert_eq!(failure.code, "AUTH_REQUIRED", "{status}");
    }
}

#[derive(Clone)]
struct Store(std::sync::Arc<std::sync::Mutex<Credentials>>);
impl dt_cli::credentials::CredentialStore for Store {
    fn read(&self, _: &str) -> dt_cli::output::Result<Option<Credentials>> {
        Ok(Some(self.0.lock().unwrap().clone()))
    }
    fn write(&self, _: &str, c: &Credentials) -> dt_cli::output::Result<()> {
        *self.0.lock().unwrap() = c.clone();
        Ok(())
    }
    fn delete(&self, _: &str) -> dt_cli::output::Result<()> {
        panic!("unexpected delete")
    }
}
fn refresh_fixture() -> (
    tempfile::TempDir,
    dt_cli::Runtime,
    Store,
    dt_cli::profile::Profile,
    serde_json::Value,
) {
    let root = tempfile::tempdir().unwrap();
    let now = chrono::Utc::now();
    let p = dt_cli::profile::Profile {
        environment: "fixture".into(),
        subject_id: "1".into(),
        authorization_id: "auth-fixture".into(),
        authorization_expires_at: (now + chrono::Duration::days(7)).to_rfc3339(),
        access_expires_at: (now - chrono::Duration::seconds(1)).to_rfc3339(),
        checked_at: now.to_rfc3339(),
        credential_key: "fixture/1/auth-fixture".into(),
    };
    let store = Store(std::sync::Arc::new(std::sync::Mutex::new(Credentials {
        access_token: "dtcli_a_old".into(),
        refresh_token: "dtcli_r_old".into(),
        version: 1,
        access_expires_at: Some(p.access_expires_at.clone()),
        authorization_expires_at: Some(p.authorization_expires_at.clone()),
        generation: 0,
        refresh_state: "ready".into(),
        refresh_request_id: None,
    })));
    let rt = dt_cli::Runtime {
        root: root.path().into(),
        environments: Default::default(),
        store: Box::new(store.clone()),
        browser: Box::new(dt_cli::login::SystemBrowser),
        interactive: false,
        aggregate_budget: std::time::Duration::from_secs(30),
    };
    let token = json!({"access_token":"dtcli_a_new","refresh_token":"dtcli_r_new",
        "token_type":"Bearer","expires_in":3600,
        "scope":"iam.profile.read iam.apps.read iam.workflow.read iam.workflow.open",
        "authorization_id":p.authorization_id,"authorization_expires_at":p.authorization_expires_at,
        "server_time":now.to_rfc3339()});
    (root, rt, store, p, token)
}

#[tokio::test]
async fn simultaneous_expired_readers_share_one_refresh_generation() {
    let (_root, rt, store, p, token) = refresh_fixture();
    // The server accepts exactly one connection: a second refresh would fail this test.
    let env = endpoint(200, token.to_string());
    let (first, second) = tokio::join!(rt.authorize(&p, &env), rt.authorize(&p, &env));
    assert_eq!(first.unwrap().generation, 1);
    assert_eq!(second.unwrap().generation, 1);
    assert_eq!(store.0.lock().unwrap().generation, 1);
}

#[tokio::test]
async fn invalid_or_uncertain_refresh_preserves_pending_without_advancing_generation() {
    for (status, field, value) in [
        (503, "expires_in", json!(3600)),
        (200, "expires_in", json!(i64::MAX)),
        (200, "access_token", json!("wrong-provider")),
        (200, "token_type", json!("Other")),
        (
            200,
            "authorization_expires_at",
            json!("2099-01-01T00:00:00Z"),
        ),
    ] {
        let (_root, rt, store, p, mut token) = refresh_fixture();
        token[field] = value;
        let env = endpoint(status, token.to_string());
        assert!(rt.authorize(&p, &env).await.is_err());
        assert_eq!(store.0.lock().unwrap().refresh_state, "pending");
        let failure = rt.authorize(&p, &env).await.err().unwrap();
        assert_eq!(failure.code, "AUTH_REFRESH_OUTCOME_UNKNOWN");
        assert_eq!(store.0.lock().unwrap().generation, 0);
    }
}

#[tokio::test]
async fn refresh_and_identity_query_reuse_the_same_connection() {
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let (_root, mut rt, _store, p, token) = refresh_fixture();
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let connections = std::sync::Arc::new(AtomicUsize::new(0));
    let count = connections.clone();
    let me = json!({"schemaVersion":"1.0","operationId":"iam.profile.get","ok":true,
        "error":null,"meta":{},"data":{"subject":{"id":"1","account":"fixture","displayName":"Fixture"},
        "authorizationId":p.authorization_id,"authorizationExpiresAt":p.authorization_expires_at,
        "allowedOperations":["iam.profile.get"],"checkedAt":p.checked_at}});
    let server = std::thread::spawn(move || {
        let replies = [token.to_string(), me.to_string()];
        let mut index = 0;
        while index < replies.len() {
            let (mut stream, _) = listener.accept().unwrap();
            count.fetch_add(1, Ordering::SeqCst);
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            loop {
                let mut request = Vec::new();
                let mut chunk = [0; 4096];
                let header_end = loop {
                    let n = stream.read(&mut chunk).unwrap();
                    if n == 0 {
                        break None;
                    }
                    request.extend_from_slice(&chunk[..n]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(end + 4);
                    }
                };
                let Some(header_end) = header_end else { break };
                let head = String::from_utf8_lossy(&request[..header_end]).to_lowercase();
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("content-length:")
                            .map(|v| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while request.len() < header_end + length {
                    let n = stream.read(&mut chunk).unwrap();
                    assert_ne!(n, 0);
                    request.extend_from_slice(&chunk[..n]);
                }
                if index == 0 {
                    assert!(head.starts_with("post /cli/oauth2/token "));
                } else {
                    assert!(head.starts_with("get /cli/v1/me "));
                    assert!(head.contains("authorization: bearer dtcli_a_new"));
                }
                let body = &replies[index];
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len()).unwrap();
                index += 1;
                if index == replies.len() {
                    return;
                }
            }
        }
    });
    rt.environments.insert(
        "fixture".into(),
        Environment {
            api_origin: origin.clone(),
            portal_origin: origin.clone(),
            recovery_url: origin,
            launch_path: "/cli/launch".into(),
        },
    );
    dt_cli::profile::save(&rt.root, "p", &p).unwrap();
    let (value, exit, _) = dt_cli::execute(
        &rt,
        ["dt-cli", "whoami", "--profile", "p"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
    )
    .await;
    assert_eq!(exit, 0, "{value}");
    server.join().unwrap();
    assert_eq!(connections.load(Ordering::SeqCst), 1);
}
