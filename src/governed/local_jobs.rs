//! Durable local read jobs. Signed checkpoints contain no business credentials.
use super::*;
use crate::tasks::journal::lock;
use hmac::{Hmac, Mac};
use rand::RngCore;

const FILE_BYTES: u64 = RESULT_BODY_LIMIT as u64
    + BODY_LIMIT as u64
    + crate::constants::query::DIRECT_MAX_PAGES as u64
        * crate::constants::query::DIRECT_CURSOR_BYTES as u64
        * 2
    + crate::constants::query::ARGUMENT_BYTES as u64 * 4;

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum JobState {
    Queued,
    Running,
    Paused,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Record {
    schema_version: u8,
    job_id: String,
    issuer: String,
    subject_id: String,
    environment: String,
    system_id: String,
    operation: Value,
    params: Value,
    progress: direct::Progress,
    state: JobState,
    failure_code: Option<String>,
    created_at: String,
    completed_at: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Sealed {
    record: Record,
    signature: String,
}

pub(super) fn is_local(id: &str) -> bool {
    id.starts_with(crate::constants::protocol::LOCAL_JOB_PREFIX)
        && id.len()
            == crate::constants::protocol::LOCAL_JOB_PREFIX.len()
                + crate::constants::identity::HANDLE_LENGTH
}
pub(super) fn valid(id: &str) -> bool {
    is_local(id)
        && id.len()
            == crate::constants::protocol::LOCAL_JOB_PREFIX.len()
                + crate::constants::identity::HANDLE_LENGTH
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
fn path(rt: &Runtime, id: &str) -> Result<PathBuf> {
    if !valid(id) {
        return Err(invalid());
    }
    Ok(directory(&rt.root)
        .join("local-jobs")
        .join(format!("{id}.json")))
}
fn signature(record: &Record, key: &[u8]) -> Result<String> {
    let bytes = serde_json::to_vec(record).map_err(|_| storage())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| storage())?;
    mac.update(b"dt-cli-direct-job-v1\0");
    mac.update(&bytes);
    Ok(URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()))
}
fn save(rt: &Runtime, record: &Record, key: &[u8]) -> Result<()> {
    let sealed = Sealed {
        record: record.clone(),
        signature: signature(record, key)?,
    };
    crate::private_store::write_private_result(&path(rt, &record.job_id)?, &sealed, FILE_BYTES)
}
fn read(rt: &Runtime, p: &GovernedProfile, id: &str, key: &[u8]) -> Result<Record> {
    let sealed: Sealed = crate::private_store::read_private_result(&path(rt, id)?, FILE_BYTES)?
        .ok_or_else(|| {
            Failure::new(
                "RESOURCE_UNAVAILABLE",
                6,
                "未找到本机任务；保留原编号核对。",
            )
        })?;
    let expected = URL_SAFE_NO_PAD
        .decode(&sealed.signature)
        .map_err(|_| storage())?;
    let bytes = serde_json::to_vec(&sealed.record).map_err(|_| storage())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| storage())?;
    mac.update(b"dt-cli-direct-job-v1\0");
    mac.update(&bytes);
    mac.verify_slice(&expected).map_err(|_| storage())?;
    let record = sealed.record;
    if record.schema_version != 1
        || record.job_id != id
        || record.issuer != p.issuer
        || record.subject_id != p.subject_id
        || record.environment != p.environment
        || record.system_id != p.system_id
    {
        return Err(identity_mismatch());
    }
    Ok(record)
}
fn view(record: &Record) -> Value {
    json!({"jobId":record.job_id, "state":record.state, "complete":record.state==JobState::Succeeded,
        "operationId":record.operation["operationId"], "operationVersion":record.operation["operationVersion"],
        "executionLocation":"local", "createdAt":record.created_at, "completedAt":record.completed_at,
        "pageCount":record.progress.pages, "itemCount":record.progress.rows.len(), "failureCode":record.failure_code,
        "taskView":record.progress.task_view})
}

pub(super) async fn submit(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    operation: &Value,
    params: &Value,
) -> Result<Value> {
    direct::Plan::parse(operation, &p.environment)?;
    let cache = live_catalog(rt, p, env, client).await?;
    let current = select(
        &cache,
        operation["operationId"]
            .as_str()
            .ok_or_else(catalog_invalid)?,
        operation["operationVersion"].as_str(),
    )?;
    if current["contractDigest"] != operation["contractDigest"]
        || current["direct"] != operation["direct"]
    {
        return Err(contract_changed());
    }
    usable(current, "read")?;
    let mut random = [0u8; crate::constants::identity::RANDOM_BYTES];
    rand::rngs::OsRng.fill_bytes(&mut random);
    let id = format!(
        "{}{}",
        crate::constants::protocol::LOCAL_JOB_PREFIX,
        URL_SAFE_NO_PAD.encode(random)
    );
    let record = Record {
        schema_version: 1,
        job_id: id,
        issuer: p.issuer.clone(),
        subject_id: p.subject_id.clone(),
        environment: p.environment.clone(),
        system_id: p.system_id.clone(),
        operation: operation.clone(),
        params: params.clone(),
        progress: direct::Progress::default(),
        state: JobState::Queued,
        failure_code: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        completed_at: None,
    };
    let key = lock::key(rt, true).await?;
    save(rt, &record, &key)?;
    Ok(view(&record))
}

