//! 本人接口发现、唯一解析与固定契约输入模板，不自动派发模糊候选。
use super::{catalog, contract::Published, model::PlatformProfile, session, transport};
use crate::output::{PartialData, PartialMeta};
use crate::{
    Runtime,
    output::{Failure, Result, invalid},
};
use serde::{Deserialize, Serialize};
mod templates;
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Item {
    /// 本人已授权的固定执行版本与摘要。
    pub published: Published,
    /// 当前明确业务别名，可空。
    pub alias: Option<String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Page {
    /// 服务端数据库分页的候选。
    pub items: Vec<Item>,
    /// 服务端核权后的总数。
    pub total: u64,
    /// 页码，从一开始。
    pub page: u32,
    /// 本页容量。
    pub page_size: u32,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Resolution {
    /// RESOLVED、AMBIGUOUS 或 UNAVAILABLE。
    state: String,
    /// 唯一固定接口，未确定为空。
    selected: Option<Item>,
    /// 供明确选择的有界候选。
    candidates: Page,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Example {
    /// 建议输入文件名。
    pub input_file_name: String,
    /// 具名调用参数模板原文。
    pub input_json: String,
    /// 需要核对并补齐的实际业务值。
    pub fields_to_fill: Vec<String>,
    /// 固定接口与版本的调用步骤。
    pub commands: Vec<String>,
}
#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct Detail {
    /// 固定授权契约。
    pub published: Published,
    /// 非秘密参数示例。
    pub example: Example,
}
pub(super) async fn selected(
    rt: &Runtime,
    p: &PlatformProfile,
    o: &crate::interface::request::Target,
) -> Result<Published> {
    select(rt, p, o, true).await
}
pub(super) async fn for_diagnostic(
    rt: &Runtime,
    p: &PlatformProfile,
    o: &crate::interface::request::Target,
) -> Result<Published> {
    select(rt, p, o, false).await
}
async fn select(
    rt: &Runtime,
    p: &PlatformProfile,
    o: &crate::interface::request::Target,
    strict: bool,
) -> Result<Published> {
    let operation = o.operation.as_str();
    let alias = o.alias.as_str();
    let version = o.version.unwrap_or(0);
    if !operation.is_empty() && version > 0 {
        let published = if strict {
            catalog::contract(rt, p, operation, version).await?
        } else {
            let credentials = session::access(rt, p).await?;
            let value: Published = transport::request(
                transport::client()?
                    .get(format!(
                        "{}/platform/cli/catalog/{operation}/{version}",
                        p.origin
                    ))
                    .bearer_auth(&credentials.access_token),
            )
            .await?;
            catalog::verify_integrity(p, &value, operation, version)?;
            value
        };
        if !o.system.as_str().is_empty() && o.system.as_str() != published.contract.system_id {
            return Err(invalid());
        }
        return Ok(published);
    }
    let credentials = session::access(rt, p).await?;
    let resolved: Resolution = transport::request(
        transport::client()?
            .get(format!("{}/platform/cli/discovery/resolve", p.origin))
            .query(&[
                ("operation", operation),
                ("alias", alias),
                ("system", o.system.as_str()),
                ("version", &version.to_string()),
            ])
            .bearer_auth(&credentials.access_token),
    )
    .await?;
    if resolved.state != "RESOLVED" || resolved.candidates.total != 1 || resolved.selected.is_none()
    {
        return Err(Failure::new(
            if resolved.state == "AMBIGUOUS" {
                "INTERFACE_AMBIGUOUS"
            } else {
                "INTERFACE_UNAVAILABLE"
            },
            2,
            "请核对本人已授权候选，明确选择系统、接口及版本。",
        )
        .with_partial(
            PartialData::Resolution(resolved),
            PartialMeta {
                stage: Some("RESOLVE"),
                next_action: Some("SELECT_EXPLICIT_TARGET"),
                ..Default::default()
            },
        ));
    }
    let published = resolved.selected.unwrap().published;
    catalog::verify_integrity(
        p,
        &published,
        &published.contract.operation_id,
        published.contract.operation_version,
    )?;
    if strict {
        catalog::verify(
            p,
            &published,
            &published.contract.operation_id,
            published.contract.operation_version,
        )?;
        super::metadata::remember_contract(p, &published);
    }
    Ok(published)
}
pub(super) async fn search(
    rt: &Runtime,
    p: &PlatformProfile,
    o: &crate::interface::request::Search,
) -> Result<serde_json::Value> {
    let credentials = session::access(rt, p).await?;
    let result: Page = transport::request(
        transport::client()?
            .get(format!("{}/platform/cli/discovery", p.origin))
            .query(&[
                ("system", o.system.as_str()),
                ("name", o.name.as_str()),
                ("method", o.method.as_str()),
                ("path", o.path.as_str()),
                ("page", &o.page.to_string()),
                ("pageSize", &o.page_size.to_string()),
            ])
            .bearer_auth(&credentials.access_token),
    )
    .await?;
    super::serialize(result)
}
pub(super) async fn execute(
    rt: &Runtime,
    p: &PlatformProfile,
    action: &str,
    o: &crate::interface::request::Target,
    template: &crate::interface::request::Template,
) -> Result<serde_json::Value> {
    let published = selected(rt, p, o).await?;
    if action == "resolve" {
        return super::serialize(published);
    }
    let credentials = session::access(rt, p).await?;
    let detail: Detail = transport::request(
        transport::client()?
            .get(format!(
                "{}/platform/cli/discovery/{}/{}",
                p.origin, published.contract.operation_id, published.contract.operation_version
            ))
            .bearer_auth(&credentials.access_token),
    )
    .await?;
    if detail.published.digest != published.digest {
        return Err(crate::http::protocol());
    }
    if action == "template" {
        return templates::export(&detail, template);
    }
    super::serialize(detail)
}
