//! Bounded aggregation contracts, driven through the real CLI executor over a controllable
//! multi-page HTTP fixture. Nothing here replaces authentication: the fixture only stands in
//! for the IAM query endpoint, and every request still carries the stored CLI access token.
use dt_cli::{
    Runtime,
    credentials::{CredentialStore, Credentials},
    output::Result,
    pagination,
    profile::{Environment, Profile},
};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const CREDENTIAL_KEY: &str = "fixture/1/auth-fixture";

#[derive(Default)]
struct SharedState {
    entries: Vec<(String, Value)>,
}

#[derive(Default)]
struct Reply {
    abort: bool,
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    delay_ms: u64,
}
impl Reply {
    fn json(value: &Value) -> Self {
        Self {
            status: 200,
            body: serde_json::to_vec(value).unwrap(),
            ..Self::default()
        }
    }
    fn raw(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            body,
            ..Self::default()
        }
    }
    fn error(status: u16, body: Value) -> Self {
        Self {
            status,
            body: serde_json::to_vec(&body).unwrap(),
            ..Self::default()
        }
    }
    fn abort() -> Self {
        Self {
            abort: true,
            ..Self::default()
        }
    }
    fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
    fn delay(mut self, ms: u64) -> Self {
        self.delay_ms = ms;
        self
    }
}

struct Server {
    origin: String,
    state: Arc<Mutex<SharedState>>,
}
impl Server {
    fn requests(&self) -> Vec<(String, Value)> {
        self.state.lock().unwrap().entries.clone()
    }
    fn pages(&self) -> Vec<i64> {
        self.requests()
            .iter()
            .filter_map(|(_, body)| body["page"].as_i64())
            .collect()
    }
}

fn message(id: &str) -> Value {
    json!({"id":id,"kind":"todo","title":format!("消息 {id}"),"applicationId":"7",
        "applicationName":"门户","formType":null,"initiatorName":"张三",
        "startedAt":"2026-09-01T02:00:00Z","arrivedAt":"2026-09-01T03:00:00Z",
        "processedAt":null,"endedAt":null})
}

#[derive(Clone)]
struct Page {
    items: Vec<Value>,
    total: Option<i64>,
    has_more: Option<bool>,
    next_page: Option<i64>,
    page: Option<i64>,
    page_size: Option<i64>,
    items_fetched: Option<i64>,
    items_returned: Option<i64>,
    page_complete: Option<bool>,
    content_complete: Option<bool>,
    scope_complete: Option<Value>,
    mode: String,
    omit: Vec<&'static str>,
    omit_pagination: bool,
}
impl Page {
    fn new(ids: &[&str]) -> Self {
        Self {
            items: ids.iter().map(|id| message(id)).collect(),
            total: Some(ids.len() as i64),
            has_more: Some(false),
            next_page: None,
            page: None,
            page_size: None,
            items_fetched: None,
            items_returned: None,
            page_complete: None,
            content_complete: None,
            scope_complete: None,
            mode: "page".into(),
            omit: Vec::new(),
            omit_pagination: false,
        }
    }
    fn items(items: Vec<Value>) -> Self {
        Self {
            items,
            ..Self::new(&[])
        }
    }
    fn total(mut self, total: i64) -> Self {
        self.total = Some(total);
        self
    }
    fn unknown_total(mut self) -> Self {
        self.total = None;
        self
    }
    fn has_more(mut self, has_more: bool) -> Self {
        self.has_more = Some(has_more);
        self
    }
    #[allow(dead_code)]
    fn unknown_has_more(mut self) -> Self {
        self.has_more = None;
        self
    }
    fn next(mut self, next: Option<i64>) -> Self {
        self.next_page = next;
        self
    }
    fn page(mut self, page: i64) -> Self {
        self.page = Some(page);
        self
    }
    fn omit_field(mut self, field: &'static str) -> Self {
        self.omit.push(field);
        self
    }
    fn no_pagination(mut self) -> Self {
        self.omit_pagination = true;
        self
    }
    fn to_json(&self, requested_page: i64, requested_size: i64) -> Value {
        let count = self.items.len() as i64;
        let mut pagination = Map::new();
        pagination.insert("mode".into(), json!(self.mode));
        pagination.insert("page".into(), json!(self.page.unwrap_or(requested_page)));
        pagination.insert(
            "pageSize".into(),
            json!(self.page_size.unwrap_or(requested_size)),
        );
        pagination.insert("pagesRead".into(), json!(1));
        pagination.insert("matchedTotal".into(), json!(self.total));
        pagination.insert(
            "itemsFetched".into(),
            json!(self.items_fetched.unwrap_or(count)),
        );
        pagination.insert(
            "itemsReturned".into(),
            json!(self.items_returned.unwrap_or(count)),
        );
        pagination.insert("hasMore".into(), json!(self.has_more));
        pagination.insert("nextPage".into(), json!(self.next_page));
        pagination.insert(
            "pageComplete".into(),
            json!(self.page_complete.unwrap_or(true)),
        );
        pagination.insert(
            "scopeComplete".into(),
            self.scope_complete.clone().unwrap_or(json!(false)),
        );
        pagination.insert("truncated".into(), json!(false));
        pagination.insert("reason".into(), Value::Null);
        pagination.insert(
            "contentComplete".into(),
            json!(self.content_complete.unwrap_or(true)),
        );
        for field in &self.omit {
            pagination.remove(*field);
        }
        let mut meta = Map::new();
        meta.insert("traceId".into(), json!("0123456789abcdef"));
        if !self.omit_pagination {
            meta.insert("pagination".into(), Value::Object(pagination));
        }
        meta.insert(
            "source".into(),
            json!({"fetchedAt":"2026-09-22T00:00:00Z","sourceSyncedAt":null,
                "freshness":"unknown","consistency":"non_snapshot"}),
        );
        json!({"schemaVersion":"1.0","operationId":"iam.workflow.list","ok":true,
            "profile":null,"data":self.items,"error":null,"meta":meta})
    }
}

