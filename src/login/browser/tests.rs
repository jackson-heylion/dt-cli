use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

struct SilentBrowser;
impl Browser for SilentBrowser {
    fn open(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

#[tokio::test(start_paused = true)]
async fn an_unfinished_connection_is_distinct_from_no_callback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let login = BrowserLogin::begin(
        &SilentBrowser,
        "https://gateway.fixture.invalid/authorize",
        address,
        false,
    );
    let _stream = TcpStream::connect(address).await.unwrap();
    let error = login
        .wait(
            &listener,
            "expected-state",
            "https://gateway.fixture.invalid",
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "TIMEOUT");
    assert_eq!(
        serde_json::to_value(error.details.unwrap()).unwrap()["callbackStatus"],
        "connected"
    );
    assert!(!error.retryable);
}

#[tokio::test(start_paused = true)]
async fn unrelated_request_does_not_extend_login_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut stream = TcpStream::connect(address).await.unwrap();
    let login = BrowserLogin::begin(
        &SilentBrowser,
        "https://gateway.fixture.invalid/authorize",
        address,
        false,
    );
    let start = tokio::time::Instant::now();
    let noise = async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let request = format!("GET /favicon.ico HTTP/1.1\r\nHost: {address}\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
    };
    let (result, ()) = tokio::join!(
        login.wait(
            &listener,
            "expected-state",
            "https://gateway.fixture.invalid",
        ),
        noise,
    );
    let failure = result.unwrap_err();
    assert_eq!(failure.code, "TIMEOUT");
    assert_eq!(failure.exit, 5);
    assert_eq!(start.elapsed(), Duration::from_secs(1500));
    let details = failure.details.unwrap();
    assert_eq!(details.callback_status.as_deref(), Some("connected"));
    assert_eq!(details.login_stage.as_deref(), Some("timed_out"));
}

#[tokio::test]
async fn malformed_request_for_callback_still_fails_validation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let login = BrowserLogin::begin(
        &SilentBrowser,
        "https://gateway.fixture.invalid/authorize",
        address,
        false,
    );
    let mut stream = TcpStream::connect(address).await.unwrap();
    let request = format!("POST /oauth/callback HTTP/1.1\r\nHost: {address}\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let failure = login
        .wait(
            &listener,
            "expected-state",
            "https://gateway.fixture.invalid",
        )
        .await
        .unwrap_err();
    assert_eq!(failure.code, "INVALID_ARGUMENT");
    let details = failure.details.unwrap();
    assert_eq!(details.login_stage.as_deref(), Some("callback_validation"));
}

#[tokio::test]
async fn observed_callbacks_keep_state_and_issuer_validation() {
    for (state, issuer, expected) in [
        ("expected-state", "https://gateway.fixture.invalid", true),
        ("wrong-state", "https://gateway.fixture.invalid", false),
        ("expected-state", "https://other.fixture.invalid", false),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let login = BrowserLogin::begin(
            &SilentBrowser,
            "https://gateway.fixture.invalid/authorize",
            address,
            false,
        );
        let mut callback = url::Url::parse(&format!("http://{address}/oauth/callback")).unwrap();
        callback.query_pairs_mut().extend_pairs([
            ("state", state),
            ("iss", issuer),
            ("code", "fixture-secret-code"),
        ]);
        let mut stream = TcpStream::connect(address).await.unwrap();
        let request = format!(
            "GET {}?{} HTTP/1.1\r\nHost: {address}\r\n\r\n",
            callback.path(),
            callback.query().unwrap(),
        );
        let client = async {
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(!response.contains("fixture-secret-code"));
            assert!(!response.contains("expected-state"));
        };
        let (result, _) = tokio::join!(
            login.wait(
                &listener,
                "expected-state",
                "https://gateway.fixture.invalid"
            ),
            client,
        );
        assert!(login.connected.load(Ordering::Relaxed));
        if expected {
            assert_eq!(result.unwrap(), "fixture-secret-code");
        } else {
            assert_eq!(result.unwrap_err().code, "AUTH_DENIED");
        }
    }
}
