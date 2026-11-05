//! 当前平台上下文只提供身份与环境选择便利，授权仍经实际服务端核验。
use super::{
    model::{IdentityDomain, SessionPurpose},
    read_profile_for, serialize, session,
};
use crate::{
    Runtime,
    output::{Failure, Result, invalid},
    private_store, profile,
};
use clap::{Arg, ArgMatches, Command};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Current {
    /// 当前上下文元数据格式。
    schema_version: u32,
    /// 明确选中的本地 profile。
    profile: String,
    /// 固定平台地址。
    origin: String,
    /// 固定运行环境。
    environment: String,
    /// 固定本人主体。
    subject_id: String,
    /// 固定身份域。
    domain: IdentityDomain,
    /// 固定业务或管理用途。
    purpose: SessionPurpose,
}
fn path(rt: &Runtime, purpose: SessionPurpose) -> std::path::PathBuf {
    rt.root
        .join("platform-context")
        .join(format!("{}.json", purpose.directory()))
}
fn required() -> Failure {
    Failure::new(
        "PLATFORM_CONTEXT_REQUIRED",
        2,
        "缺少当前平台上下文；请显式提供 --profile，或先设置对应身份和环境。",
    )
}
pub(super) fn selected(
    rt: &Runtime,
    options: &ArgMatches,
    purpose: SessionPurpose,
) -> Result<String> {
    if let Some(name) = options.get_one::<String>("profile") {
        profile::validate_name(name)?;
        return Ok(name.clone());
    }
    let current: Current = private_store::read(&path(rt, purpose))?.ok_or_else(required)?;
    profile::validate_name(&current.profile)?;
    let stored = read_profile_for(rt, &current.profile, purpose)?;
    if current.schema_version != 1
        || current.purpose != purpose
        || current.origin != stored.origin
        || current.environment != stored.environment
        || current.subject_id != stored.subject_id
        || current.domain != stored.domain
    {
        return Err(Failure::new(
            "PLATFORM_CONTEXT_CHANGED",
            4,
            "当前 profile 的身份或环境已变化，请重新明确设置上下文。",
        ));
    }
    Ok(current.profile)
}
fn purpose(options: &ArgMatches) -> Result<SessionPurpose> {
    match options.get_one::<String>("purpose").map(String::as_str) {
        Some("business") => Ok(SessionPurpose::Cli),
        Some("management") => Ok(SessionPurpose::Management),
        _ => Err(invalid()),
    }
}
pub(super) fn command() -> Command {
    let choice = || {
        Arg::new("purpose")
            .long("purpose")
            .default_value("business")
            .value_parser(["business", "management"])
    };
    Command::new("context")
        .subcommand(
            Command::new("use")
                .arg(Arg::new("profile").long("profile").required(true))
                .arg(Arg::new("environment").long("environment").required(true))
                .arg(choice()),
        )
        .subcommand(Command::new("show").arg(choice()))
        .subcommand(Command::new("clear").arg(choice()))
}
pub(super) async fn execute(rt: &Runtime, arguments: &ArgMatches) -> Result<serde_json::Value> {
    let (action, options) = arguments.subcommand().ok_or_else(invalid)?;
    let purpose = purpose(options)?;
    if action == "clear" {
        let file = path(rt, purpose);
        if private_store::read::<Current>(&file)?.is_some() {
            std::fs::remove_file(file).map_err(|_| private_store::unavailable())?;
        }
        return serialize(Cleared {
            purpose,
            cleared: true,
        });
    }
    let name = if action == "use" {
        options
            .get_one::<String>("profile")
            .ok_or_else(invalid)?
            .clone()
    } else {
        let current: Current = private_store::read(&path(rt, purpose))?.ok_or_else(required)?;
        current.profile
    };
    profile::validate_name(&name)?;
    let stored = read_profile_for(rt, &name, purpose)?;
    if action == "use" && options.get_one::<String>("environment") != Some(&stored.environment) {
        return Err(Failure::new(
            "PLATFORM_CONTEXT_ENVIRONMENT_MISMATCH",
            2,
            "指定环境与所选 profile 不一致，未设置上下文。",
        ));
    }
    let current = Current {
        schema_version: 1,
        profile: name,
        origin: stored.origin.clone(),
        environment: stored.environment.clone(),
        subject_id: stored.subject_id.clone(),
        domain: stored.domain.clone(),
        purpose,
    };
    if action == "show" {
        let saved: Current = private_store::read(&path(rt, purpose))?.ok_or_else(required)?;
        if saved.subject_id != current.subject_id
            || saved.domain != current.domain
            || saved.environment != current.environment
            || saved.origin != current.origin
            || saved.purpose != purpose
        {
            return Err(Failure::new(
                "PLATFORM_CONTEXT_CHANGED",
                4,
                "上下文身份或环境已变化，请重新明确设置。",
            ));
        }
    }
    let identity = session::me(rt, &stored).await?;
    if action == "use" {
        private_store::write(&path(rt, purpose), &current)?;
    }
    serialize(ContextView {
        context: current,
        identity,
    })
}
#[derive(Serialize)]
struct Cleared {
    /// 明确清除的用途。
    purpose: SessionPurpose,
    /// 本地上下文已清除。
    cleared: bool,
}
#[derive(Serialize)]
struct ContextView {
    /// 公开本人及环境，不含凭证或安全存储键。
    context: Current,
    /// 本次在线核验的会话身份。
    identity: super::model::SessionView,
}
