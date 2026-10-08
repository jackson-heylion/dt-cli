//! Governed-access command contracts against a fake IAM: an independent provider profile,
//! catalog cache and credentials, online contract checks before any exchange, fixed error
//! mappings, complete-only job results, and no credential ever echoed back.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use dt_cli::{
    Runtime,
    credentials::{CredentialStore, Credentials},
    login::Browser,
    output::Result,
    profile::Environment,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

const SYSTEM: &str = "supply-chain-server";
const ENV: &str = "fixture";
const READ: &str = "supply-chain-server.order-config.read";
const BATCH: &str = "supply-chain-server.order-config.batch.read";
const SHOPS: &str = "supply-chain-server.shops.list";
const OLD_KEY: &str = "fixture/1/auth-fixture";
const OLD_ACCESS: &str = "dtcli_a_fixture-access";
const OLD_REFRESH: &str = "dtcli_r_fixture-refresh";
const READ_PARAMS: &str = r#"{"shopCode":"2021003","itemCode":"101010009"}"#;
const LIKE: &str = "hrmp.like.send";
const LIKE_PARAMS: &str = r#"{"beLikedPersonId":"1003","likeTypeId":"64","likeTypeName":"勇","likeBehaviorContent":"勇于担当","specificDeeds":"周末主动支援新店开业"}"#;

#[derive(Clone, Debug)]
struct Seen {
    method: String,
    path: String,
    authorization: Option<String>,
    idempotency: Option<String>,
    body: String,
}
impl Seen {
    fn form(&self) -> BTreeMap<String, String> {
        url::form_urlencoded::parse(self.body.as_bytes())
            .into_owned()
            .collect()
    }
}

type Reply = (u16, Vec<(String, String)>, Vec<u8>);

/// A minimal stateful IAM: OAuth code + refresh rotation with replay detection, exact
/// contract exchange, invoke, jobs and the legacy `/cli/v1/me` for personal profiles.
struct Fake {
    seen: Vec<Seen>,
    system: String,
    environment: String,
    subject: String,
    authorization: String,
    authorization_expires_at: String,
    authorize: Vec<BTreeMap<String, String>>,
    challenge: Option<String>,
    code: String,
    access: String,
    refresh: String,
    retired_refresh: Vec<String>,
    refresh_receipts: BTreeMap<String, (String, Value, String)>,
    drop_refresh_reply_once: bool,
    system_tokens: Vec<String>,
    issued: Vec<String>,
    counter: u32,
    revoked: bool,
    catalog_401_once: bool,
    operations: Vec<Value>,
    items: Value,
    jobs: BTreeMap<String, String>,
    job_contracts: BTreeMap<String, Value>,
    auto_complete_jobs: bool,
    job_initial_state: Option<&'static str>,
    overrides: BTreeMap<&'static str, Reply>,
    origin: String,
    /// Write intents by id and their idempotency keys; `writes` counts likes actually sent.
    intents: BTreeMap<String, Value>,
    intent_keys: BTreeMap<String, (String, String)>,
    writes: u32,
    /// The next dispatch: "unknown" answers 202, "drop" closes without answer, "hold" stalls.
    dispatch: Option<&'static str>,
    auto_approve_prepared: bool,
    hold_prepare_reply_once: bool,
    hold_authorize_reply_once: bool,
    drop_prepare_reply_once: bool,
    drop_authorize_reply_once: bool,
    params_mutation: Option<std::path::PathBuf>,
    record_fault: Option<(&'static str, std::path::PathBuf)>,
}

#[path = "governed_contract/intent_fixture.rs"]
mod intent_fixture;
#[path = "governed_contract/schema.rs"]
mod schema;
#[path = "governed_contract/state.rs"]
mod state;
use schema::*;
#[path = "governed_contract/server.rs"]
mod server;
use server::*;
#[path = "governed_contract/runtime.rs"]
mod runtime;
use runtime::*;
#[path = "governed_contract/like_fixture.rs"]
mod like_fixture;
use like_fixture::*;
#[path = "governed_contract/agent_cases.rs"]
mod agent_cases;
#[path = "governed_contract/catalog_cases.rs"]
mod catalog_cases;
#[path = "governed_contract/job_cases.rs"]
mod job_cases;
#[path = "governed_contract/logout_cases.rs"]
mod logout_cases;
#[path = "governed_contract/profile_cases.rs"]
mod profile_cases;
#[path = "governed_contract/read_cases.rs"]
mod read_cases;
#[path = "governed_contract/refresh_cases.rs"]
mod refresh_cases;
#[path = "governed_contract/task_plan_cases.rs"]
mod task_plan_cases;
#[path = "governed_contract/wait_cases.rs"]
mod wait_cases;
#[path = "governed_contract/write_cases.rs"]
mod write_cases;

#[path = "governed_contract/task_execution_cases.rs"]
mod task_execution_cases;

#[cfg(unix)]
#[path = "governed_contract/task_process_cases.rs"]
mod task_process_cases;

#[path = "governed_contract/wait_boundaries.rs"]
mod wait_boundaries;

#[path = "governed_contract/task_compare_failures.rs"]
mod task_compare_failures;
