//! `workflow open` contracts: one explicit message, sensitive-result adaptation, and a
//! browser handoff that never leaks the internal launch URL.
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
    time::Duration,
};

const CREDENTIAL_KEY: &str = "fixture/1/auth-fixture";
const PORTAL: &str = "https://portal.fixture.invalid";
const TICKET: &str = "fixture-delivery-ticket-value";

#[derive(Default)]
struct State {
    paths: Vec<String>,
    bodies: Vec<Value>,
}

struct Server {
    origin: String,
    state: Arc<Mutex<State>>,
}
impl Server {
    fn paths(&self) -> Vec<String> {
        self.state.lock().unwrap().paths.clone()
    }
    fn open_calls(&self) -> usize {
        self.paths()
            .iter()
            .filter(|path| path.contains("/open"))
            .count()
    }
    fn bodies(&self) -> Vec<Value> {
        self.state.lock().unwrap().bodies.clone()
    }
}

fn serve<F>(handler: F) -> Server
where
    F: Fn(usize, &str, &Value) -> (u16, Vec<u8>) + Send + Sync + 'static,
{
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let state: Arc<Mutex<State>> = Arc::new(Mutex::new(State::default()));
    let shared = state.clone();
    let handler = Arc::new(handler);
    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(mut stream) = incoming else { break };
            let state = shared.clone();
            let handler = handler.clone();
            std::thread::spawn(move || {
                use std::io::{Read, Write};
                stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
                let mut data = Vec::new();
                let mut chunk = [0u8; 4096];
                let head_end = loop {
                    let Ok(n) = stream.read(&mut chunk) else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&chunk[..n]);
                    if let Some(position) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        break position + 4;
                    }
                };
                let head = String::from_utf8_lossy(&data[..head_end]).to_string();
                let path = head
                    .lines()
                    .next()
                    .and_then(|line| line.split(' ').nth(1))
                    .unwrap_or_default()
                    .to_owned();
                let length = head
                    .lines()
                    .find_map(|line| {
                        line.to_lowercase()
                            .strip_prefix("content-length:")
                            .map(|n| n.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while data.len() < head_end + length {
                    let Ok(n) = stream.read(&mut chunk) else {
                        return;
                    };
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(&chunk[..n]);
                }
                let body = if length > 0 && data.len() >= head_end + length {
                    serde_json::from_slice(&data[head_end..head_end + length])
                        .unwrap_or(Value::Null)
                } else {
                    Value::Null
                };
                let index = {
                    let mut state = state.lock().unwrap();
                    state.paths.push(path.clone());
                    state.bodies.push(body.clone());
                    state.paths.len() - 1
                };
                let (status, payload) = handler(index, &path, &body);
                let head = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
                    payload.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&payload);
            });
        }
    });
    Server { origin, state }
}

fn reply(status: u16, body: Value) -> (u16, Vec<u8>) {
    (status, serde_json::to_vec(&body).unwrap())
}

fn launch_body(id: &str, application_id: &str, url: &str, expires_in: i64) -> Value {
    json!({"schemaVersion":"1.0","operationId":"iam.workflow.open","ok":true,"profile":null,
        "data":{"messageId":id,"applicationId":application_id,"launchUrl":url,
            "expiresAt":(chrono::Utc::now() + chrono::Duration::seconds(expires_in)).to_rfc3339()},
        "error":null,"meta":{"traceId":"0123456789abcdef"}})
}

fn entry() -> String {
    format!("{PORTAL}/cli/launch?ticket={TICKET}")
}

/// How the system browser stub behaves, so each test can prove whether it was reached.
#[derive(Clone)]
enum BrowserMode {
    Record(Arc<Mutex<Vec<String>>>),
    Fail,
    Forbid,
}
impl Browser for BrowserMode {
    fn open(&self, url: &str) -> Result<()> {
        match self {
            Self::Record(seen) => {
                seen.lock().unwrap().push(url.to_owned());
                Ok(())
            }
            Self::Fail => Err(dt_cli::output::Failure::new(
                "BROWSER_OPEN_FAILED",
                6,
                "浏览器未能启动。",
            )),
            Self::Forbid => panic!("this command must not open a browser"),
        }
    }
}

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
            .insert(key.into(), serde_json::to_string(credentials).unwrap());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

