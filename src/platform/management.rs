//! 本人平台管理命名空间；独立用途记录，不复用门户 Cookie 或员工业务凭证。
mod http;
pub(crate) mod model;
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

pub(super) async fn execute(
    rt: &Runtime,
    options: &crate::interface::request::Management,
) -> Result<serde_json::Value> {
    let action = options.action;
    let name =
        super::context::selected(rt, options.profile.as_deref(), SessionPurpose::Management)?;
    profile::validate_name(&name)?;
    if action == "login" {
        let login = options.login.as_ref().ok_or_else(invalid)?;
        let endpoint = endpoints::resolve(&login.environment, login.origin.as_deref())?;
        return serialize(
            login_flow::login_platform(
                rt,
                &name,
                &endpoint,
                "platform-management",
                login.no_browser,
                SessionPurpose::Management,
            )
            .await?,
        );
    }
    if action == "logout"
        && !crate::private_store::validate(
            &super::profile_path_for(rt, &name, SessionPurpose::Management),
            false,
        )?
    {
        return serialize(session::absent_logout());
    }
    let stored = read_profile_for(rt, &name, SessionPurpose::Management)?;
    match action {
        "check" => serialize(session::me(rt, &stored).await?),
        "logout" => serialize(session::logout(rt, &name, &stored).await?),
        "plan" => serialize(plans::plan(rt, &stored, &options.manifest).await?),
        "inspect" => serialize(
            plans::inspect(rt, &stored, options.id.as_deref().ok_or_else(invalid)?).await?,
        ),
        "status" => {
            serialize(plans::status(rt, &stored, options.id.as_deref().ok_or_else(invalid)?).await?)
        }
        "apply" => serialize(
            plans::apply(
                rt,
                &stored,
                options.id.as_deref().ok_or_else(invalid)?,
                &options.digest,
                options.request_id.as_deref(),
                options.retry,
            )
            .await?,
        ),
        "capabilities" | "systems" | "drafts" | "versions" | "grants" | "audit" => {
            let credential = session::access(rt, &stored).await?;
            let mut url =
                url::Url::parse(&format!("{}/platform/management/{action}", stored.origin))
                    .map_err(|_| invalid())?;
            let id = options.id.as_deref();
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
            if let Some(page) = options.page {
                url.query_pairs_mut()
                    .append_pair("page", &page.to_string())
                    .append_pair("pageSize", "20");
            }
            if let Some(system) = options.system.as_deref() {
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
