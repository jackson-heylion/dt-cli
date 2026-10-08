//! Governed system access: an independent provider with its own profile, catalog cache and
//! credentials. Personal-workflow profiles, tokens and commands never cross into this module.
use crate::{
    Runtime,
    catalog::{Catalog, Operation, flag, string},
    credentials::Credentials,
    http, login,
    output::{Failure, Result, invalid, storage},
    profile,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use clap::ArgMatches;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

const PROVIDER: &str = "governed";
const CLIENT_ID: &str = "dt-cli";
/// The only execution-contract structure this CLI understands.
const CONTRACT_SCHEMA_VERSION: u64 = 1;
const AUTHORIZE_ROUTE: &str = "/cli-api/oauth2/authorize";
const TOKEN_ROUTE: &str = "/cli-api/oauth2/token";
const EXCHANGE_ROUTE: &str = "/cli-api/oauth2/exchange";
const REVOKE_ROUTE: &str = "/cli-api/oauth2/revoke";
const CATALOG_ROUTE: &str = "/cli-api/v1/catalog";
const JOBS_ROUTE: &str = "/cli-api/v1/jobs";
const INTENTS_ROUTE: &str = "/cli-api/v1/intents";
const INTENT_PAGE: &str = "/cli-governed-intent.html?intentId=";
const TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
const ACCESS_TOKEN_TYPE: &str = "urn:ietf:params:oauth:token-type:access_token";
const BODY_LIMIT: usize = 2 * 1024 * 1024;
/// A complete batch result may hold up to 10 MiB of items (spec §2.2); the envelope adds a little.
const RESULT_BODY_LIMIT: usize = 10 * 1024 * 1024 + 64 * 1024;
/// IAM re-checks the current store scope (up to 30 s) before it sends a result, then streams it.
const RESULT_TIMEOUT: Duration = Duration::from_secs(90);
const PARAMS_LIMIT: usize = 65_536;
/// Server meta fields passed through to the envelope; anything else is dropped.
const META_FIELDS: [&str; 11] = [
    "contractVersion",
    "contractDigest",
    "traceId",
    "invocationId",
    "state",
    "sideEffect",
    "asOf",
    "complete",
    "nextPage",
    "itemsReturned",
    "recovery",
];

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct GovernedProfile {
    provider: String,
    issuer: String,
    environment: String,
    system_id: String,
    subject_id: String,
    authorization_id: String,
    authorization_expires_at: String,
    access_expires_at: String,
    credential_key: String,
}

/// The offline catalog, bound to exactly one provider, issuer, employee, system,
/// environment and parent authorization; each operation carries its contract digest.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Cache {
    provider: String,
    issuer: String,
    environment: String,
    system_id: String,
    subject_id: String,
    authorization_id: String,
    catalog_revision: String,
    cached_at: String,
    operations: Vec<Value>,
    /// Compiled once when the cache is validated; never serialized or shared across profiles.
    #[serde(skip)]
    validators: Vec<Option<ContractValidators>>,
}

#[derive(Clone)]
struct ContractValidators {
    input: std::sync::Arc<jsonschema::Validator>,
    output: std::sync::Arc<jsonschema::Validator>,
}

impl Cache {
    fn validators(&self, operation: &Value) -> Result<&ContractValidators> {
        self.operations
            .iter()
            .position(|item| {
                item["operationId"] == operation["operationId"]
                    && item["operationVersion"] == operation["operationVersion"]
            })
            .and_then(|index| self.validators.get(index))
            .and_then(Option::as_ref)
            .ok_or_else(catalog_invalid)
    }
}

fn not_configured() -> Failure {
    Failure::new("PROFILE_NOT_CONFIGURED", 2, "受控 profile 尚未配置。")
}
fn provider_mismatch() -> Failure {
    Failure::new(
        "PROFILE_PROVIDER_MISMATCH",
        2,
        "此 profile 属于另一类授权；受控访问与个人流程命令不能混用。",
    )
}
fn profile_invalid() -> Failure {
    Failure::new(
        "PROFILE_INVALID",
        2,
        "受控 profile 的身份绑定无效；未读取凭证。",
    )
}
fn catalog_invalid() -> Failure {
    Failure::new(
        "CATALOG_INVALID",
        2,
        "受控目录与当前 profile 或合同不兼容。",
    )
}
fn not_synced() -> Failure {
    Failure::new(
        "CATALOG_NOT_SYNCED",
        6,
        "请先执行 catalog sync 同步受控目录。",
    )
}
fn upgrade_required() -> Failure {
    Failure::new(
        "CLIENT_UPGRADE_REQUIRED",
        6,
        "该操作合同需要更新版本的 dt-cli；未执行。",
    )
}
fn contract_changed() -> Failure {
    Failure::new(
        "CONTRACT_CHANGED",
        6,
        "操作合同已变化，请重新同步目录并取得同意；未执行。",
    )
}
fn auth_required() -> Failure {
    Failure::new("AUTH_REQUIRED", 4, "受控访问授权已失效，请重新登录。")
}
fn revoked() -> Failure {
    Failure::new("AUTHORIZATION_REVOKED", 4, "受控访问授权已撤销。")
}

fn identity_mismatch() -> Failure {
    Failure::new(
        "IDENTITY_MISMATCH",
        4,
        "在线授权与本地受控 profile 不一致。",
    )
}

mod storage;
pub use storage::profile_exists;

pub(crate) fn binding(rt: &Runtime, name: &str) -> Result<crate::profiles::Binding> {
    let p = require_profile(rt, name)?;
    if p.issuer != rt.environment(&p.environment)?.api_origin {
        return Err(profile_invalid());
    }
    Ok(crate::profiles::Binding {
        profile: name.into(),
        provider: PROVIDER.into(),
        issuer: p.issuer,
        environment: p.environment,
        system_id: Some(p.system_id),
        subject_id: p.subject_id,
        authorization_id: p.authorization_id,
        authorization_expires_at: p.authorization_expires_at,
    })
}

