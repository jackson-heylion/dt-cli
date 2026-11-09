//! 新平台独立登录入口；复用安全存储和本机回调，不接收业务凭证。
mod actions;
mod catalog;
mod commands;
mod context;
mod contract;
mod diagnostic;
mod discovery;
mod executor;
mod input_model;
mod invocation;
#[cfg(test)]
mod invocation_tests;
mod jobs;
mod login_flow;
mod management;
mod metadata;
mod model;
mod profiles;
mod records;
mod session;
mod timing;
mod transport;
mod view_memory;
mod writes;

use crate::{
    Runtime,
    credentials::Credentials,
    login,
    output::{self, Failure, Result, invalid},
    private_store, profile,
};
use model::*;
use serde_json::Value;
use tokio::net::TcpListener;

pub(crate) fn selected(args: &[String]) -> bool {
    args.get(1).is_some_and(|v| v == "platform")
}

pub(crate) async fn execute(rt: &Runtime, args: Vec<String>) -> (Value, u8, bool) {
    let (operation, result) = dispatch(rt, args).await;
    let (body, status) = output::envelope_result(operation, None, result);
    (body, status, false)
}

async fn dispatch(rt: &Runtime, args: Vec<String>) -> (&'static str, Result<Value>) {
    if args.iter().any(|value| value == "--help") {
        // 任意层级的 --help 都返回该作用域可直接复制的命令清单。
        let cleaned: Vec<String> = args
            .iter()
            .filter(|value| value.as_str() != "--help")
            .cloned()
            .collect();
        let reparsed = commands::command()
            .ignore_errors(true)
            .try_get_matches_from(
                std::iter::once("dt-cli platform".to_owned()).chain(cleaned.into_iter().skip(2)),
            );
        if let Ok(parsed) = reparsed {
            let scope = parsed
                .subcommand_name()
                .map(str::to_owned)
                .unwrap_or_else(|| "platform".to_owned());
            return (
                "platform.help",
                Ok(serde_json::json!({
                    "scope": scope,
                    "commands": help_lines(&scope)
                })),
            );
        }
    }
    let arguments = std::iter::once("dt-cli platform".to_owned()).chain(args.into_iter().skip(2));
    let matches = match commands::command().try_get_matches_from(arguments) {
        Ok(value) => value,
        Err(_) => return ("platform", Err(invalid())),
    };
    let Some((action, options)) = matches.subcommand() else {
        return ("platform", Err(invalid()));
    };
    if action == "profiles" {
        return ("platform.profiles", profiles::list(rt));
    }
    if action == "context" {
        return ("platform.context", context::execute(rt, options).await);
    }
    if action == "manage" {
        return ("platform.manage", management::execute(rt, options).await);
    }
    if action == "writes" || action == "jobs" {
        return (
            if action == "writes" {
                "platform.writes"
            } else {
                "platform.jobs"
            },
            actions::execute(rt, action, options).await,
        );
    }
    let operation = match action {
        "login" => "platform.login",
        "check" => "platform.check",
        "sessions" => "platform.sessions",
        "revoke" => "platform.revoke",
        "logout" => "platform.logout",
        "catalog" => "platform.catalog",
        "views" => "platform.views",
        "call" => "platform.call",
        "search" => "platform.search",
        "resolve" => "platform.resolve",
        "describe" => "platform.describe",
        "template" => "platform.template",
        "diagnose" => "platform.diagnose",
        _ => "platform",
    };
    let result = async {
        let name = context::selected(rt, options, SessionPurpose::Cli)?;
        profile::validate_name(&name)?;
        if action == "login" {
            return login_flow::login_platform(
                rt,
                &name,
                options.get_one::<String>("origin").unwrap(),
                options.get_one::<String>("environment").unwrap(),
                options.get_one::<String>("system").unwrap(),
                options.get_flag("no-browser"),
                SessionPurpose::Cli,
            )
            .await;
        }
        let stored = read_profile(rt, &name)?;
        match action {
            "check" => serialize(session::me(rt, &stored).await?),
            "search" | "resolve" | "describe" | "template" => {
                discovery::execute(rt, &stored, action, options).await
            }
            "diagnose" => diagnostic::execute(rt, &stored, options).await,
            "catalog" => serialize(
                catalog::list(
                    rt,
                    &stored,
                    options
                        .get_one::<String>("page")
                        .unwrap()
                        .parse()
                        .map_err(|_| invalid())?,
                )
                .await?,
            ),
            "views" | "call" => {
                let selected = discovery::selected(rt, &stored, options).await?;
                let operation = &selected.contract.operation_id;
                let version = selected.contract.operation_version;
                if action == "views" {
                    serialize(catalog::views_for(rt, &stored, &selected.contract).await?)
                } else {
                    let view = match options.get_one::<String>("view") {
                        Some(v) => Some((
                            v.parse::<u32>().map_err(|_| invalid())?,
                            options.get_one::<String>("object").unwrap().clone(),
                        )),
                        None => None,
                    };
                    if let Some(file) = options.get_one::<String>("input-file") {
                        let mut input = invocation::input_file(
                            file,
                            options.get_one::<String>("body-file").map(String::as_str),
                        )?;
                        if let Some((view_id, object_id)) = view {
                            if input.view.is_some() {
                                return Err(invalid());
                            }
                            input.view = Some(input_model::TaskView { view_id, object_id });
                        }
                        return executor::present(
                            executor::read_input(rt, &stored, operation, version, input).await?,
                        );
                    }
                    executor::present(
                        catalog::read(
                            rt,
                            &stored,
                            operation,
                            version,
                            options.get_one::<String>("path").unwrap(),
                            options.get_one::<String>("query").unwrap(),
                            view,
                        )
                        .await?,
                    )
                }
            }
            "sessions" => {
                let page = options
                    .get_one::<String>("page")
                    .unwrap()
                    .parse::<u32>()
                    .map_err(|_| invalid())?;
                if !(1..=100000).contains(&page) {
                    return Err(invalid());
                }
                serialize(session::list(rt, &stored, page).await?)
            }
            "revoke" => {
                let id = options.get_one::<String>("session").unwrap();
                if !handle(id) {
                    return Err(invalid());
                }
                session::revoke(rt, &stored, id).await?;
                Ok(serde_json::json!({"revoked":true,"sessionId":id}))
            }
            "logout" => serialize(session::logout(rt, &name, &stored).await?),
            _ => Err(invalid()),
        }
    }
    .await;
    (operation, result)
}

