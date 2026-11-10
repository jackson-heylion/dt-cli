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
