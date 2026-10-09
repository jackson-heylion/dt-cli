use dt_cli::{
    Runtime,
    credentials::{CredentialStore, Credentials},
    login::Browser,
    output::{Failure, Result},
    profile::Environment,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
struct NoCredentials;
impl CredentialStore for NoCredentials {
    fn read(&self, _: &str) -> Result<Option<Credentials>> {
        panic!("callback failure read credentials")
    }
    fn write(&self, _: &str, _: &Credentials) -> Result<()> {
        panic!("callback failure wrote credentials")
    }
    fn delete(&self, _: &str) -> Result<()> {
        panic!("callback failure deleted credentials")
    }
}
struct Callback {
    mode: &'static str,
    port: Arc<Mutex<Option<u16>>>,
}
impl Browser for Callback {
    fn open(&self, address: &str) -> Result<()> {
        let authorization = url::Url::parse(address).unwrap();
        let fields: BTreeMap<String, String> = authorization
            .query_pairs()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let mut redirect = url::Url::parse(&fields["redirect_uri"]).unwrap();
        *self.port.lock().unwrap() = redirect.port();
        if self.mode == "browser-failure" {
            return Err(Failure::new("BROWSER_OPEN_FAILED", 6, "浏览器未能启动。"));
        }
        if self.mode == "timeout" {
            return Ok(());
        }
        redirect
            .query_pairs_mut()
            .append_pair(
                "state",
                if self.mode == "state" {
                    "wrong-state"
                } else {
                    &fields["state"]
                },
            )
            .append_pair(
                "iss",
                if self.mode == "issuer" {
                    "https://evil.invalid"
                } else {
                    "https://iam.fixture.invalid"
                },
            );
        if self.mode == "deny" {
            redirect
                .query_pairs_mut()
                .append_pair("error", "access_denied");
        } else {
            redirect
                .query_pairs_mut()
                .append_pair("code", "synthetic-code");
        }
        if self.mode == "path" {
            redirect.set_path("/wrong");
        }
        if self.mode == "duplicate" {
            redirect
                .query_pairs_mut()
                .append_pair("state", &fields["state"]);
        }
        let wrong_host = self.mode == "host";
        tokio::spawn(async move {
            let mut request = reqwest::Client::new().get(redirect);
            if wrong_host {
                request = request.header("Host", "evil.invalid");
            }
            let _ = request.send().await;
        });
        Ok(())
    }
}
async fn check(mode: &'static str, expected: u8) {
    let root = tempfile::tempdir().unwrap();
    let port = Arc::new(Mutex::new(None));
    let rt = Runtime {
        root: root.path().into(),
        environments: BTreeMap::from([(
            "fixture".into(),
            Environment {
                api_origin: "https://iam.fixture.invalid".into(),
                portal_origin: "https://portal.fixture.invalid".into(),
                recovery_url: "https://portal.fixture.invalid/cli".into(),
                launch_path: "/cli/launch".into(),
            },
        )]),
        store: Box::new(NoCredentials),
        aggregate_budget: std::time::Duration::from_secs(30),
        browser: Box::new(Callback {
            mode,
            port: port.clone(),
        }),
        interactive: false,
    };
    let (v, code, _) = dt_cli::execute(
        &rt,
        [
            "dt-cli",
            "auth",
            "login",
            "--profile",
            "p",
            "--environment",
            "fixture",
            "--interaction",
            "browser",
        ]
        .map(str::to_owned)
        .to_vec(),
    )
    .await;
    assert_eq!(code, expected, "mode={mode}: {v}");
    assert!(!v.to_string().contains("synthetic-code"));
    let p = port.lock().unwrap().unwrap();
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, p)).unwrap();
    drop(listener);
    assert!(dt_cli::profile::read(root.path(), "p").unwrap().is_none());
    let lock = dt_cli::profile::lock(root.path(), "p").await.unwrap();
    drop(lock);
}
#[tokio::test]
async fn callback_rejects_state_issuer_path_duplicate_and_denial() {
    for (mode, exit) in [
        ("host", 2),
        ("state", 4),
        ("issuer", 4),
        ("path", 2),
        ("duplicate", 2),
        ("deny", 4),
        ("browser-failure", 6),
    ] {
        check(mode, exit).await;
    }
}
#[tokio::test(start_paused = true)]
async fn browser_deadline_is_bounded_and_cleans_up() {
    check("timeout", 5).await;
}
