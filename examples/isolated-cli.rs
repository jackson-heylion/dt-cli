//! Explicit acceptance fixture. Not installed by `cargo install` or part of release artifacts.
//! Uses the production command executor and real OS secure store; only environment/browser-launch
//! boundaries are supplied by the isolated IAM demo harness.
use dt_cli::{
    Runtime,
    credentials::{CredentialStore, SystemStore},
    login::Browser,
    output::{self, Result},
    profile::{self, Environment},
};
use serde::Deserialize;
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    origin: String,
    root: PathBuf,
    browser_request: PathBuf,
}
struct BrowserHandoff(PathBuf);
impl Browser for BrowserHandoff {
    fn open(&self, url: &str) -> Result<()> {
        std::fs::write(&self.0, url).map_err(|_| output::storage())
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let config = std::env::var_os("DT_CLI_ISOLATED_FIXTURE")
        .expect("explicit fixture configuration required");
    let fixture: Fixture = serde_json::from_slice(&std::fs::read(config).expect("fixture file"))
        .expect("fixture schema");
    let origin = url::Url::parse(&fixture.origin).expect("fixture origin");
    assert!(
        origin.scheme() == "http"
            && matches!(origin.host_str(), Some("127.0.0.1") | Some("localhost"))
            && origin.port().is_some(),
        "fixture is loopback only"
    );
    let env = Environment {
        api_origin: fixture.origin.clone(),
        portal_origin: fixture.origin.clone(),
        recovery_url: format!("{}/cli-consent.html", fixture.origin),
        launch_path: "/cli/launch".into(),
    };
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).is_some_and(|s| s == "fixture-cleanup") {
        let profile = profile::read(&fixture.root, "browser-demo").expect("profile read");
        let mut remote = "not-applicable";
        if let Some(profile) = profile {
            assert_eq!(
                profile.environment, "fixture",
                "only fixture profiles may be cleaned"
            );
            if let Some(credentials) = SystemStore
                .read(&profile.credential_key)
                .expect("secure store read")
            {
                remote = if dt_cli::http::revoke(
                    &dt_cli::http::client().unwrap(),
                    &env,
                    &credentials.refresh_token,
                )
                .await
                {
                    "confirmed"
                } else {
                    "unknown"
                };
            }
            SystemStore
                .delete(&profile.credential_key)
                .expect("secure store cleanup");
            assert!(
                SystemStore
                    .read(&profile.credential_key)
                    .expect("secure store verification")
                    .is_none()
            );
            std::fs::remove_file(fixture.root.join("browser-demo.json")).expect("profile cleanup");
        }
        println!(
            "{}",
            serde_json::json!({"localCleanup":"confirmed","remoteRevocation":remote})
        );
        return;
    }
    let rt = Runtime {
        root: fixture.root,
        environments: BTreeMap::from([("fixture".into(), env)]),
        store: Box::new(SystemStore),
        browser: Box::new(BrowserHandoff(fixture.browser_request)),
        interactive: dt_cli::login::terminal(),
        aggregate_budget: std::time::Duration::from_secs(dt_cli::pagination::AGGREGATE_SECONDS),
    };
    let (value, exit, table) = dt_cli::execute(&rt, args).await;
    println!("{}", output::render(&value, table));
    std::process::exit(exit.into());
}
