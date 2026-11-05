use super::{context, invocation, jobs, model::SessionPurpose, read_profile, writes};
use crate::{
    Runtime,
    output::{Result, invalid},
    profile,
};
use clap::ArgMatches;
fn text<'a>(options: &'a ArgMatches, key: &str) -> Result<&'a str> {
    options
        .get_one::<String>(key)
        .map(String::as_str)
        .ok_or_else(invalid)
}
pub(super) async fn execute(
    rt: &Runtime,
    kind: &str,
    arguments: &ArgMatches,
) -> Result<serde_json::Value> {
    let (action, options) = arguments.subcommand().ok_or_else(invalid)?;
    if action == "list" {
        let page = text(options, "page")?.parse().map_err(|_| invalid())?;
        return if kind == "jobs" {
            jobs::list(rt, page).await
        } else {
            writes::list(rt, page).await
        };
    }
    if kind == "jobs" {
        match action {
            "status" => return jobs::status(rt, text(options, "id")?).await,
            "results" => {
                return jobs::result(
                    rt,
                    text(options, "id")?,
                    text(options, "page")?.parse().map_err(|_| invalid())?,
                )
                .await;
            }
            _ => {}
        }
    } else if action == "inspect" {
        return writes::inspect(rt, text(options, "id")?).await;
    }
    let name = context::selected(rt, options, SessionPurpose::Cli)?;
    profile::validate_name(&name)?;
    let stored = read_profile(rt, &name)?;
    if kind == "jobs" && action == "resume" {
        return jobs::resume(rt, &stored, text(options, "id")?).await;
    }
    if kind == "writes" {
        match action {
            "status" => return writes::status(rt, &stored, text(options, "id")?).await,
            "confirm" => {
                return writes::confirm(
                    rt,
                    &stored,
                    text(options, "id")?,
                    text(options, "fingerprint")?,
                )
                .await;
            }
            "dispatch" => return writes::dispatch(rt, &stored, text(options, "id")?).await,
            _ => {}
        }
    }
    let selected = super::discovery::selected(rt, &stored, options).await?;
    let version = selected.contract.operation_version;
    let operation = &selected.contract.operation_id;
    let input = invocation::input_file(
        text(options, "input-file")?,
        options.get_one::<String>("body-file").map(String::as_str),
    )?;
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
            text(options, "instruction")?,
        )
        .await
    } else {
        Err(invalid())
    }
}