/// 帮助文本的单一来源:主命令与 writes/jobs 作用域返回可直接复制的命令清单。
fn help_lines(scope: &str) -> Vec<String> {
    let login = "platform login --profile <name> --origin <https-origin> --environment <env> [--no-browser]";
    let owned = |items: Vec<&str>| items.into_iter().map(str::to_owned).collect::<Vec<_>>();
    let mut lines=match scope {
        "manage" => management::help_lines(),
        "search" | "resolve" | "describe" | "template" | "diagnose" => discovery::help_lines(scope),
        "context" => owned(vec!["platform context use --profile <name> --environment <env> [--purpose business|management]", "platform context show [--purpose business|management]", "platform context clear [--purpose business|management]"]),
        "writes" => owned(vec![
            "platform writes list [--page <n>]",
            "platform writes prepare --profile <name> --operation <id> --version <n> --input-file <json> --instruction <user-instruction>",
            "platform writes inspect --id <record-id>",
            "platform writes confirm --profile <name> --id <record-id> --fingerprint <sha256>",
            "platform writes dispatch --profile <name> --id <record-id>",
            "platform writes status --profile <name> --id <record-id>",
        ]),
        "jobs" => owned(vec![
            "platform jobs list [--page <n>]",
            "platform jobs start --profile <name> --operation <id> --version <n> --input-file <json>",
            "platform jobs resume --profile <name> --id <job-id>",
            "platform jobs status --id <job-id>",
            "platform jobs results --id <job-id> [--page <n>]",
        ]),
        "login" => vec![login.to_owned()],
        "profiles" => vec!["platform profiles".to_owned()],
        "check" => vec!["platform check --profile <name>".to_owned()],
        "sessions" => vec!["platform sessions --profile <name> [--page <n>]".to_owned()],
        "revoke" => vec!["platform revoke --profile <name> --session <id>".to_owned()],
        "logout" => vec!["platform logout --profile <name>".to_owned()],
        "catalog" => vec!["platform catalog --profile <name> [--page <n>]".to_owned()],
        "views" => vec!["platform views --profile <name> --operation <id> --version <n>".to_owned()],
        "call" => vec![
            "platform call --profile <name> --operation <id> --version <n> (--path </path> [--query <query>] | --input-file <json> [--body-file <json>]) [--view <id> --object <id>]".to_owned(),
        ],
        _ => owned(vec![
            login,
            "platform profiles",
            "platform check --profile <name>",
            "platform sessions --profile <name> [--page <n>]",
            "platform revoke --profile <name> --session <id>",
            "platform logout --profile <name>",
            "platform catalog --profile <name> [--page <n>]",
            "platform views --profile <name> --operation <id> --version <n>",
            "platform call --profile <name> --operation <id> --version <n> (--path </path> [--query <query>] | --input-file <json> [--body-file <json>]) [--view <id> --object <id>]",
            "platform jobs list [--page <n>]",
            "platform jobs start --profile <name> --operation <id> --version <n> --input-file <json>",
            "platform jobs resume --profile <name> --id <job-id>",
            "platform jobs status --id <job-id>",
            "platform jobs results --id <job-id> [--page <n>]",
            "platform writes list [--page <n>]",
            "platform writes prepare --profile <name> --operation <id> --version <n> --input-file <json> --instruction <user-instruction>",
            "platform writes inspect --id <record-id>",
            "platform writes confirm --profile <name> --id <record-id> --fingerprint <sha256>",
            "platform writes dispatch --profile <name> --id <record-id>",
            "platform writes status --profile <name> --id <record-id>",
        ]),
    }
    ;
    if scope == "platform" {
        lines.extend(management::help_lines());
        lines.extend(discovery::help_lines("all"));
        lines.extend(help_lines("context"));
    }
    lines
}

fn serialize<T: serde::Serialize>(value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|_| crate::http::protocol())
}
fn handle(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn profile_path_for(rt: &Runtime, name: &str, purpose: SessionPurpose) -> std::path::PathBuf {
    rt.root
        .join(purpose.directory())
        .join(format!("{name}.json"))
}
fn read_profile(rt: &Runtime, name: &str) -> Result<PlatformProfile> {
    read_profile_for(rt, name, SessionPurpose::Cli)
}
fn read_profile_for(rt: &Runtime, name: &str, purpose: SessionPurpose) -> Result<PlatformProfile> {
    let stored: PlatformProfile = private_store::read(&profile_path_for(rt, name, purpose))?
        .ok_or_else(|| {
            Failure::new(
                "PROFILE_NOT_CONFIGURED",
                2,
                "请先登录对应用途的开放平台会话。",
            )
        })?;
    transport::origin(&stored.origin)?;
    if stored.schema_version != 1 || !handle(&stored.session_id) || stored.purpose != purpose {
        return Err(invalid());
    }
    Ok(stored)
}
