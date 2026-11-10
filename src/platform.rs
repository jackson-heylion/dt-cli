//! 新平台独立登录入口；复用安全存储和本机回调，不接收业务凭证。
mod actions;
mod catalog;
mod context;
pub(crate) mod contract;
mod diagnostic;
pub(crate) mod discovery;
mod endpoints;
mod errors;
pub(crate) mod executor;
mod input_model;
mod invocation;
#[cfg(test)]
mod invocation_tests;
pub(crate) mod jobs;
mod login_flow;
pub(crate) mod management;
mod metadata;
pub(crate) mod model;
mod profiles;
mod records;
mod recovery;
mod session;
mod timing;
mod transport;
mod view_memory;
pub(crate) mod writes;

use crate::{
    Runtime,
    credentials::Credentials,
    login,
    output::{Failure, Result, invalid},
    private_store, profile,
};
use model::*;
use serde_json::Value;
use tokio::net::TcpListener;

pub(crate) async fn execute(
    rt: &Runtime,
    request: crate::interface::request::Platform,
) -> Result<Value> {
    let mut result = execute_inner(rt, request.clone()).await;
    if let Err(error) = &mut result {
        recovery::attach(rt, &request, error).await;
    }
    result
}
async fn execute_inner(
    rt: &Runtime,
    request: crate::interface::request::Platform,
) -> Result<Value> {
    use crate::interface::request::Action;
    match &request.action {
        Action::Profiles => return profiles::list(rt),
        Action::Context(input) => return context::execute(rt, input).await,
        Action::Manage(input) => return management::execute(rt, input).await,
        Action::Jobs(input) => return actions::execute(rt, "jobs", input).await,
        Action::Writes(input) => return actions::execute(rt, "writes", input).await,
        _ => {}
    }
    let name = context::selected(rt, request.profile.as_deref(), SessionPurpose::Cli)?;
    profile::validate_name(&name)?;
    if let Action::Login(input) = request.action {
        let endpoint = endpoints::resolve(&input.environment, input.origin.as_deref())?;
        return serialize(
            login_flow::login_platform(
                rt,
                &name,
                &endpoint,
                &input.system,
                input.no_browser,
                SessionPurpose::Cli,
            )
            .await?,
        );
    }
    if matches!(request.action, Action::Logout)
        && !private_store::validate(&profile_path_for(rt, &name, SessionPurpose::Cli), false)?
    {
        return serialize(session::absent_logout());
    }
    let stored = read_profile(rt, &name)?;
    match request.action {
        Action::Check => serialize(session::me(rt, &stored).await?),
        Action::Search(input) => discovery::search(rt, &stored, &input).await,
        Action::Resolve(input) => {
            discovery::execute(rt, &stored, "resolve", &input, &Default::default()).await
        }
        Action::Describe(input) => {
            discovery::execute(rt, &stored, "describe", &input, &Default::default()).await
        }
        Action::Template(target, input) => {
            discovery::execute(rt, &stored, "template", &target, &input).await
        }
        Action::Diagnose(input) => diagnostic::execute(rt, &stored, &input).await,
        Action::Catalog(page) => serialize(catalog::list(rt, &stored, page).await?),
        Action::Views(input) => {
            let selected = discovery::selected(rt, &stored, &input).await?;
            serialize(catalog::views_for(rt, &stored, &selected.contract).await?)
        }
        Action::Call(input) => {
            let selected = discovery::selected(rt, &stored, &input.target).await?;
            let operation = &selected.contract.operation_id;
            let version = selected.contract.operation_version;
            let view = input.view.map(|id| (id, input.object.unwrap()));
            if !input.files.input.is_empty() {
                let mut invocation =
                    invocation::input_file(&input.files.input, input.files.body.as_deref())?;
                if let Some((view_id, object_id)) = view {
                    if invocation.view.is_some() {
                        return Err(invalid());
                    }
                    invocation.view = Some(input_model::TaskView { view_id, object_id });
                }
                return executor::present(
                    executor::read_input(rt, &stored, operation, version, invocation).await?,
                );
            }
            executor::present(
                catalog::read(
                    rt,
                    &stored,
                    operation,
                    version,
                    &input.path,
                    &input.query,
                    view,
                )
                .await?,
            )
        }
        Action::Sessions(page) => serialize(session::list(rt, &stored, page).await?),
        Action::Revoke(id) => {
            if !handle(&id) {
                return Err(invalid());
            }
            session::revoke(rt, &stored, &id).await?;
            #[derive(serde::Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Revocation {
                /// 平台已确认指定会话撤销。
                revoked: bool,
                /// 原明确会话编号。
                session_id: String,
            }
            serialize(Revocation {
                revoked: true,
                session_id: id,
            })
        }
        Action::Logout => serialize(session::logout(rt, &name, &stored).await?),
        _ => Err(invalid()),
    }
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
    endpoints::validate_profile(&stored.origin, &stored.environment)?;
    let prefix = format!(
        "{}:{}:{name}:",
        purpose.directory(),
        transport::challenge(&stored.origin)
    );
    if !stored
        .credential_key
        .strip_prefix(&prefix)
        .is_some_and(handle)
    {
        return Err(invalid());
    }
    if stored.schema_version != 1 || !handle(&stored.session_id) || stored.purpose != purpose {
        return Err(invalid());
    }
    Ok(stored)
}