#[derive(Clone)]
struct MemoryStore(Arc<Mutex<BTreeMap<String, String>>>);
impl Default for MemoryStore {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(BTreeMap::new())))
    }
}
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

struct NoBrowser;
impl dt_cli::login::Browser for NoBrowser {
    fn open(&self, _: &str) -> Result<()> {
        panic!("a read-only aggregation must never open a browser")
    }
}

struct Harness {
    _root: tempfile::TempDir,
    store: MemoryStore,
    runtime: Runtime,
}
struct PanickingStore(Arc<AtomicBool>);
impl CredentialStore for PanickingStore {
    fn read(&self, _: &str) -> Result<Option<Credentials>> {
        self.0.store(true, Ordering::SeqCst);
        panic!("this command must not read credentials")
    }
    fn write(&self, _: &str, _: &Credentials) -> Result<()> {
        panic!("this command must not write credentials")
    }
    fn delete(&self, _: &str) -> Result<()> {
        panic!("this command must not delete credentials")
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

fn harness(origin: &str, budget: Duration) -> Harness {
    let root = tempfile::tempdir().unwrap();
    let store = MemoryStore::default();
    let stored = credentials();
    store.write(CREDENTIAL_KEY, &stored).unwrap();
    let profile = json!({"environment":"fixture","subjectId":"1","authorizationId":"auth-fixture",
        "authorizationExpiresAt":stored.authorization_expires_at,"accessExpiresAt":stored.access_expires_at,
        "checkedAt":"2026-09-22T00:00:00Z","credentialKey":CREDENTIAL_KEY});
    std::fs::write(root.path().join("p.json"), profile.to_string()).unwrap();
    let runtime = Runtime {
        root: root.path().into(),
        environments: BTreeMap::from([(
            "fixture".into(),
            Environment {
                api_origin: origin.into(),
                portal_origin: "https://portal.fixture.invalid".into(),
                recovery_url: "https://portal.fixture.invalid/cli".into(),
                launch_path: "/cli/launch".into(),
            },
        )]),
        store: Box::new(store.clone()),
        browser: Box::new(NoBrowser),
        interactive: false,
        aggregate_budget: budget,
    };
    Harness {
        _root: root,
        store,
        runtime,
    }
}

/// A runtime whose credential store panics if anything touches it.
fn sealed(root: &std::path::Path, origin: &str, budget: Duration) -> Runtime {
    Runtime {
        root: root.into(),
        environments: BTreeMap::from([(
            "fixture".into(),
            Environment {
                api_origin: origin.into(),
                portal_origin: "https://portal.fixture.invalid".into(),
                recovery_url: "https://portal.fixture.invalid/cli".into(),
                launch_path: "/cli/launch".into(),
            },
        )]),
        store: Box::new(PanickingStore(Arc::new(AtomicBool::new(false)))),
        browser: Box::new(NoBrowser),
        interactive: false,
        aggregate_budget: budget,
    }
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
    (value, exit)
}

async fn run_table(rt: &Runtime, args: &[&str]) -> (Value, u8, String) {
    let (value, exit, table) = dt_cli::execute(
        rt,
        std::iter::once("dt-cli")
            .chain(args.iter().copied())
            .map(str::to_owned)
            .collect(),
    )
    .await;
    let rendered = dt_cli::output::render(&value, table);
    (value, exit, rendered)
}

fn listing(page: i64, size: i64, ids: &[&str], total: i64, has_more: bool) -> Reply {
    Reply::json(
        &Page::new(ids)
            .total(total)
            .has_more(has_more)
            .next(has_more.then_some(page + 1))
            .to_json(page, size),
    )
}

/// Three pages of two records; page 3 proves the end of the scope.
fn three_pages(_: usize, _: &str, body: &Value) -> Reply {
    let page = body["page"].as_i64().unwrap();
    let size = body["pageSize"].as_i64().unwrap();
    let ids: &[&str] = match page {
        1 => &["a1", "a2"],
        2 => &["b1", "b2"],
        _ => &["c1"],
    };
    listing(page, size, ids, 5, page < 3)
}

fn body_of(value: &Value) -> &Value {
    &value["data"]
}

fn pagination(value: &Value) -> &Value {
    &value["meta"]["pagination"]
}

// 1 and 2: the default stays single-page, `--all` starts at page 1 and keeps pageSize fixed.

#[path = "pagination_contract/transport.rs"]
mod transport;
use transport::serve;
#[path = "pagination_contract/arguments.rs"]
mod arguments;
#[path = "pagination_contract/brief.rs"]
mod brief;
#[path = "pagination_contract/bytes.rs"]
mod bytes;
#[path = "pagination_contract/drift.rs"]
mod drift;
#[path = "pagination_contract/limits.rs"]
mod limits;
#[path = "pagination_contract/output.rs"]
mod output;
#[path = "pagination_contract/recovery.rs"]
mod recovery;
