//! Reviewed task recipes over existing executors. Discovery and planning are purely local.
mod canonical;
mod compare;
mod definitions;
mod execution;
mod inbox;
mod journal;
mod planning;

pub(crate) fn system_for(id: &str) -> Option<&'static str> {
    definitions::get(id)
        .ok()
        .and_then(|d| d.system_id.as_deref())
}
/// Keep the task envelope informative even when validation fails before a run exists.
pub(crate) fn annotate(value: &mut Value, id: Option<&str>) {
    let task = id
        .or_else(|| value["data"]["taskId"].as_str())
        .and_then(|id| definitions::get(id).ok());
    if let Some(task) = task {
        value["meta"]["taskId"] = serde_json::json!(task.task_id);
        value["meta"]["taskSchemaVersion"] = serde_json::json!(task.task_schema_version);
        value["meta"]["recipeVersion"] = serde_json::json!(task.recipe_version);
        value["meta"]["effect"] = serde_json::json!(task.effect);
    }
    if value["meta"]["complete"].is_null() {
        value["meta"]["complete"] = serde_json::json!(value["ok"] == true);
    }
    if value["meta"]["observedAt"].is_null() {
        value["meta"]["observedAt"] = serde_json::json!(chrono::Utc::now().to_rfc3339());
    }
    if value["meta"]["sources"].is_null() {
        value["meta"]["sources"] = serde_json::json!([]);
    }
    if task.is_some() && value["meta"]["availability"].is_null() {
        let code = value["error"]["code"].as_str().unwrap_or("");
        let state = match code {
            "CLIENT_UPGRADE_REQUIRED" | "TASK_CONTRACT_UNSUPPORTED" => "unsupported",
            "PROFILE_REQUIRED"
            | "PROFILE_NOT_CONFIGURED"
            | "PROFILE_SELECTION_MISMATCH"
            | "PROFILE_PROVIDER_MISMATCH"
            | "AUTH_REQUIRED"
            | "AUTHORIZATION_EXPIRED"
            | "AUTHORIZATION_REVOKED"
            | "SCOPE_DENIED"
            | "DATA_SCOPE_DENIED"
            | "GRANT_EXPIRED"
            | "CONTRACT_CHANGED"
            | "VERSION_REQUIRED"
            | "UNKNOWN_OPERATION" => "blocked",
            _ => "unknown",
        };
        value["meta"]["availability"] = if value["data"]["availability"].is_object() {
            value["data"]["availability"].clone()
        } else {
            serde_json::json!({"state":state,"reason":if code.is_empty(){"offline-not-verified"}else{code}})
        };
    }
}
use crate::{
    Runtime,
    catalog::{Operation, string},
    output::{Result, invalid},
};
use clap::ArgMatches;
use serde_json::Value;

pub(crate) async fn dispatch(
    rt: &Runtime,
    operation: &Operation,
    args: &ArgMatches,
) -> Result<Value> {
    match operation.operation_id.as_str() {
        "tasks.list" => planning::list(rt, string(args, "query"), string(args, "profile")),
        "tasks.show" => planning::show(
            rt,
            string(args, "task").ok_or_else(invalid)?,
            string(args, "profile"),
            string(args, "version"),
        ),
        "tasks.plan" => {
            let params = crate::input::read_parameters(args)?.unwrap_or_else(|| "{}".into());
            planning::plan(
                rt,
                string(args, "task").ok_or_else(invalid)?,
                string(args, "profile"),
                string(args, "version"),
                &params,
            )
        }
        "tasks.status" => execution::status(rt, string(args, "run").ok_or_else(invalid)?).await,
        "tasks.run" => {
            let id = string(args, "task").ok_or_else(invalid)?;
            let task = definitions::get(id)?;
            let raw = crate::input::read_parameters(args)?.unwrap_or_else(|| "{}".into());
            if crate::catalog::flag(args, "dry-run")
                || (task.effect == "write" && !crate::catalog::flag(args, "execute"))
            {
                if crate::catalog::flag(args, "dry-run") && crate::catalog::flag(args, "execute") {
                    return Err(invalid());
                }
                return planning::plan(
                    rt,
                    id,
                    string(args, "profile"),
                    string(args, "version"),
                    &raw,
                );
            }
            if crate::catalog::flag(args, "execute") && task.effect != "write" {
                return Err(invalid());
            }
            let params = task.validate(&raw)?;
            if id == "like.send"
                && (string(args, "version").is_none() || string(args, "params-file").is_none())
            {
                return Err(invalid());
            }
            if id == "inbox.brief" && string(args, "run-id").is_some() {
                return Err(invalid());
            }
            let binding = crate::profiles::resolve(
                rt,
                string(args, "profile"),
                &task.provider,
                task.system_id.as_deref(),
                task.effect == "write",
            )?;
            match id {
                "inbox.brief" => inbox::run(rt, &binding, &params).await,
                _ => {
                    execution::run(
                        rt,
                        id,
                        binding,
                        string(args, "version"),
                        &params,
                        string(args, "run-id"),
                    )
                    .await
                }
            }
        }
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests;
