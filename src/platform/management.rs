//! 本人平台管理命名空间；独立用途记录，不复用门户 Cookie 或员工业务凭证。
mod http;
mod model;
mod plans;
mod query_model;

use super::{
    endpoints, login_flow, model::SessionPurpose, read_profile_for, serialize, session, transport,
};
use crate::{
    Runtime,
    output::{Result, invalid},
    profile,
};
use clap::{Arg, ArgMatches, Command};

pub(super) fn command() -> Command {
    let profile_arg = || Arg::new("profile").long("profile");
    Command::new("manage")
        .disable_help_flag(true)
        .subcommand(
            Command::new("login")
                .arg(profile_arg().required(true))
                .arg(Arg::new("origin").long("origin").hide(true))
                .arg(super::commands::environment_arg())
                .arg(
                    Arg::new("no-browser")
                        .long("no-browser")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
        .subcommand(Command::new("check").arg(profile_arg()))
        .subcommand(Command::new("logout").arg(profile_arg()))
        .subcommand(Command::new("capabilities").arg(profile_arg()))
        .subcommand(
            Command::new("systems")
                .arg(profile_arg())
                .arg(Arg::new("id").long("id"))
                .arg(Arg::new("page").long("page").default_value("1")),
        )
        .subcommand(
            Command::new("drafts")
                .arg(profile_arg())
                .arg(
                    Arg::new("system")
                        .long("system")
                        .required_unless_present("id"),
                )
                .arg(Arg::new("id").long("id"))
                .arg(Arg::new("page").long("page").default_value("1")),
        )
        .subcommand(
            Command::new("versions")
                .arg(profile_arg())
                .arg(Arg::new("id").long("id").required(true))
                .arg(Arg::new("page").long("page").default_value("1")),
        )
        .subcommand(
            Command::new("grants")
                .arg(profile_arg())
                .arg(Arg::new("id").long("id").required(true))
                .arg(Arg::new("page").long("page").default_value("1")),
        )
        .subcommand(
            Command::new("audit")
                .arg(profile_arg())
                .arg(Arg::new("id").long("id").required(true))
                .arg(Arg::new("page").long("page").default_value("1")),
        )
        .subcommand(
            Command::new("plan")
                .arg(profile_arg())
                .arg(Arg::new("manifest").long("manifest").required(true)),
        )
        .subcommand(
            Command::new("inspect")
                .arg(profile_arg())
                .arg(Arg::new("id").long("id").required(true)),
        )
        .subcommand(
            Command::new("status")
                .arg(profile_arg())
                .arg(Arg::new("id").long("id").required(true)),
        )
        .subcommand(
            Command::new("apply")
                .arg(profile_arg())
                .arg(Arg::new("id").long("id").required(true))
                .arg(Arg::new("digest").long("digest").required(true))
                .arg(Arg::new("request-id").long("request-id"))
                .arg(
                    Arg::new("retry")
                        .long("retry")
                        .action(clap::ArgAction::SetTrue),
                ),
        )
}

pub(super) fn help_lines() -> Vec<String> {
    [
        "platform manage login --profile <name> [--environment prod|stg] [--no-browser]",
        "platform manage check --profile <name>",
        "platform manage logout --profile <name>",
        "platform manage capabilities --profile <name>",
        "platform manage systems --profile <name> [--id <system:env>] [--page <n>]",
        "platform manage drafts --profile <name> (--system <system:env> | --id <api:env>) [--page <n>]",
        "platform manage versions --profile <name> --id <api:env> [--page <n>]",
        "platform manage grants --profile <name> --id <api:env> [--page <n>]",
        "platform manage audit --profile <name> --id <plan-id> [--page <n>]",
        "platform manage plan --profile <name> --manifest <json>",
        "platform manage inspect --profile <name> --id <plan-id>",
        "platform manage apply --profile <name> --id <plan-id> --digest <sha256> [--request-id <id>] [--retry]",
        "platform manage status --profile <name> --id <plan-id>",
    ].into_iter().map(str::to_owned).collect()
}

pub(super) async fn execute(rt: &Runtime, arguments: &ArgMatches) -> Result<serde_json::Value> {
    let (action, options) = arguments.subcommand().ok_or_else(invalid)?;
    let text = |key| {
        options
            .get_one::<String>(key)
            .map(String::as_str)
            .ok_or_else(invalid)
    };
    let name = super::context::selected(rt, options, SessionPurpose::Management)?;
    profile::validate_name(&name)?;
    if action == "login" {
        let endpoint = endpoints::resolve(
            text("environment")?,
            options.get_one::<String>("origin").map(String::as_str),
        )?;
        return serialize(
            login_flow::login_platform(
                rt,
                &name,
                &endpoint,
                "platform-management",
                options.get_flag("no-browser"),
                SessionPurpose::Management,
            )
            .await?,
        );
    }
    let stored = read_profile_for(rt, &name, SessionPurpose::Management)?;
    match action {
        "check" => serialize(session::me(rt, &stored).await?),
        "logout" => serialize(session::logout(rt, &name, &stored).await?),
        "plan" => serialize(plans::plan(rt, &stored, text("manifest")?).await?),
        "inspect" => serialize(plans::inspect(rt, &stored, text("id")?).await?),
        "status" => serialize(plans::status(rt, &stored, text("id")?).await?),
        "apply" => serialize(
            plans::apply(
                rt,
                &stored,
                text("id")?,
                text("digest")?,
                options.get_one::<String>("request-id").map(String::as_str),
                options.get_flag("retry"),
            )
            .await?,
        ),
        "capabilities" | "systems" | "drafts" | "versions" | "grants" | "audit" => {
            let credential = session::access(rt, &stored).await?;
            let mut url =
                url::Url::parse(&format!("{}/platform/management/{action}", stored.origin))
                    .map_err(|_| invalid())?;
            let id = options.try_get_one::<String>("id").ok().flatten();
            if let Some(id) = id {
                if id.len() > 128
                    || !id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_:-".contains(&b))
                {
                    return Err(invalid());
                }
                let path = if action == "audit" {
                    format!("/platform/management/plans/{id}/audit")
                } else if action == "versions" || action == "grants" {
                    format!("/platform/management/drafts/{id}/{action}")
                } else {
                    format!("/platform/management/{action}/{id}")
                };
                url.set_path(&path);
            }
            if let Some(page) = options.try_get_one::<String>("page").ok().flatten() {
                let n: u32 = page.parse().map_err(|_| invalid())?;
                if !(1..=100000).contains(&n) {
                    return Err(invalid());
                }
                url.query_pairs_mut()
                    .append_pair("page", page)
                    .append_pair("pageSize", "20");
            }
            if let Some(system) = options.try_get_one::<String>("system").ok().flatten() {
                url.query_pairs_mut()
                    .append_pair("systemEnvironmentId", system);
            }
            let request = transport::client()?
                .get(url)
                .bearer_auth(&credential.access_token);
            match action {
                "capabilities" => serialize(http::request::<model::Capabilities>(request).await?),
                "systems" if id.is_some() => {
                    serialize(http::request::<model::SystemView>(request).await?)
                }
                "systems" => {
                    serialize(http::request::<model::Page<model::SystemView>>(request).await?)
                }
                "drafts" if id.is_some() => {
                    serialize(http::request::<query_model::Draft>(request).await?)
                }
                "drafts" => serialize(http::request::<query_model::DraftPage>(request).await?),
                "versions" => serialize(
                    http::request::<model::Page<super::contract::Published>>(request).await?,
                ),
                "grants" => serialize(http::request::<model::Page<model::Grant>>(request).await?),
                "audit" => {
                    serialize(http::request::<model::Page<query_model::AuditEvent>>(request).await?)
                }
                _ => Err(invalid()),
            }
        }
        _ => Err(invalid()),
    }
}
