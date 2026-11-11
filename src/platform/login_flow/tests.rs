//! 隔离 HTTP 验证完整登录边界；不打开真实浏览器或访问正式／测试平台。
use super::*;
use crate::{
    credentials::{CredentialStore, FileStore},
    login::Browser,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
mod fixture;
use fixture::*;

fn browser(callback: bool, fail: bool, delay_seconds: u64) -> BrowserProbe {
    BrowserProbe {
        calls: Arc::new(AtomicUsize::new(0)),
        address: Arc::new(Mutex::new(None)),
        fail,
        callback,
        delay_seconds,
        pages: Arc::new(Mutex::new(vec![])),
    }
}
fn runtime(root: &std::path::Path, browser: BrowserProbe, store_failure: bool) -> Runtime {
    let store: Box<dyn CredentialStore> = if store_failure {
        Box::new(WriteFailureStore {
            inner: FileStore::new(root),
        })
    } else {
        Box::new(FileStore::new(root))
    };
    Runtime {
        root: root.into(),
        store,
        browser: Box::new(browser),
        interactive: false,
        aggregate_budget: Duration::from_secs(30),
    }
}
fn assert_empty(root: &std::path::Path, purpose: SessionPurpose) {
    for directory in [purpose.directory(), "credentials"] {
        let path = root.join(directory);
        if path.exists() {
            assert_eq!(std::fs::read_dir(path).unwrap().count(), 0);
        }
    }
}

#[tokio::test]
async fn network_or_protocol_failure_stops_before_browser_and_preserves_storage() {
    for purpose in [SessionPurpose::Cli, SessionPurpose::Management] {
        for (status, kind, body) in [
            (502, "text/html", "proxy-secret-body"),
            (200, "text/html", "iAdmin"),
            (401, "text/html", "AUTH_REQUIRED"),
            (401, "application/json", "{\"code\":\"AUTH_DENIED\"}"),
            (401, "application/json", "invalid-json-secret"),
            (303, "application/json", "{\"code\":\"AUTH_REQUIRED\"}"),
        ] {
            let server = Server::start(
                Scenario {
                    preflight_status: status,
                    preflight_type: kind,
                    preflight_body: body,
                    ..Default::default()
                },
                purpose,
            )
            .await;
            let endpoint = endpoints::resolve("fixture", Some(&server.origin)).unwrap();
            let root = tempfile::tempdir().unwrap();
            let probe = browser(false, false, 0);
            let calls = probe.calls.clone();
            let rt = runtime(root.path(), probe, false);
            let error = login_with_client(
                &rt,
                "preflight",
                &endpoint,
                "platform",
                false,
                purpose,
                &local_client(),
            )
            .await
            .err()
            .unwrap();
            assert_eq!(error.code, "PLATFORM_NETWORK_PREFLIGHT_FAILED");
            let details = error.details.as_ref().unwrap();
            assert_eq!(details.login_stage.as_deref(), Some("checking_network"));
            assert_eq!(details.http_status, Some(status));
            assert_eq!(details.reason.as_deref(), Some("unexpected_response"));
            assert_eq!(details.process_id, Some(std::process::id()));
            assert!(!error.retryable);
            assert!(!serde_json::to_string(&error).unwrap().contains(body));
            assert_eq!(calls.load(Ordering::Relaxed), 0);
            let requests = server.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].method, "GET");
            assert!(!requests[0].authenticated);
            assert_empty(root.path(), purpose);
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        let endpoint = endpoints::resolve("fixture", Some(&origin)).unwrap();
        let root = tempfile::tempdir().unwrap();
        let probe = browser(false, false, 0);
        let calls = probe.calls.clone();
        let rt = runtime(root.path(), probe, false);
        let error = login_with_client(
            &rt,
            "offline",
            &endpoint,
            "platform",
            false,
            purpose,
            &local_client(),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(error.code, "NETWORK_ERROR");
        assert_eq!(
            error.details.unwrap().reason.as_deref(),
            Some("connection_failed")
        );
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert_empty(root.path(), purpose);
    }
}

#[tokio::test(start_paused = true)]
async fn callback_timeout_keeps_launch_evidence_and_releases_port_and_profile_lock() {
    for purpose in [SessionPurpose::Cli, SessionPurpose::Management] {
        for (fail, disabled, expected_launch, expected_calls) in [
            (false, false, "requested", 1),
            (true, false, "failed", 1),
            (false, true, "disabled", 0),
        ] {
            let server = Server::start(Default::default(), purpose).await;
            let endpoint = endpoints::resolve("fixture", Some(&server.origin)).unwrap();
            let root = tempfile::tempdir().unwrap();
            let probe = browser(false, fail, 0);
            let calls = probe.calls.clone();
            let rt = runtime(root.path(), probe, false);
            let error = login_with_client(
                &rt,
                "timeout",
                &endpoint,
                "platform",
                disabled,
                purpose,
                &local_client(),
            )
            .await
            .err()
            .unwrap();
            assert_eq!(error.code, "TIMEOUT");
            assert!(!error.retryable);
            let details = error.details.as_ref().unwrap();
            assert_eq!(details.browser_launch.as_deref(), Some(expected_launch));
            assert_eq!(details.wait_seconds, Some(1500));
            assert_eq!(details.poll_max_seconds, Some(300));
            assert_eq!(details.command_timeout_seconds, Some(1800));
            assert_eq!(details.process_id, Some(std::process::id()));
            assert_eq!(details.callback_status.as_deref(), Some("waiting"));
            assert_eq!(calls.load(Ordering::Relaxed), expected_calls);
            let listener =
                std::net::TcpListener::bind(details.callback_address.as_ref().unwrap()).unwrap();
            drop(listener);
            let lock = profile::lock(root.path(), &format!("{}-timeout", purpose.directory()))
                .await
                .unwrap();
            drop(lock);
            assert_empty(root.path(), purpose);
            let serialized = serde_json::to_string(&error).unwrap();
            for hidden in [
                server.origin.as_str(),
                "state=",
                "code_verifier",
                "redirect_uri",
            ] {
                assert!(!serialized.contains(hidden));
            }
        }
    }
}

#[tokio::test]
async fn callback_success_does_not_hide_exchange_session_or_storage_failures() {
    for purpose in [SessionPurpose::Cli, SessionPurpose::Management] {
        for (token_status, session_status, revoked, store_failure, expected_stage) in [
            (502, 200, false, false, "exchanging_code"),
            (200, 503, false, false, "checking_session"),
            (200, 200, true, false, "checking_session"),
            (200, 200, false, true, "saving_local_session"),
        ] {
            let server = Server::start(
                Scenario {
                    token_status,
                    session_status,
                    revoked,
                    ..Default::default()
                },
                purpose,
            )
            .await;
            let endpoint = endpoints::resolve("fixture", Some(&server.origin)).unwrap();
            let root = tempfile::tempdir().unwrap();
            let probe = browser(true, false, 0);
            let pages = probe.pages.clone();
            let rt = runtime(root.path(), probe, store_failure);
            let result = login_with_client(
                &rt,
                "failure",
                &endpoint,
                "platform",
                false,
                purpose,
                &local_client(),
            )
            .await;
            let (envelope, exit) = crate::output::envelope_result(
                "login",
                Some("failure"),
                result.and_then(super::super::serialize),
            );
            assert_ne!(exit, 0);
            assert_eq!(envelope["ok"], false);
            assert_eq!(envelope["error"]["details"]["loginStage"], expected_stage);
            assert_eq!(envelope["error"]["details"]["callbackStatus"], "validated");
            if token_status == 502 {
                assert_eq!(
                    envelope["error"]["details"]["authorizationCodeStatus"],
                    "unknown"
                );
                assert_eq!(envelope["error"]["details"]["httpStatus"], 502);
            }
            assert_eq!(envelope["error"]["retryable"], false);
            let requests = server.requests.lock().unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.path.ends_with("/oauth2/token"))
                    .count(),
                1
            );
            assert!(
                requests
                    .iter()
                    .find(|r| r.path.ends_with("/oauth2/token"))
                    .unwrap()
                    .valid_exchange
            );
            assert!(
                pages
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|p| p.contains("身份确认已返回"))
            );
            assert_empty(root.path(), purpose);
            for hidden in [
                "fixture-code",
                "dtp_cli_at_",
                "dtp_mgmt_at_",
                "code_verifier",
            ] {
                assert!(!envelope.to_string().contains(hidden));
            }
        }
    }
}