struct ForbiddenStore;
impl CredentialStore for ForbiddenStore {
    fn read(&self, _: &str) -> Result<Option<Credentials>> {
        panic!("this command must not read credentials")
    }
    fn write(&self, _: &str, _: &Credentials) -> Result<()> {
        panic!("this command must not write credentials")
    }
    fn delete(&self, _: &str) -> Result<()> {
        panic!("this command must not delete credentials")
    }
}

fn environment(origin: &str) -> Environment {
    Environment {
        api_origin: origin.into(),
        portal_origin: PORTAL.into(),
        recovery_url: format!("{PORTAL}/cli-authorizations.html"),
        launch_path: "/cli/launch".into(),
    }
}

fn credentials() -> Credentials {
    let now = chrono::Utc::now();
    Credentials {
        access_token: "dtcli_a_fixture-access".into(),
        refresh_token: "dtcli_r_fixture-refresh".into(),
        version: 1,
        access_expires_at: Some((now + chrono::Duration::hours(1)).to_rfc3339()),
        authorization_expires_at: Some((now + chrono::Duration::days(6)).to_rfc3339()),
        generation: 0,
        refresh_state: "ready".into(),
        refresh_request_id: None,
    }
}

struct Harness {
    _root: tempfile::TempDir,
    runtime: Runtime,
    opened: Arc<Mutex<Vec<String>>>,
    store: MemoryStore,
}

fn harness(origin: &str, mode: BrowserMode) -> Harness {
    let opened = match &mode {
        BrowserMode::Record(seen) => seen.clone(),
        _ => Arc::new(Mutex::new(Vec::new())),
    };
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    store.write(CREDENTIAL_KEY, &credentials()).unwrap();
    let now = chrono::Utc::now();
    let profile = json!({"environment":"fixture","subjectId":"1","authorizationId":"auth-fixture",
        "authorizationExpiresAt":(now + chrono::Duration::days(6)).to_rfc3339(),
        "accessExpiresAt":(now + chrono::Duration::hours(1)).to_rfc3339(),
        "checkedAt":"2026-09-22T00:00:00Z","credentialKey":CREDENTIAL_KEY});
    std::fs::write(root.path().join("p.json"), profile.to_string()).unwrap();
    Harness {
        runtime: Runtime {
            root: root.path().into(),
            environments: BTreeMap::from([("fixture".into(), environment(origin))]),
            store: Box::new(store.clone()),
            browser: Box::new(mode),
            interactive: false,
            aggregate_budget: Duration::from_secs(5),
        },
        opened,
        store,
        _root: root,
    }
}

/// A runtime whose credential store never allows access, for zero-side-effect commands.
fn sealed(origin: &str) -> (tempfile::TempDir, Runtime) {
    let root = tempfile::tempdir().unwrap();
    let runtime = Runtime {
        root: root.path().into(),
        environments: BTreeMap::from([("fixture".into(), environment(origin))]),
        store: Box::new(ForbiddenStore),
        browser: Box::new(BrowserMode::Forbid),
        interactive: false,
        aggregate_budget: Duration::from_secs(5),
    };
    (root, runtime)
}

fn recording() -> BrowserMode {
    BrowserMode::Record(Arc::new(Mutex::new(Vec::new())))
}

async fn run(rt: &Runtime, args: &[&str]) -> (Value, u8, bool) {
    dt_cli::execute(
        rt,
        std::iter::once("dt-cli")
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect(),
    )
    .await
}

/// The internal entry and its ticket must never be observable from the client.
fn assert_no_entry(text: &str, label: &str) {
    for needle in ["ticket", TICKET, "launchUrl", "cli/launch", "SSO"] {
        assert!(!text.contains(needle), "{label} leaked {needle}: {text}");
    }
}

// 1: exactly one explicit, canonical message identifier.

#[path = "open_contract/cases.rs"]
mod cases;