pub(super) async fn run(rt: &Runtime, name: &str, id: &str, seconds: u64) -> Result<Value> {
    if !valid(id) || !(1..=crate::constants::query::MAX_WAIT_SECONDS).contains(&seconds) {
        return Err(invalid());
    }
    let p = require_profile(rt, name)?;
    let env = rt.environment(&p.environment)?;
    let client = http::client()?;
    let file = path(rt, id)?;
    let _guard = lock::acquire(&file.with_extension("lock")).await?;
    let key = lock::key(rt, false).await?;
    let mut record = read(rt, &p, id, &key)?;
    if record.state == JobState::Cancelled {
        return Err(partial_result().with_partial(view(&record), json!({"complete":false})));
    }
    let cache = live_catalog(rt, &p, env, &client).await?;
    let current = select(
        &cache,
        record.operation["operationId"]
            .as_str()
            .ok_or_else(catalog_invalid)?,
        record.operation["operationVersion"].as_str(),
    )?;
    if current["contractDigest"] != record.operation["contractDigest"]
        || current["direct"] != record.operation["direct"]
    {
        return Err(contract_changed());
    }
    usable(current, "read")?;
    record.state = JobState::Running;
    record.failure_code = None;
    save(rt, &record, &key)?;
    let mut progress = record.progress.clone();
    let initial_spent = progress.spent_millis;
    let started = tokio::time::Instant::now();
    let query = direct::collect(
        rt,
        &p,
        env,
        &client,
        &cache,
        &record.operation,
        &record.params,
        &mut progress,
        |completed| {
            let mut checkpoint = record.clone();
            checkpoint.progress = completed.clone();
            save(rt, &checkpoint, &key)
        },
    );
    let observed = tokio::select! {
        result = query => result,
        _ = tokio::signal::ctrl_c() => Err(Failure::new("WAIT_CANCELLED",8,"本机任务已暂停，保存的页可继续恢复。")),
        _ = tokio::time::sleep(Duration::from_secs(seconds)) => Err(Failure::new("JOB_WAIT_TIMEOUT",7,"本机等待时间已用完；请恢复原任务。")),
    };
    progress.spent_millis = progress
        .spent_millis
        .max(initial_spent.saturating_add(started.elapsed().as_millis() as u64));
    record.progress = progress;
    match observed {
        Ok(mut result) => {
            record.state = JobState::Succeeded;
            record.completed_at = record.progress.as_of.clone();
            save(rt, &record, &key)?;
            result["jobId"] = json!(id);
            result["state"] = json!("succeeded");
            Ok(result)
        }
        Err(failure) => {
            record.state = if matches!(
                failure.code,
                "WAIT_CANCELLED"
                    | "JOB_WAIT_TIMEOUT"
                    | "NETWORK_ERROR"
                    | "DEPENDENCY_UNAVAILABLE"
                    | "RATE_LIMITED"
            ) {
                JobState::Paused
            } else {
                JobState::Failed
            };
            record.failure_code = Some(failure.code.to_owned());
            save(rt, &record, &key)?;
            Err(failure.with_partial(
                view(&record),
                json!({"complete":false,"executionLocation":"local"}),
            ))
        }
    }
}

pub(super) async fn action(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    operation: &str,
    id: &str,
) -> Result<Value> {
    if !valid(id) {
        return Err(invalid());
    }
    let key = lock::key(rt, false).await?;
    let mut record = read(rt, p, id, &key)?;
    match operation {
        "jobs.status" => {
            let guard = lock::try_acquire(&path(rt, id)?.with_extension("lock"))?;
            record = read(rt, p, id, &key)?;
            let runner_active = guard.is_none();
            let mut result = view(&record);
            result["runnerActive"] = json!(runner_active);
            if record.state == JobState::Running && !runner_active {
                result["recordedState"] = json!(record.state);
                result["state"] = json!("paused");
            }
            Ok(result)
        }
        "jobs.cancel" => {
            let file = path(rt, id)?;
            let _guard = lock::acquire(&file.with_extension("lock")).await?;
            record = read(rt, p, id, &key)?;
            if record.state != JobState::Succeeded && record.state != JobState::Cancelled {
                record.state = JobState::Cancelled;
                save(rt, &record, &key)?;
            }
            Ok(view(&record))
        }
        "jobs.result" => {
            if record.state != JobState::Succeeded || !record.progress.complete {
                return Err(partial_result().with_partial(view(&record), json!({"complete":false})));
            }
            let cache = live_catalog(rt, p, env, client).await?;
            let mut progress = record.progress;
            let mut result = direct::collect(
                rt,
                p,
                env,
                client,
                &cache,
                &record.operation,
                &record.params,
                &mut progress,
                |_| Ok(()),
            )
            .await?;
            result["jobId"] = json!(id);
            result["state"] = json!("succeeded");
            Ok(result)
        }
        _ => Err(invalid()),
    }
}
