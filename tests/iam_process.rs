//! Only the OS browser and credential store are replaced. All auth traffic uses the production IAM HTTP handlers.
use dt_cli::{
    Runtime,
    credentials::{CredentialStore, Credentials},
    login::Browser,
    output::Result,
    profile::Environment,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
#[derive(Clone, Default)]
struct Store(Arc<Mutex<BTreeMap<String, String>>>);
impl CredentialStore for Store {
    fn read(&self, k: &str) -> Result<Option<Credentials>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(k)
            .map(|v| serde_json::from_str(v).unwrap()))
    }
    fn write(&self, k: &str, c: &Credentials) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(k.into(), serde_json::to_string(c).unwrap());
        Ok(())
    }
    fn delete(&self, k: &str) -> Result<()> {
        self.0.lock().unwrap().remove(k);
        Ok(())
    }
}
struct FixtureBrowser {
    api: String,
    employee: String,
}
impl Browser for FixtureBrowser {
    fn open(&self, url: &str) -> Result<()> {
        let url = url.to_owned();
        let api = self.api.clone();
        let employee = self.employee.clone();
        std::thread::spawn(move || {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async move {
                    let client = reqwest::Client::builder()
                        .redirect(reqwest::redirect::Policy::none())
                        .build()
                        .unwrap();
                    let r = client.get(url).send().await.unwrap();
                    assert_eq!(r.status(), 302);
                    let redirect =
                        url::Url::parse(r.headers()["location"].to_str().unwrap()).unwrap();
                    let id = redirect
                        .query_pairs()
                        .find(|(k, _)| k == "requestId")
                        .unwrap()
                        .1
                        .to_string();
                    let detail: Value = client
                        .get(format!("{api}/portal/cli/consents/{id}"))
                        .bearer_auth(&employee)
                        .send()
                        .await
                        .unwrap()
                        .json()
                        .await
                        .unwrap();
                    let response: Value = client
                        .post(format!("{api}/portal/cli/consents"))
                        .bearer_auth(employee)
                        .header("Origin", "https://portal.fixture.invalid")
                        .header("X-CLI-CSRF", detail["data"]["csrfToken"].as_str().unwrap())
                        .json(&json!({"requestId":id,"decision":"approve"}))
                        .send()
                        .await
                        .unwrap()
                        .json()
                        .await
                        .unwrap();
                    client
                        .get(response["data"]["redirectUri"].as_str().unwrap())
                        .send()
                        .await
                        .unwrap();
                });
        });
        Ok(())
    }
}
struct UnavailableStore;
impl CredentialStore for UnavailableStore {
    fn read(&self, _: &str) -> Result<Option<Credentials>> {
        Err(dt_cli::output::storage())
    }
    fn write(&self, _: &str, _: &Credentials) -> Result<()> {
        Err(dt_cli::output::storage())
    }
    fn delete(&self, _: &str) -> Result<()> {
        Err(dt_cli::output::storage())
    }
}
async fn run(rt: &Runtime, args: &[&str]) -> (Value, u8) {
    let (v, c, _) = dt_cli::execute(
        rt,
        std::iter::once("dt-cli")
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect(),
    )
    .await;
    (v, c)
}
#[test]
#[ignore = "started by IAM CliHttpTest with an isolated TCP server"]
fn iam_process_child() {
    let api = std::env::var("DT_CLI_FIXTURE_URL").unwrap();
    tokio::runtime::Runtime::new().unwrap().block_on(async {
        let root = tempfile::tempdir().unwrap();
        let store = Store::default();
        let mut rt = Runtime {
            root: root.path().into(),
            environments: BTreeMap::from([(
                "fixture".into(),
                Environment {
                    api_origin: api.clone(),
                    portal_origin: "https://portal.fixture.invalid".into(),
                    recovery_url: "https://portal.fixture.invalid/cli".into(),
                    launch_path: "/cli/launch".into(),
                },
            )]),
            store: Box::new(store.clone()),
            browser: Box::new(FixtureBrowser {
                api: api.clone(),
                employee: "fixture-portal-a".into(),
            }),
            interactive: true,
            aggregate_budget: std::time::Duration::from_secs(30),
        };
        let (v, c) = run(
            &rt,
            &[
                "setup",
                "--profile",
                "中文 空格",
                "--environment",
                "fixture",
            ],
        )
        .await;
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["subject"]["id"], "1");
        let original = dt_cli::profile::read(&rt.root, "中文 空格")
            .unwrap()
            .unwrap();
        for args in [
            vec!["whoami", "--profile", "中文 空格"],
            vec!["auth", "check", "--profile", "中文 空格"],
            vec![
                "api",
                "call",
                "iam.profile.get",
                "--profile",
                "中文 空格",
                "--params",
                "{}",
            ],
            vec!["doctor", "--online", "--profile", "中文 空格"],
        ] {
            let (v, c) = run(&rt, &args).await;
            assert_eq!(c, 0, "{v}");
            assert!(v["data"]["checkedAt"].is_string());
            assert!(!v.to_string().contains("dtcli_a_"));
        }
        let (v, c) = run(&rt, &["auth", "status", "--profile", "中文 空格"]).await;
        assert_eq!(c, 0);
        assert_eq!(v["data"]["onlineVerified"], false);
        rt.browser = Box::new(FixtureBrowser {
            api: api.clone(),
            employee: "fixture-portal-b".into(),
        });
        let (v, c) = run(&rt, &["auth", "login", "--profile", "中文 空格"]).await;
        assert_eq!(c, 4, "{v}");
        assert_eq!(v["meta"]["recovery"]["remoteRevocation"], "confirmed");
        assert_eq!(
            dt_cli::profile::read(&rt.root, "中文 空格")
                .unwrap()
                .unwrap()
                .authorization_id,
            original.authorization_id
        );
        rt.browser = Box::new(FixtureBrowser {
            api: api.clone(),
            employee: "fixture-portal-a".into(),
        });
        rt.store = Box::new(UnavailableStore);
        let (v, c) = run(&rt, &["auth", "login", "--profile", "中文 空格"]).await;
        assert_eq!(c, 1, "{v}");
        assert_eq!(v["meta"]["recovery"]["remoteRevocation"], "confirmed");
        assert_eq!(
            dt_cli::profile::read(&rt.root, "中文 空格")
                .unwrap()
                .unwrap()
                .authorization_id,
            original.authorization_id
        );
        rt.store = Box::new(store.clone());
        let (v, c) = run(&rt, &["auth", "login", "--profile", "中文 空格"]).await;
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["previousAuthorizationRevocation"], "confirmed");
        let p = dt_cli::profile::read(&rt.root, "中文 空格")
            .unwrap()
            .unwrap();
        assert_ne!(p.authorization_id, original.authorization_id);
        assert!(store.read(&original.credential_key).unwrap().is_none());
    });
}
