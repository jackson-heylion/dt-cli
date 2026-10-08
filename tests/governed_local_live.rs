//! Live smoke test against the dt-iam local debug fixture (`scripts/run-cli-local.sh` in dt-iam,
//! which also seeds the governed supply-chain templates against synthetic loopback replicas).
//! The real CLI code talks to the real IAM governed entry over HTTP; only the browser is scripted.
//!
//! DT_IAM_CLI_LOCAL_PASSWORD=... cargo test --features local-dev --test governed_local_live -- --ignored
#![cfg(feature = "local-dev")]
use dt_cli::{
    Runtime,
    credentials::{CredentialStore, Credentials},
    login::Browser,
    output::Result,
    profile,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

const ORIGIN: &str = "http://localhost:10998";
const READ: &str = "supply-chain-server.order-config.read";
const SHOPS: &str = "supply-chain-server.shops.list";
const BATCH: &str = "supply-chain-server.order-config.batch.read";

#[derive(Clone, Default)]
struct MemoryStore(Arc<Mutex<BTreeMap<String, String>>>);
impl CredentialStore for MemoryStore {
    fn read(&self, key: &str) -> Result<Option<Credentials>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(key)
            .map(|value| serde_json::from_str(value).unwrap()))
    }
    fn write(&self, key: &str, credentials: &Credentials) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(key.to_owned(), serde_json::to_string(credentials).unwrap());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