pub(crate) async fn setup(
    rt: &Runtime,
    name: &str,
    environment: &str,
    system: &str,
) -> Result<Value> {
    login_profile(rt, name, Some(environment), Some(system)).await
}
use storage::*;

/// Local contract metadata never grants execution authority.
pub(crate) fn task_contract(
    rt: &Runtime,
    name: &str,
    id: &str,
    version: Option<&str>,
    effect: &str,
    params: Option<&Value>,
) -> Result<Value> {
    let p = require_profile(rt, name)?;
    let cache = read_cache(&rt.root, name, &p)?.ok_or_else(not_synced)?;
    let operation = select(&cache, id, version)?;
    usable(operation, effect)?;
    if effect == "write" && operation["confirmation"]["channel"] != "agent-cli" {
        return Err(Failure::new(
            "TASK_CONTRACT_UNSUPPORTED",
            6,
            "该任务需要agent-cli确认渠道，当前合同不支持。",
        ));
    }
    if let Some(params) = params {
        validate_params(&params.to_string(), &cache.validators(operation)?.input)?;
    }
    Ok(operation.clone())
}
mod catalog;
use catalog::*;
mod wire;
use wire::*;
mod session;
use session::*;
mod consent;
use consent::*;
mod read;
use read::*;
mod wait;
use wait::*;
mod intents;
pub(crate) mod tasks;
pub(crate) use intents::require_current_intent;
use intents::*;

fn project_meta(meta: &Value) -> Value {
    let mut projected = Map::new();
    for key in META_FIELDS {
        if let Some(value) = meta.get(key) {
            projected.insert(key.into(), value.clone());
        }
    }
    if let Some(trace) = trace(&json!({"traceId": meta["traceId"]})) {
        projected.insert("traceId".into(), json!(trace));
    } else {
        projected.remove("traceId");
    }
    Value::Object(projected)
}
fn job_id(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
pub async fn dispatch(rt: &Runtime, op: &Operation, leaf: &ArgMatches) -> Result<Value> {
    let id = op.operation_id.as_str();
    if flag(leaf, "help") {
        return Catalog::shared().schema(id);
    }
    let name = string(leaf, "profile").ok_or_else(invalid)?;
    profile::validate_name(name)?;
    if flag(leaf, "dry-run") && id != "api.call" && id != "intents.prepare" {
        return Ok(json!({
            "preview": true,
            "provider": PROVIDER,
            "operationId": id,
            "effect": op.effect,
            "httpRequests": 0,
            "credentialReads": 0,
            "browserOpened": false,
        }));
    }
    match id {
        "auth.login" => {
            login_profile(
                rt,
                name,
                string(leaf, "environment"),
                string(leaf, "system"),
            )
            .await
        }
        "auth.status" => status(rt, name),
        "auth.logout" => logout(rt, name).await,
        "auth.check" | "iam.profile.get" => check(rt, name).await,
        "doctor" if flag(leaf, "online") => check(rt, name).await,
        "doctor" => {
            require_profile(rt, name)?;
            Ok(json!({"provider":PROVIDER,"onlineVerified":false,"credentialStore":"not-probed"}))
        }
        "catalog.sync" => sync(rt, name).await,
        "discover" => discover(rt, name, string(leaf, "query"), string(leaf, "limit")),
        "schema" => schema(
            rt,
            name,
            string(leaf, "operation").ok_or_else(invalid)?,
            string(leaf, "version"),
        ),
        "api.call" | "jobs.submit" => {
            let raw = crate::input::read_parameters(leaf)?.unwrap_or_else(|| "{}".to_owned());
            call_operation(
                rt,
                name,
                string(leaf, "operation").ok_or_else(invalid)?,
                string(leaf, "version"),
                &raw,
                id == "jobs.submit",
                flag(leaf, "dry-run"),
            )
            .await
        }
        "jobs.status" | "jobs.result" | "jobs.cancel" => {
            job_action(rt, name, id, string(leaf, "job-id").ok_or_else(invalid)?).await
        }
        "intents.prepare" => {
            let raw = crate::input::read_parameters(leaf)?.unwrap_or_else(|| "{}".to_owned());
            let view = prepare_intent(
                rt,
                name,
                string(leaf, "operation").ok_or_else(invalid)?,
                string(leaf, "version"),
                &raw,
                string(leaf, "idempotency-key"),
                flag(leaf, "dry-run"),
            )
            .await?;
            if flag(leaf, "wait") && !flag(leaf, "dry-run") {
                wait_and_invoke(rt, name, view).await
            } else {
                Ok(view)
            }
        }
        "intents.authorize" => {
            authorize_intent(
                rt,
                name,
                string(leaf, "intent-id").ok_or_else(invalid)?,
                string(leaf, "arguments-digest").ok_or_else(invalid)?,
                string(leaf, "contract-digest").ok_or_else(invalid)?,
            )
            .await
        }
        "jobs.wait" => {
            wait_job(
                rt,
                name,
                string(leaf, "job-id").ok_or_else(invalid)?,
                string(leaf, "timeout")
                    .ok_or_else(invalid)?
                    .parse()
                    .map_err(|_| invalid())?,
            )
            .await
        }
        "intents.status" | "intents.cancel" => {
            intent_action(rt, name, id, string(leaf, "intent-id").ok_or_else(invalid)?).await
        }
        "intents.invoke" => {
            invoke_intent(rt, name, string(leaf, "intent-id").ok_or_else(invalid)?).await
        }
        _ => Err(provider_mismatch()),
    }
}
