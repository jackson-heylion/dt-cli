use super::{context, invocation, jobs, model::SessionPurpose, read_profile, writes};
use crate::{
    Runtime,
    output::{Result, invalid},
    profile,
};
pub(super) async fn execute(
    rt: &Runtime,
    kind: &str,
    options: &crate::interface::request::Record,
) -> Result<serde_json::Value> {
    let action = options.action;
    if action == "list" {
        let page = options.page;
        return if kind == "jobs" {
            jobs::list(rt, page).await
        } else {
            writes::list(rt, page).await
        };
    }
    if kind == "jobs" {
        match action {
            "status" => return jobs::status(rt, &options.id).await,
            "results" => {
                return jobs::result(rt, &options.id, options.page).await;
            }
            _ => {}
        }
    } else if action == "inspect" {
        return writes::inspect(rt, &options.id).await;
    }
    let name = context::selected(rt, options.profile.as_deref(), SessionPurpose::Cli)?;
    profile::validate_name(&name)?;
    let stored = read_profile(rt, &name)?;
    if kind == "jobs" && action == "resume" {
        return jobs::resume(rt, &stored, &options.id).await;
    }
    if kind == "writes" {
        match action {
            "status" => return writes::status(rt, &stored, &options.id).await,
            "confirm" => {
                return writes::confirm(rt, &stored, &options.id, &options.fingerprint).await;
            }
            "dispatch" => return writes::dispatch(rt, &stored, &options.id).await,
            _ => {}
        }
    }
    let selected = super::discovery::selected(rt, &stored, &options.target).await?;
    let version = selected.contract.operation_version;
    let operation = &selected.contract.operation_id;
    let input = invocation::input_file(&options.files.input, options.files.body.as_deref())?;
    if kind == "jobs" && action == "start" {
        jobs::start(rt, &name, &stored, operation, version, input).await
    } else if kind == "writes" && action == "prepare" {
        writes::prepare(
            rt,
            &name,
            &stored,
            operation,
            version,
            input,
            &options.instruction,
        )
        .await
    } else {
        Err(invalid())
    }
}
