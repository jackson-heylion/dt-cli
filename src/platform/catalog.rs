use super::{contract::*, model::PlatformProfile, session, transport};
use crate::{
    Runtime,
    output::{Failure, Result, invalid},
};
use sha2::{Digest, Sha256};

pub(super) async fn list(
    rt: &Runtime,
    profile: &PlatformProfile,
    page: u32,
) -> Result<CatalogPage> {
    if !(1..=100000).contains(&page) {
        return Err(invalid());
    }
    let credentials = session::access(rt, profile).await?;
    transport::request(
        transport::client()?
            .get(format!(
                "{}/platform/cli/catalog?page={page}&pageSize=20",
                profile.origin
            ))
            .bearer_auth(&credentials.access_token),
    )
    .await
}
pub(super) async fn views(
    rt: &Runtime,
    profile: &PlatformProfile,
    operation: &str,
    version: u64,
) -> Result<Vec<ViewOption>> {
    identifier(operation)?;
    let credentials = session::access(rt, profile).await?;
    let candidates: Vec<ViewOption> = transport::request(
        transport::client()?
            .get(format!(
                "{}/platform/cli/catalog/{operation}/{version}/views",
                profile.origin
            ))
            .bearer_auth(&credentials.access_token),
    )
    .await?;
    Ok(candidates)
}
pub(super) async fn views_for(
    rt: &Runtime,
    p: &PlatformProfile,
    c: &ExecutionContract,
) -> Result<Vec<ViewOption>> {
    let mut candidates = views(rt, p, &c.operation_id, c.operation_version).await?;
    if let Some(recent) = super::view_memory::recent(rt, p, &c.system_id) {
        for candidate in &mut candidates {
            candidate.recently_selected = candidate.supported
                && candidate.view_id == recent.view_id
                && candidate.object_id == recent.object_id;
        }
    }
    Ok(candidates)
}
pub(super) fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 100
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(invalid());
    }
    Ok(())
}
pub(super) async fn contract(
    rt: &Runtime,
    profile: &PlatformProfile,
    operation: &str,
    version: u64,
) -> Result<Published> {
    identifier(operation)?;
    if let Some(published) = super::metadata::contract(profile, operation, version) {
        return Ok(published);
    }
    // 定向读取版本，服务端独立核验授权；不遍历全部目录。
    let credentials = session::access(rt, profile).await?;
    let published: Published = transport::request(
        transport::client()?
            .get(format!(
                "{}/platform/cli/catalog/{operation}/{version}",
                profile.origin
            ))
            .bearer_auth(&credentials.access_token),
    )
    .await?;
    verify(profile, &published, operation, version)?;
    super::metadata::remember_contract(profile, &published);
    Ok(published)
}
pub(super) fn verify_integrity(
    profile: &PlatformProfile,
    published: &Published,
    operation: &str,
    version: u64,
) -> Result<()> {
    let encoded = serde_json::to_vec(&published.contract).map_err(|_| crate::http::protocol())?;
    if format!("{:x}", Sha256::digest(encoded)) != published.digest {
        return Err(crate::http::protocol());
    }
    let c = &published.contract;
    if c.contract_schema_version != 1
        || c.operation_id != operation
        || c.operation_version != version
        || c.environment != profile.environment
        || (profile.system_id != "platform" && c.system_id != profile.system_id)
    {
        return Err(crate::http::protocol());
    }
    Ok(())
}
pub(super) fn verify(
    profile: &PlatformProfile,
    published: &Published,
    operation: &str,
    version: u64,
) -> Result<()> {
    verify_integrity(profile, published, operation, version)?;
    let c = &published.contract;
    // 语义版本逐段比较，不把 0.10 误判为 0.9 之前。
    let parse = |v: &str| -> Option<Vec<u64>> {
        let parts: Vec<_> = v.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        parts.iter().map(|p| p.parse::<u64>().ok()).collect()
    };
    if parse(env!("CARGO_PKG_VERSION")).ok_or_else(crate::http::protocol)?
        < parse(&c.minimum_cli_version).ok_or_else(crate::http::protocol)?
    {
        return Err(Failure::new(
            "CLI_UPGRADE_REQUIRED",
            2,
            "该执行版本要求更新 CLI，请先升级。",
        ));
    }
    Ok(())
}
pub(super) async fn read(
    rt: &Runtime,
    profile: &PlatformProfile,
    operation: &str,
    version: u64,
    path: &str,
    query: &str,
    view: Option<(u32, String)>,
) -> Result<super::executor::ExecutionResult> {
    let started = std::time::Instant::now();
    let published = contract(rt, profile, operation, version).await?;
    let c = &published.contract;
    if c.effect != Effect::Read {
        return Err(Failure::new(
            "WRITE_INTENT_REQUIRED",
            4,
            "写入只能通过准备、明确确认和一次派发执行。",
        ));
    }
    if c.media != Media::None {
        return Err(Failure::new(
            "INPUT_FILE_REQUIRED",
            2,
            "此接口使用请求体，请通过 --input-file 和可选的 --body-file 传入完整参数。",
        ));
    }
    validate_path(&c.path, path)?;
    if query.len() > 8192 || query.chars().any(char::is_control) || query.contains('#') {
        return Err(invalid());
    }
    for (name, _) in url::form_urlencoded::parse(query.as_bytes()) {
        if !c
            .parameters
            .iter()
            .any(|p| p.location == Location::Query && p.name == name)
        {
            return Err(Failure::new(
                "PARAMETER_NOT_DECLARED",
                2,
                "查询参数未在固定版本声明。",
            ));
        }
    }
    for p in &c.parameters {
        if p.required
            && p.location == Location::Query
            && !url::form_urlencoded::parse(query.as_bytes()).any(|(n, _)| n == p.name)
        {
            return Err(Failure::new("REQUIRED_PARAMETER", 2, "缺少必填查询参数。"));
        }
        if p.required && p.location == Location::Header {
            return Err(Failure::new(
                "INPUT_FILE_REQUIRED",
                2,
                "此接口需要业务头，请通过 --input-file 传入完整参数。",
            ));
        }
    }
    let requested =
        view.map(|(view_id, object_id)| super::input_model::TaskView { view_id, object_id });
    let selected = super::executor::choose_view(rt, profile, c, requested).await?;
    let input = super::input_model::WireInvocation {
        path: path.into(),
        query: query.into(),
        headers: vec![],
        body: String::new(),
        view: selected,
    };
    let preparation = super::timing::millis(started);
    let mut result = super::executor::execute(
        rt,
        profile,
        &published,
        &input,
        None,
        c.limits.max_seconds.into(),
    )
    .await?;
    if let Some(timing) = &mut result.timing {
        timing.preparation_millis += preparation;
        timing.client_elapsed_millis = super::timing::millis(started);
    }
    Ok(result)
}
fn validate_path(template: &str, path: &str) -> Result<()> {
    if !path.starts_with('/')
        || path.contains('%')
        || path.contains('\\')
        || path.contains(';')
        || path.contains('?')
        || path.contains('#')
        || path.contains("//")
        || path.len() > 2048
        || path.bytes().any(|b| !(33..=126).contains(&b))
    {
        return Err(invalid());
    }
    let a: Vec<_> = template.split('/').collect();
    let b: Vec<_> = path.split('/').collect();
    if a.len() != b.len() {
        return Err(invalid());
    }
    for (part, value) in a.iter().zip(b) {
        if value == "."
            || value == ".."
            || (!(*part == value
                || part.starts_with('{') && part.ends_with('}') && !value.is_empty()))
        {
            return Err(invalid());
        }
    }
    Ok(())
}
