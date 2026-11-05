//! 本人接口发现、唯一解析与固定契约输入模板，不自动派发模糊候选。
use super::{catalog, contract::Published, model::PlatformProfile, session, transport};
use crate::{
    Runtime,
    output::{Failure, Result, invalid},
};
use clap::{Arg, ArgMatches, Command};
use serde::{Deserialize, Serialize};
mod templates;
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Item {
    /// 本人已授权的固定执行版本与摘要。
    pub published: Published,
    /// 当前明确业务别名，可空。
    pub alias: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Page {
    /// 服务端数据库分页的候选。
    pub items: Vec<Item>,
    /// 服务端核权后的总数。
    pub total: u64,
    /// 页码，从一开始。
    pub page: u32,
    /// 本页容量。
    pub page_size: u32,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Resolution {
    /// RESOLVED、AMBIGUOUS 或 UNAVAILABLE。
    state: String,
    /// 唯一固定接口，未确定为空。
    selected: Option<Item>,
    /// 供明确选择的有界候选。
    candidates: Page,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Example {
    /// 建议输入文件名。
    pub input_file_name: String,
    /// 具名调用参数模板原文。
    pub input_json: String,
    /// 需要核对并补齐的实际业务值。
    pub fields_to_fill: Vec<String>,
    /// 固定接口与版本的调用步骤。
    pub commands: Vec<String>,
}
#[derive(Deserialize, Serialize)]
pub(super) struct Detail {
    /// 固定授权契约。
    pub published: Published,
    /// 非秘密参数示例。
    pub example: Example,
}
pub(super) fn target(command: Command) -> Command {
    command
        .arg(
            Arg::new("operation")
                .long("operation")
                .required_unless_present("alias"),
        )
        .arg(Arg::new("alias").long("alias").conflicts_with("operation"))
        .arg(Arg::new("system").long("system"))
        .arg(Arg::new("version").long("version"))
}
pub(super) fn commands() -> Vec<Command> {
    let profile = || Arg::new("profile").long("profile");
    vec![
        Command::new("search")
            .arg(profile())
            .arg(Arg::new("system").long("system"))
            .arg(Arg::new("name").long("name"))
            .arg(Arg::new("method").long("method"))
            .arg(Arg::new("path").long("path"))
            .arg(Arg::new("page").long("page").default_value("1"))
            .arg(Arg::new("page-size").long("page-size").default_value("20")),
        target(Command::new("resolve").arg(profile())),
        target(Command::new("describe").arg(profile())),
        target(Command::new("template").arg(profile()))
            .arg(Arg::new("output").long("output"))
            .arg(
                Arg::new("overwrite-digest")
                    .long("overwrite-digest")
                    .requires("output"),
            ),
    ]
}
pub(super) fn text<'a>(o: &'a ArgMatches, key: &str) -> &'a str {
    o.get_one::<String>(key).map(String::as_str).unwrap_or("")
}
pub(super) async fn selected(
    rt: &Runtime,
    p: &PlatformProfile,
    o: &ArgMatches,
) -> Result<Published> {
    select(rt, p, o, true).await
}
pub(super) async fn for_diagnostic(
    rt: &Runtime,
    p: &PlatformProfile,
    o: &ArgMatches,
) -> Result<Published> {
    select(rt, p, o, false).await
}
async fn select(
    rt: &Runtime,
    p: &PlatformProfile,
    o: &ArgMatches,
    strict: bool,
) -> Result<Published> {
    let operation = text(o, "operation");
    let alias = text(o, "alias");
    let version = match o.get_one::<String>("version") {
        Some(v) => v.parse::<u64>().map_err(|_| invalid())?,
        None => 0,
    };
    if o.contains_id("version") && version == 0 {
        return Err(invalid());
    }
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
        if !text(o, "system").is_empty() && text(o, "system") != published.contract.system_id {
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
                ("system", text(o, "system")),
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
            super::serialize(&resolved)?,
            serde_json::json!({"stage":"RESOLVE","nextAction":"SELECT_EXPLICIT_TARGET"}),
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
pub(super) async fn execute(
    rt: &Runtime,
    p: &PlatformProfile,
    action: &str,
    o: &ArgMatches,
) -> Result<serde_json::Value> {
    if action == "search" {
        let page = text(o, "page").parse::<u32>().map_err(|_| invalid())?;
        let size = text(o, "page-size").parse::<u32>().map_err(|_| invalid())?;
        if !(1..=100000).contains(&page) || !(1..=100).contains(&size) {
            return Err(invalid());
        }
        let credentials = session::access(rt, p).await?;
        let result: Page = transport::request(
            transport::client()?
                .get(format!("{}/platform/cli/discovery", p.origin))
                .query(&[
                    ("system", text(o, "system")),
                    ("name", text(o, "name")),
                    ("method", text(o, "method")),
                    ("path", text(o, "path")),
                    ("page", &page.to_string()),
                    ("pageSize", &size.to_string()),
                ])
                .bearer_auth(&credentials.access_token),
        )
        .await?;
        return super::serialize(result);
    }
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
        return templates::export(&detail, o);
    }
    super::serialize(detail)
}

pub(super) fn help_lines(scope: &str) -> Vec<String> {
    let target =
        "[--profile <name>] (--operation <id> | --alias <name> [--system <id>]) [--version <n>]";
    let lines=vec![
        "platform search [--profile <name>] [--system <id>] [--name <text>] [--method <HTTP>] [--path <text>] [--page <n>] [--page-size <n>]".to_owned(),
        format!("platform resolve {target}"),format!("platform describe {target}"),
        format!("platform template {target} [--output <file>] [--overwrite-digest <existing-sha256>]"),
        format!("platform diagnose {target} [--probe --input-file <file> [--body-file <file>] --scope <contract-data-scope>]")
    ];
    lines
        .into_iter()
        .filter(|line| scope == "all" || line.starts_with(&format!("platform {scope} ")))
        .collect()
}
