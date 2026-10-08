//! Durable receipts contain only binding, digests and opaque IDs; no business payloads.
mod lock;
use super::canonical;
use crate::{
    Runtime,
    output::{Failure, Result},
    private_store,
    profiles::Binding,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use std::{
    fs::{self, File},
    path::PathBuf,
};
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub(super) struct Record {
    pub schema_version: u8,
    pub run_id: String,
    pub task_id: String,
    pub recipe_version: u64,
    pub binding: Binding,
    pub operation_id: String,
    pub operation_version: String,
    pub contract_digest: String,
    pub request_digest: String,
    pub arguments_digest: String,
    pub idempotency_key: Option<String>,
    pub remote_id: Option<String>,
    pub stage: String,
    pub created_at: DateTime<Utc>,
    pub observed_at: DateTime<Utc>,
    pub confirmed_terminal_at: Option<DateTime<Utc>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Sealed {
    record: Record,
    signature: String,
}

pub(super) struct Journal {
    path: PathBuf,
    key: Zeroizing<Vec<u8>>,
    _lock: File,
    last_trace: Option<String>,
    pub record: Record,
}
fn path(rt: &Runtime, id: &str) -> Result<PathBuf> {
    if id.len() != 43
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(crate::output::invalid());
    }
    Ok(rt.root.join("workpacks").join(format!("{id}.json")))
}
fn mac(key: &[u8], record: &Record) -> Result<Hmac<Sha256>> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).map_err(|_| private_store::unavailable())?;
    mac.update(&serde_json::to_vec(record).map_err(|_| private_store::unavailable())?);
    Ok(mac)
}
fn verify(key: &[u8], sealed: &Sealed) -> Result<()> {
    let bytes = URL_SAFE_NO_PAD
        .decode(&sealed.signature)
        .map_err(|_| private_store::unavailable())?;
    mac(key, &sealed.record)?
        .verify_slice(&bytes)
        .map_err(|_| private_store::unavailable())?;
    let r = &sealed.record;
    if r.schema_version != 1
        || r.recipe_version != 1
        || r.observed_at < r.created_at
        || !matches!(r.task_id.as_str(), "like.send" | "order-config.compare")
        || !matches!(
            r.stage.as_str(),
            "created"
                | "prepare-attempted"
                | "prepared"
                | "authorize-attempted"
                | "approved"
                | "dispatch-attempted"
                | "submit-attempted"
                | "submitted"
                | "observed"
        )
    {
        return Err(private_store::unavailable());
    }
    Ok(())
}
impl Journal {
    pub async fn create(
        rt: &Runtime,
        task: &str,
        binding: Binding,
        contract: &Value,
        params: &Value,
        api_params: &Value,
    ) -> Result<Self> {
        let id = crate::login::random();
        let path = path(rt, &id)?;
        let key = lock::key(rt, true).await?;
        cleanup(rt, &key)?;
        let guard = lock::acquire(&path.with_extension("lock")).await?;
        if private_store::read::<Sealed>(&path)?.is_some() {
            return Err(private_store::unavailable());
        }
        let now = Utc::now();
        let text = |field| {
            contract[field]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(crate::http::protocol)
        };
        let record = Record {
            schema_version: 1,
            run_id: id,
            task_id: task.into(),
            recipe_version: 1,
            binding,
            operation_id: text("operationId")?,
            operation_version: text("operationVersion")?,
            contract_digest: text("contractDigest")?,
            request_digest: canonical::digest(params)?,
            arguments_digest: canonical::digest(api_params)?,
            idempotency_key: (task == "like.send").then(crate::login::random),
            remote_id: None,
            stage: if task == "like.send" {
                "prepare-attempted"
            } else {
                "submit-attempted"
            }
            .into(),
            created_at: now,
            observed_at: now,
            confirmed_terminal_at: None,
        };
        let journal = Self {
            path,
            key,
            _lock: guard,
            last_trace: None,
            record,
        };
        journal.save()?;
        Ok(journal)
    }
    pub async fn load(rt: &Runtime, id: &str) -> Result<Self> {
        let path = path(rt, id)?;
        if private_store::read::<Sealed>(&path)?.is_none() {
            return Err(Failure::new(
                "RUN_NOT_FOUND",
                6,
                "执行记录不存在；请使用原 jobId 或 intentId 查询。",
            ));
        }
        let key = lock::key(rt, false).await?;
        let guard = lock::acquire(&path.with_extension("lock")).await?;
        let sealed = private_store::read::<Sealed>(&path)?.ok_or_else(|| {
            Failure::new(
                "RUN_NOT_FOUND",
                6,
                "执行记录不存在；请使用原 jobId 或 intentId 查询。",
            )
        })?;
        verify(&key, &sealed)?;
        if sealed.record.run_id != id {
            return Err(private_store::unavailable());
        }
        if crate::profiles::binding(rt, &sealed.record.binding.profile)? != sealed.record.binding {
            return Err(Failure::new(
                "RUN_BINDING_CHANGED",
                4,
                "当前授权与原执行记录不一致；未查询或派发。",
            ));
        }
        let journal = Self {
            path,
            key,
            _lock: guard,
            last_trace: None,
            record: sealed.record,
        };
        Ok(journal)
    }
    pub fn params_match(&self, params: &Value) -> Result<()> {
        if canonical::digest(params)? != self.record.request_digest {
            return Err(Failure::new(
                "RUN_ARGUMENTS_CHANGED",
                6,
                "参数与原执行记录不一致；未继续执行。",
            ));
        }
        Ok(())
    }
    pub fn step(&mut self, stage: &str) -> Result<()> {
        self.record.stage = stage.into();
        self.record.observed_at = Utc::now();
        self.save()
    }
    pub fn attach(&mut self, remote: &Value) -> Result<()> {
        self.last_trace = trace(remote);
        let field = if self.record.task_id == "like.send" {
            "intentId"
        } else {
            "jobId"
        };
        let id = remote[field]
            .as_str()
            .filter(|id| {
                id.len() == 43
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            })
            .ok_or_else(crate::http::protocol)?;
        if self.record.remote_id.as_ref().is_some_and(|old| old != id) {
            return Err(crate::http::protocol());
        }
        self.record.remote_id = Some(id.into());
        // One atomic observation binds the handle and completed preparation/submission.
        // Persist even malformed provenance so its original handle is never lost.
        self.record.stage = if self.record.task_id == "like.send" {
            "prepared"
        } else {
            "submitted"
        }
        .into();
        self.record.observed_at = Utc::now();
        self.save()?;
        self.check_remote(remote)
    }
    pub fn check_remote(&self, remote: &Value) -> Result<()> {
        if remote["operationId"] != self.record.operation_id
            || remote["operationVersion"] != self.record.operation_version
            || remote["contractDigest"] != self.record.contract_digest
            || remote["argumentsDigest"] != self.record.arguments_digest
        {
            return Err(Failure::new(
                "RUN_PROVENANCE_MISMATCH",
                6,
                "服务端记录与原执行绑定不一致；未继续执行。",
            ));
        }
        Ok(())
    }
    pub fn observe(&mut self, remote: &Value) -> Result<()> {
        self.last_trace = trace(remote);
        self.check_remote(remote)?;
        let terminal = if self.record.task_id == "like.send" {
            matches!(
                remote["state"].as_str(),
                Some("succeeded" | "rejected" | "expired" | "cancelled")
            ) && matches!(remote["sideEffect"].as_str(), Some("none" | "confirmed"))
        } else {
            matches!(
                remote["state"].as_str(),
                Some("succeeded" | "failed" | "cancelled")
            )
        };
        if terminal && self.record.remote_id.is_some() {
            self.record.confirmed_terminal_at.get_or_insert(Utc::now());
        }
        if !terminal {
            self.record.confirmed_terminal_at = None;
        }
        self.step("observed")
    }
    pub fn recovery(&self, mut failure: Failure) -> Failure {
        if failure.trace_id.is_none() {
            failure.trace_id = self.last_trace.clone();
        }
        let r = failure.recovery.get_or_insert_with(|| Box::new(json!({})));
        r["runId"] = json!(self.record.run_id);
        if let Some(id) = &self.record.remote_id {
            r[if self.record.task_id == "like.send" {
                "intentId"
            } else {
                "jobId"
            }] = json!(id);
        }
        failure.retryable = false;
        failure
    }
    fn save(&self) -> Result<()> {
        let signature =
            URL_SAFE_NO_PAD.encode(mac(&self.key, &self.record)?.finalize().into_bytes());
        private_store::write(
            &self.path,
            &Sealed {
                record: self.record.clone(),
                signature,
            },
        )
    }
}
fn trace(remote: &Value) -> Option<String> {
    remote["_meta"]["traceId"]
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
        .map(str::to_owned)
}
fn cleanup(rt: &Runtime, key: &[u8]) -> Result<()> {
    for entry in
        fs::read_dir(rt.root.join("workpacks")).map_err(|_| private_store::unavailable())?
    {
        let entry = entry.map_err(|_| private_store::unavailable())?;
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if super::journal::path(rt, id).is_err() {
            continue;
        }
        // Ignore damaged records during cleanup; never remove unresolved or unverified state.
        if let Ok(Some(sealed)) = private_store::read::<Sealed>(&path)
            && verify(key, &sealed).is_ok()
            && sealed.record.remote_id.is_some()
            && sealed
                .record
                .confirmed_terminal_at
                .is_some_and(|at| Utc::now() - at >= chrono::Duration::hours(24))
        {
            let lock_path = path.with_extension("lock");
            if private_store::validate(&lock_path, false).is_err() {
                continue;
            }
            let guard = std::fs::OpenOptions::new().read(true).open(lock_path);
            if let Ok(guard) = guard
                && fs2::FileExt::try_lock_exclusive(&guard).is_ok()
                && let Ok(Some(current)) = private_store::read::<Sealed>(&path)
                && verify(key, &current).is_ok()
                && current.record.remote_id.is_some()
                && current
                    .record
                    .confirmed_terminal_at
                    .is_some_and(|at| Utc::now() - at >= chrono::Duration::hours(24))
            {
                let _ = fs::remove_file(&path);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