/// Performs the employee's browser steps on the real IAM pages' APIs, then delivers the code.
struct ScriptedConsent {
    password: String,
}
impl Browser for ScriptedConsent {
    fn open(&self, address: &str) -> Result<()> {
        let (address, password) = (address.to_owned(), self.password.clone());
        tokio::spawn(async move { consent(address, password).await });
        Ok(())
    }
}
async fn consent(authorize: String, password: String) {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let located = client.get(&authorize).send().await.unwrap();
    assert_eq!(located.status(), 302);
    let location = located.headers()["location"].to_str().unwrap().to_owned();
    let request_id = location.split("requestId=").nth(1).unwrap().to_owned();
    let begin = client
        .get(format!(
            "{ORIGIN}/cli-api/oauth2/browser/login?requestId={request_id}"
        ))
        .send()
        .await
        .unwrap();
    let cookie = begin.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let started: Value = begin.json().await.unwrap();
    let verified = client
        .post(format!("{ORIGIN}/cli-api/oauth2/browser/login"))
        .header("Cookie", &cookie)
        .header("Origin", ORIGIN)
        .header(
            "X-Governed-CSRF",
            started["data"]["csrfToken"].as_str().unwrap(),
        )
        .json(&json!({"requestId": request_id, "account": "local-dev", "password": password}))
        .send()
        .await
        .unwrap();
    assert_eq!(verified.status(), 200);
    let page: Value = client
        .get(format!("{ORIGIN}/portal/cli-api/consents/{request_id}"))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let contracts: Vec<String> = page["data"]["contracts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            format!(
                "{}@{}#{}",
                c["operationId"].as_str().unwrap(),
                c["operationVersion"].as_str().unwrap(),
                c["contractDigest"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(contracts.len(), 3, "{page}");
    let decision: Value = client
        .post(format!("{ORIGIN}/portal/cli-api/consents/{request_id}"))
        .header("Cookie", &cookie)
        .header("Origin", ORIGIN)
        .header(
            "X-Governed-CSRF",
            page["data"]["csrfToken"].as_str().unwrap(),
        )
        .json(&json!({"decision": "approve", "contracts": contracts}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let redirect = decision["data"]["redirectUrl"].as_str().unwrap().to_owned();
    client.get(redirect).send().await.unwrap();
}

async fn run(rt: &Runtime, args: &[&str]) -> (Value, u8) {
    let (value, exit, _) = dt_cli::execute(
        rt,
        std::iter::once("dt-cli")
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect(),
    )
    .await;
    let text = value.to_string();
    for fragment in ["dtcli_g_a_", "dtcli_g_r_", "dtcli_g_s_", "local-native."] {
        assert!(
            !text.contains(fragment),
            "{args:?} leaked {fragment}: {text}"
        );
    }
    (value, exit)
}

#[tokio::test]
#[ignore = "needs the dt-iam local debug fixture on 127.0.0.1:10998"]
async fn governed_supply_chain_read_end_to_end_against_local_iam() {
    let password =
        std::env::var("DT_IAM_CLI_LOCAL_PASSWORD").expect("set DT_IAM_CLI_LOCAL_PASSWORD");
    let root = tempfile::tempdir().unwrap();
    let environments = profile::environments()
        .unwrap()
        .into_iter()
        .filter(|(name, _)| name == "local")
        .collect();
    let rt = Runtime {
        root: root.path().into(),
        environments,
        store: Box::new(MemoryStore::default()),
        browser: Box::new(ScriptedConsent { password }),
        interactive: true,
        aggregate_budget: Duration::from_secs(30),
    };

    let (login, exit) = run(
        &rt,
        &[
            "auth",
            "login",
            "--profile",
            "e2e",
            "--environment",
            "local",
            "--system",
            "supply-chain-server",
        ],
    )
    .await;
    assert_eq!(exit, 0, "{login}");
    assert_eq!(login["data"]["systemId"], "supply-chain-server");

    let (synced, exit) = run(&rt, &["catalog", "sync", "--profile", "e2e"]).await;
    assert_eq!(exit, 0, "{synced}");
    let (found, exit) = run(&rt, &["discover", "--profile", "e2e", "--query", "order"]).await;
    assert_eq!(exit, 0, "{found}");
    assert_eq!(found["data"]["permissionVerified"], false);
    let (schema, exit) = run(&rt, &["schema", "--profile", "e2e", READ]).await;
    assert_eq!(exit, 0, "{schema}");

    let own = r#"{"shopCode":"2021003","itemCode":"101010009"}"#;
    let (read, exit) = run(
        &rt,
        &["api", "call", READ, "--profile", "e2e", "--params", own],
    )
    .await;
    assert_eq!(exit, 0, "{read}");
    let rows = read["data"]["items"].as_array().expect("items");
    assert_eq!(rows.len(), 1, "{read}");
    assert_eq!(rows[0]["minOrderNum"], "1");
    assert_eq!(rows[0]["maxOrderNum"], Value::Null);
    assert!(rows[0].get("minSafeStock").is_none() && rows[0].get("creator").is_none());

    let (shops, exit) = run(
        &rt,
        &["api", "call", SHOPS, "--profile", "e2e", "--params", "{}"],
    )
    .await;
    assert_eq!(exit, 0, "{shops}");
    let codes: Vec<_> = shops["data"]["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|shop| shop["shopCode"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(codes, ["2021003", "2021004"]);

    let foreign = r#"{"shopCode":"2029999","itemCode":"101010009"}"#;
    let (denied, exit) = run(
        &rt,
        &["api", "call", READ, "--profile", "e2e", "--params", foreign],
    )
    .await;
    assert_eq!(exit, 4, "{denied}");
    assert_eq!(denied["error"]["code"], "DATA_SCOPE_DENIED");

    let (submitted, exit) = run(
        &rt,
        &[
            "jobs",
            "submit",
            BATCH,
            "--profile",
            "e2e",
            "--params",
            r#"{"itemName":"白芝麻"}"#,
        ],
    )
    .await;
    assert_eq!(exit, 0, "{submitted}");
    let job = submitted["data"]["jobId"].as_str().unwrap().to_owned();
    let mut state = String::new();
    for _ in 0..100 {
        let (status, exit) = run(&rt, &["jobs", "status", &job, "--profile", "e2e"]).await;
        assert_eq!(exit, 0, "{status}");
        state = status["data"]["state"].as_str().unwrap_or("").to_owned();
        if state != "queued" && state != "running" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(state, "succeeded");
    let (result, exit) = run(&rt, &["jobs", "result", &job, "--profile", "e2e"]).await;
    assert_eq!(exit, 0, "{result}");
    assert_eq!(result["data"]["complete"], true);
    assert_eq!(
        result["data"]["items"].as_array().unwrap().len(),
        4,
        "{result}"
    );

    let (logout, exit) = run(&rt, &["auth", "logout", "--profile", "e2e"]).await;
    assert_eq!(exit, 0, "{logout}");
    assert_eq!(logout["data"]["remoteRevocation"], "confirmed");
}