#[tokio::test]
async fn login_survives_old_deadline_and_only_succeeds_after_credentials_are_saved() {
    for purpose in [SessionPurpose::Cli, SessionPurpose::Management] {
        let server = Server::start(Default::default(), purpose).await;
        let endpoint = endpoints::resolve("fixture", Some(&server.origin)).unwrap();
        let root = tempfile::tempdir().unwrap();
        let probe = browser(true, false, 601);
        let calls = probe.calls.clone();
        let address = probe.address.clone();
        let rt = runtime(root.path(), probe, false);
        let client = local_client();
        tokio::time::pause();
        let future = login_with_client(
            &rt, "success", &endpoint, "platform", false, purpose, &client,
        );
        tokio::pin!(future);
        tokio::select! {
            result = &mut future => panic!("旧期限前就结束：{}", result.err().map(|e| e.code).unwrap_or("success")),
            _ = async { while calls.load(Ordering::Relaxed) == 0 { tokio::task::yield_now().await; } } => {}
        }
        tokio::select! {
            _ = &mut future => panic!("300秒进度读取后登录进程提前结束"),
            _ = tokio::time::sleep(Duration::from_secs(301)) => {}
        }
        assert!(std::net::TcpListener::bind(address.lock().unwrap().unwrap()).is_err());
        let result = future.await.unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(result.purpose, purpose.text());
        let stored = read_profile_for(&rt, "success", purpose).unwrap();
        assert_eq!(stored.session_id, result.session_id);
        assert!(rt.store.read(&stored.credential_key).unwrap().is_some());
        let serialized = serde_json::to_string(&result).unwrap();
        assert!(!serialized.contains(purpose.access_prefix()));
        assert_eq!(server.requests.lock().unwrap().len(), 3);
    }
}
