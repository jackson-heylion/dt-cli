//! Client-side execution of deployment-owned, published read contracts.
use super::*;
use std::collections::BTreeMap;
mod pacing;
mod scope;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Plan {
    schema_version: u8,
    base_uri: String,
    method: String,
    path: String,
    execution: Value,
    limits: Limits,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Limits {
    timeout_seconds: u64,
    page_bytes: usize,
    max_pages: u32,
    max_records: usize,
    max_result_bytes: usize,
    max_seconds: u64,
    concurrency: u32,
    #[serde(default = "pacing::default_interval")]
    min_request_interval_millis: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Mapping {
    source: String,
    value: Value,
    #[serde(default)]
    required: bool,
    array_encoding: Option<String>,
    #[serde(default)]
    wrap_array: bool,
}

struct Credential {
    token: Zeroizing<String>,
    trusted: Value,
}

fn mismatch() -> Failure {
    Failure::new(
        "UPSTREAM_CONTRACT_MISMATCH",
        5,
        "业务响应与已发布接口说明不符。",
    )
}
fn limit() -> Failure {
    Failure::new(
        "RESOURCE_LIMIT",
        7,
        "本机查询达到接口限额；保留未完成范围。",
    )
}
fn business_auth_required() -> Failure {
    let mut failure = Failure::new(
        "BUSINESS_CREDENTIAL_REJECTED",
        3,
        "重新获取的业务凭证仍未被目标系统接受。本人 IAM 登录可能仍然有效。",
    );
    failure.hint = "保留原任务，联系目标系统维护者核对所选视角的身份处理；不切换账号或视角绕过。";
    failure
}

impl Plan {
    pub(super) fn parse(operation: &Value, environment: &str) -> Result<Self> {
        let plan: Self =
            serde_json::from_value(operation["direct"].clone()).map_err(|_| catalog_invalid())?;
        let base = url::Url::parse(&plan.base_uri).map_err(|_| catalog_invalid())?;
        let loopback = base.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        let local =
            matches!(environment, "local" | "fixture") && loopback && base.scheme() == "http";
        if plan.schema_version != 1
            || !(base.scheme() == "https" || local)
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.path().contains(['%', '\\'])
            || base.path().contains("..")
            || !plan.path.starts_with('/')
            || plan.path.contains("..")
            || plan.path.contains(['%', '?', '#', '\\'])
            || plan.path.contains("//")
            || !matches!(plan.method.as_str(), "GET" | "POST")
            || plan.execution["engine"] != "direct-read"
            || plan.execution["schemaVersion"] != 1
            || plan.limits.timeout_seconds == 0
            || plan.limits.timeout_seconds > crate::constants::query::REQUEST_SECONDS
            || plan.limits.page_bytes == 0
            || plan.limits.page_bytes > BODY_LIMIT
            || plan.limits.max_pages == 0
            || plan.limits.max_pages > crate::constants::query::DIRECT_MAX_PAGES
            || plan.limits.max_records == 0
            || plan.limits.max_records > crate::constants::query::DIRECT_MAX_RECORDS
            || plan.limits.max_result_bytes == 0
            || plan.limits.max_result_bytes > RESULT_BODY_LIMIT
            || plan.limits.max_seconds == 0
            || plan.limits.max_seconds > crate::constants::query::DIRECT_MAX_SECONDS
            || plan.limits.min_request_interval_millis
                < crate::constants::query::DIRECT_REQUEST_INTERVAL_MILLIS
            || plan.limits.min_request_interval_millis
                > crate::constants::query::DIRECT_MAX_REQUEST_INTERVAL_MILLIS
            || plan.limits.concurrency == 0
        {
            return Err(catalog_invalid());
        }
        for location in ["path", "query", "body"] {
            let Some(value) = plan.execution["request"].get(location) else {
                continue;
            };
            let mappings = value.as_object().ok_or_else(catalog_invalid)?;
            for mapping in mappings.values() {
                let value: Mapping =
                    serde_json::from_value(mapping.clone()).map_err(|_| catalog_invalid())?;
                if !matches!(
                    value.source.as_str(),
                    "fixed" | "argument" | "trusted" | "page"
                ) || value
                    .array_encoding
                    .as_deref()
                    .is_some_and(|encoding| encoding != "comma")
                {
                    return Err(catalog_invalid());
                }
            }
        }
        scope::validate(&plan)?;
        Ok(plan)
    }

    fn mapping(
        &self,
        location: &str,
        params: &Value,
        trusted: &Value,
        page: &Value,
    ) -> Result<BTreeMap<String, Value>> {
        let mut result = BTreeMap::new();
        let Some(value) = self.execution["request"].get(location) else {
            return Ok(result);
        };
        for (name, mapping) in value.as_object().ok_or_else(catalog_invalid)? {
            let mapping: Mapping =
                serde_json::from_value(mapping.clone()).map_err(|_| catalog_invalid())?;
            let key = mapping.value.as_str().unwrap_or("");
            let mut value = match mapping.source.as_str() {
                "fixed" => mapping.value.clone(),
                "argument" => params.get(key).cloned().unwrap_or(Value::Null),
                "trusted" => trusted.get(key).cloned().unwrap_or(Value::Null),
                "page" => page.get(key).cloned().unwrap_or(Value::Null),
                _ => return Err(catalog_invalid()),
            };
            if value.is_null() {
                if mapping.required {
                    return Err(invalid());
                }
                continue;
            }
            if mapping.wrap_array {
                value = json!([value]);
            }
            if location == "query" && value.is_array() {
                if mapping.array_encoding.as_deref() != Some("comma") {
                    return Err(catalog_invalid());
                }
                let parts = value
                    .as_array()
                    .ok_or_else(invalid)?
                    .iter()
                    .map(scalar)
                    .collect::<Result<Vec<_>>>()?;
                if parts.iter().any(|part| part.contains(',')) {
                    return Err(invalid());
                }
                value = json!(parts.join(","));
            }
            result.insert(name.clone(), value);
        }
        Ok(result)
    }

    fn request(
        &self,
        client: &reqwest::Client,
        params: &Value,
        credential: &Credential,
        page: &Value,
    ) -> Result<reqwest::RequestBuilder> {
        let mut path = self.path.clone();
        for (name, value) in self.mapping("path", params, &credential.trusted, page)? {
            let text = scalar(&value)?;
            if text.is_empty()
                || !text
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
            {
                return Err(invalid());
            }
            path = path.replace(&format!("{{{name}}}"), &text);
        }
        if path.contains(['{', '}']) {
            return Err(catalog_invalid());
        }
        let mut url = url::Url::parse(&self.base_uri).map_err(|_| catalog_invalid())?;
        url.set_path(&format!("{}{}", url.path().trim_end_matches('/'), path));
        for (name, value) in self.mapping("query", params, &credential.trusted, page)? {
            url.query_pairs_mut().append_pair(&name, &scalar(&value)?);
        }
        let body = self.mapping("body", params, &credential.trusted, page)?;
        let token = credential
            .token
            .strip_prefix("Bearer ")
            .ok_or_else(http::protocol)?;
        let request = client
            .request(
                if self.method == "GET" {
                    reqwest::Method::GET
                } else {
                    reqwest::Method::POST
                },
                url,
            )
            .bearer_auth(token)
            .timeout(Duration::from_secs(self.limits.timeout_seconds));
        if self.method == "GET" {
            if !body.is_empty() {
                return Err(catalog_invalid());
            }
            Ok(request)
        } else {
            let mut nested = json!({});
            for (pointer, value) in body {
                let parts: Vec<_> = pointer
                    .strip_prefix('/')
                    .unwrap_or(&pointer)
                    .split('/')
                    .collect();
                let mut parent = &mut nested;
                for name in &parts[..parts.len() - 1] {
                    let fields = parent.as_object_mut().ok_or_else(catalog_invalid)?;
                    parent = fields
                        .entry((*name).to_owned())
                        .or_insert_with(|| json!({}));
                }
                parent
                    .as_object_mut()
                    .ok_or_else(catalog_invalid)?
                    .insert(parts[parts.len() - 1].to_owned(), value);
            }
            Ok(request.json(&nested))
        }
    }
}

fn scalar(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Bool(_) | Value::Number(_) => Ok(value.to_string()),
        _ => Err(invalid()),
    }
}

async fn credential(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    operation: &Value,
    params: &Value,
) -> Result<Credential> {
    let cache = live_catalog(rt, p, env, client).await?;
    let current = select(
        &cache,
        operation["operationId"]
            .as_str()
            .ok_or_else(catalog_invalid)?,
        operation["operationVersion"].as_str(),
    )?;
    if current["contractDigest"] != operation["contractDigest"]
        || current["direct"] != operation["direct"]
    {
        return Err(contract_changed());
    }
    usable(current, "read")?;
    let system = exchange(rt, p, env, client, current).await?;
    let route = format!(
        "/cli-api/v1/systems/{}/{}",
        p.system_id,
        crate::constants::protocol::DIRECT_CREDENTIAL_SUFFIX
    );
    let mut reply = send(client.post(endpoint(env, &route)?).bearer_auth(system.as_str()).json(&json!({
        "contract": {"operationId":current["operationId"],"operationVersion":current["operationVersion"],"contractDigest":current["contractDigest"]},
        "arguments": params
    }))).await?;
    if reply.status == 422 {
        let alternatives: Vec<_> = views::candidates(rt, p, env, client)
            .await?
            .into_iter()
            .filter(|view| view.supported)
            .collect();
        return Err(Failure::new(
            "VIEW_UNSUPPORTED",
            6,
            "目标业务系统未提供所选视角的身份处理；请从本人可用视角中重新选择。",
        )
        .with_partial(
            json!({"availableViews":alternatives}),
            json!({"complete":false}),
        ));
    }
    if reply.status != 200 {
        return Err(remote_failure(&reply));
    }
    if reply.body["subjectId"] != p.subject_id
        || reply.body["expiresAt"].as_str().and_then(rfc3339).is_none()
        || !reply.body["trusted"].is_object()
    {
        return Err(http::protocol());
    }
    let identity = &operation["direct"]["execution"]["identity"];
    if let Some(field) = match identity["mode"].as_str() {
        Some("supply-chain.brand") => Some("brandId"),
        Some("supply-chain.delivery") => Some("deliveryCenterId"),
        _ => None,
    } {
        let parameter = identity["contextParameter"]
            .as_str()
            .ok_or_else(catalog_invalid)?;
        if reply.body["trusted"][field] != params[parameter] {
            return Err(http::protocol());
        }
    }
    if identity["mode"] == "supply-chain.view" {
        let selected = &params[identity["contextParameter"]
            .as_str()
            .ok_or_else(catalog_invalid)?];
        let view: views::ViewOption = serde_json::from_value(reply.body["trusted"]["view"].clone())
            .map_err(|_| http::protocol())?;
        if json!(view.view_id) != selected["viewId"]
            || json!(view.object_id) != selected["objectId"]
            || !view.supported
            || !view.valid()
            || !matches!(
                view.view_type.as_str(),
                "GROUP" | "BRAND" | "DELIVERY" | "SHOP" | "SUPPLIER"
            )
        {
            return Err(http::protocol());
        }
    }
    let token = reply
        .body
        .as_object_mut()
        .and_then(|fields| fields.remove("token"))
        .and_then(|token| token.as_str().map(str::to_owned))
        .ok_or_else(http::protocol)?;
    if !token.starts_with("Bearer ") || token.contains(['\r', '\n']) {
        return Err(http::protocol());
    }
    let expiry = reply.body["expiresAt"]
        .as_str()
        .and_then(rfc3339)
        .ok_or_else(http::protocol)?;
    if expiry <= chrono::Utc::now()
        || expiry
            > chrono::Utc::now()
                + chrono::Duration::seconds(
                    crate::constants::query::BUSINESS_CREDENTIAL_MAX_SECONDS,
                )
    {
        return Err(http::protocol());
    }
    Ok(Credential {
        token: Zeroizing::new(token),
        trusted: reply.body["trusted"].clone(),
    })
}

fn scope_digest(trusted: &Value) -> Result<String> {
    let mut scope = trusted.clone();
    for key in [
        "authorizedShopCodes",
        "selectedShopCodes",
        "authorizedShopIds",
    ] {
        if let Some(values) = scope[key].as_array_mut() {
            values.sort_by_key(Value::to_string);
        }
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&scope).map_err(|_| http::protocol())?)
    ))
}

async fn verified_credential<F>(
    plan: &Plan,
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    operation: &Value,
    params: &Value,
    started: tokio::time::Instant,
    spent: u64,
    progress: &mut Progress,
    checkpoint: &mut F,
) -> Result<Credential>
where
    F: FnMut(&Progress) -> Result<()>,
{
    for attempt in 0..2 {
        let mut value = credential(rt, p, env, client, operation, params).await?;
        let observed = scope_digest(&value.trusted)?;
        if progress
            .native_scope_digest
            .as_ref()
            .is_some_and(|old| old != &observed)
        {
            return Err(Failure::new(
                "DATA_SCOPE_DENIED",
                4,
                "本人原任务范围已变化；未发起业务核对或读取请求。",
            ));
        }
        if progress.native_scope_digest.is_none() {
            progress.native_scope_digest = Some(observed);
            if let Some(view) = value.trusted.get("view").filter(|view| !view.is_null()) {
                let view: views::ViewOption =
                    serde_json::from_value(view.clone()).map_err(|_| http::protocol())?;
                if !view.valid() {
                    return Err(http::protocol());
                }
                progress.task_view = Some(view);
            }
            checkpoint(progress)?;
        }
        match scope::verify(rt, plan, client, params, &mut value, started, spent).await {
            Ok(()) => return Ok(value),
            Err(failure) if failure.code == "BUSINESS_CREDENTIAL_EXPIRED" && attempt == 0 => {
                continue;
            }
            Err(failure) if failure.code == "BUSINESS_CREDENTIAL_EXPIRED" => {
                return Err(business_auth_required());
            }
            Err(failure) => return Err(failure),
        }
    }
    Err(http::protocol())
}

fn items(plan: &Plan, body: &Value, trusted: &Value) -> Result<(Vec<Value>, usize)> {
    let response = &plan.execution["response"];
    if let Some(pointer) = response["successPointer"].as_str()
        && !pointer.is_empty()
        && body.pointer(pointer) != response.get("successValue")
    {
        return Err(Failure::new(
            "BUSINESS_REJECTED",
            6,
            "业务系统未确认本次查询成功。",
        ));
    }
    let selected = body
        .pointer(
            response["itemsPointer"]
                .as_str()
                .ok_or_else(catalog_invalid)?,
        )
        .ok_or_else(mismatch)?;
    let rows = if response["singleObject"] == true {
        vec![selected.clone()]
    } else {
        selected.as_array().cloned().ok_or_else(mismatch)?
    };
    let source_rows = rows.len();
    let projection = response["projection"]
        .as_object()
        .ok_or_else(catalog_invalid)?;
    let mut result = Vec::new();
    for row in rows {
        let row = if plan.execution["scope"]["kind"] == "authorized-shop-tree" {
            let Some(filtered) = scope::tree(&row, &plan.execution["scope"]["tree"], trusted, 0)?
            else {
                continue;
            };
            filtered
        } else {
            row
        };
        let scope = &plan.execution["scope"];
        match scope["kind"].as_str() {
            Some("authorized-shops") | Some("shops") if scope["bindSingleShop"] != true => {
                let code = row
                    .pointer(
                        scope["rowShopPointer"]
                            .as_str()
                            .ok_or_else(catalog_invalid)?,
                    )
                    .and_then(Value::as_str)
                    .ok_or_else(mismatch)?;
                let selected = trusted["selectedShopCodes"]
                    .as_array()
                    .ok_or_else(http::protocol)?;
                if !selected.iter().any(|value| value == code) {
                    if scope["kind"] == "authorized-shops" {
                        continue;
                    }
                    return Err(Failure::new(
                        "DATA_SCOPE_DENIED",
                        4,
                        "业务响应包含任务所选范围以外的门店。",
                    ));
                }
            }
            Some("authorized-shop-tree") => {}
            Some("shops" | "brand" | "employee" | "native") => {}
            _ => return Err(catalog_invalid()),
        }
        let mut fields = Map::new();
        for (name, mapping) in projection {
            let value = if let Some(pointer) = mapping.as_str() {
                row.pointer(pointer).cloned().unwrap_or(Value::Null)
            } else if mapping["source"] == "trusted" {
                trusted
                    .get(mapping["value"].as_str().ok_or_else(catalog_invalid)?)
                    .cloned()
                    .unwrap_or(Value::Null)
            } else {
                return Err(catalog_invalid());
            };
            fields.insert(name.clone(), value);
        }
        result.push(Value::Object(fields));
    }
    Ok((result, source_rows))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Progress {
    pub rows: Vec<Value>,
    pub next_page: u32,
    pub cursor: Value,
    pub seen_cursors: BTreeSet<String>,
    pub complete: bool,
    pub pages: u32,
    #[serde(default)]
    pub source_rows: usize,
    #[serde(default)]
    pub expected_total: Option<u64>,
    #[serde(default)]
    pub expected_pages: Option<u64>,
    pub spent_millis: u64,
    pub scope_digest: Option<String>,
    #[serde(default)]
    pub native_scope_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_view: Option<views::ViewOption>,
    pub as_of: Option<String>,
    #[serde(default)]
    pub read_started_at: Option<String>,
}
impl Default for Progress {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            next_page: 1,
            cursor: Value::Null,
            seen_cursors: BTreeSet::new(),
            complete: false,
            pages: 0,
            source_rows: 0,
            expected_total: None,
            expected_pages: None,
            spent_millis: 0,
            scope_digest: None,
            native_scope_digest: None,
            task_view: None,
            as_of: None,
            read_started_at: None,
        }
    }
}

pub(super) async fn call(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    cache: &Cache,
    operation: &Value,
    params: &Value,
) -> Result<Value> {
    collect(
        rt,
        p,
        env,
        client,
        cache,
        operation,
        params,
        &mut Progress::default(),
        |_| Ok(()),
    )
    .await
}

pub(super) async fn collect<F>(
    rt: &Runtime,
    p: &GovernedProfile,
    env: &profile::Environment,
    client: &reqwest::Client,
    cache: &Cache,
    operation: &Value,
    params: &Value,
    progress: &mut Progress,
    mut checkpoint: F,
) -> Result<Value>
where
    F: FnMut(&Progress) -> Result<()>,
{
    let plan = Plan::parse(operation, &p.environment)?;
    let started = tokio::time::Instant::now();
    if progress.pages == 0 && progress.read_started_at.is_none() {
        progress.read_started_at = Some(chrono::Utc::now().to_rfc3339());
    }
    let pagination = &plan.execution["pagination"];
    let mode = pagination["mode"].as_str().ok_or_else(catalog_invalid)?;
    let previous_millis = progress.spent_millis;
    let verification = verified_credential(
        &plan,
        rt,
        p,
        env,
        client,
        operation,
        params,
        started,
        previous_millis,
        progress,
        &mut checkpoint,
    )
    .await?;
    let task_view = verification
        .trusted
        .get("view")
        .cloned()
        .unwrap_or(Value::Null);
    let observed_scope = scope_digest(&verification.trusted)?;
    let mut initial_credential = Some(verification);
    if progress
        .scope_digest
        .as_ref()
        .is_some_and(|old| old != &observed_scope)
    {
        return Err(Failure::new(
            "DATA_SCOPE_DENIED",
            4,
            "任务原有数据范围已变化；保留原记录，停止继续读取。",
        ));
    }
    progress.scope_digest = Some(observed_scope);
    checkpoint(progress)?;
    for page_num in progress.next_page..=plan.limits.max_pages {
        if progress.complete {
            break;
        }
        if previous_millis.saturating_add(started.elapsed().as_millis() as u64)
            >= plan.limits.max_seconds.saturating_mul(1000)
        {
            return Err(limit());
        }
        let page = json!({"pageNum":page_num, "pageSize":pagination["pageSize"], "cursor":progress.cursor});
        let mut reply = None;
        let mut trusted = Value::Null;
        for attempt in 0..2 {
            let credential = match initial_credential.take() {
                Some(value) => value,
                None => {
                    verified_credential(
                        &plan,
                        rt,
                        p,
                        env,
                        client,
                        operation,
                        params,
                        started,
                        previous_millis,
                        progress,
                        &mut checkpoint,
                    )
                    .await?
                }
            };
            trusted = credential.trusted.clone();
            if progress.scope_digest.as_ref() != Some(&scope_digest(&trusted)?) {
                return Err(Failure::new(
                    "DATA_SCOPE_DENIED",
                    4,
                    "业务门店编号或仓库范围发生变化；保留已完成页。",
                ));
            }
            let remaining = plan.limits.max_seconds.saturating_mul(1000).saturating_sub(
                previous_millis.saturating_add(started.elapsed().as_millis() as u64),
            );
            if remaining == 0 {
                return Err(limit());
            }
            let observed = tokio::time::timeout(
                Duration::from_millis(remaining),
                pacing::send(rt, &plan, plan.request(client, params, &credential, &page)?),
            )
            .await
            .map_err(|_| limit())??;
            if observed.status == 401 && attempt == 0 {
                continue;
            }
            reply = Some(observed);
            break;
        }
        let reply = reply.ok_or_else(http::protocol)?;
        if reply.status
            != plan.execution["response"]["httpStatus"]
                .as_u64()
                .ok_or_else(catalog_invalid)? as u16
        {
            return Err(match reply.status {
                401 => business_auth_required(),
                403 => Failure::new("DATA_SCOPE_DENIED", 4, "业务系统拒绝当前身份或范围。"),
                429 => Failure::new("RATE_LIMITED", 5, "业务系统要求降低查询频率。"),
                _ => Failure::new(
                    "DEPENDENCY_UNAVAILABLE",
                    5,
                    "业务请求未取得已发布说明要求的响应。",
                ),
            });
        }
        let (page_rows, source_rows) = items(&plan, &reply.body, &trusted)?;
        if mode != "none"
            && source_rows as u64
                > pagination["pageSize"]
                    .as_u64()
                    .ok_or_else(catalog_invalid)?
        {
            return Err(mismatch());
        }
        let row_schema = operation["outputSchema"]
            .get("items")
            .cloned()
            .unwrap_or(json!(true));
        let validator = jsonschema::draft202012::new(&row_schema).map_err(|_| catalog_invalid())?;
        if page_rows.iter().any(|row| !validator.is_valid(row)) {
            return Err(mismatch());
        }
        if progress.source_rows.saturating_add(source_rows) > plan.limits.max_records {
            return Err(limit());
        }
        if progress.rows.len().saturating_add(page_rows.len()) > plan.limits.max_records {
            return Err(limit());
        }
        let mut next = progress.clone();
        next.rows.extend(page_rows);
        next.source_rows = next.source_rows.saturating_add(source_rows);
        next.pages = page_num;
        if serde_json::to_vec(&next.rows)
            .map_err(|_| mismatch())?
            .len()
            > plan.limits.max_result_bytes
        {
            return Err(limit());
        }
        next.complete = match mode {
            "none" => true,
            "page" => {
                let number = reply
                    .body
                    .pointer(
                        pagination["pageNumPointer"]
                            .as_str()
                            .ok_or_else(catalog_invalid)?,
                    )
                    .and_then(Value::as_u64)
                    .ok_or_else(mismatch)?;
                let count = reply
                    .body
                    .pointer(
                        pagination["pagesPointer"]
                            .as_str()
                            .ok_or_else(catalog_invalid)?,
                    )
                    .and_then(Value::as_u64)
                    .ok_or_else(mismatch)?;
                if number != u64::from(page_num)
                    || count > u64::from(plan.limits.max_pages)
                    || count != 0 && number > count
                    || count == 0 && source_rows != 0
                    || next
                        .expected_pages
                        .is_some_and(|previous| previous != count)
                {
                    return Err(mismatch());
                }
                next.expected_pages = Some(count);
                count == 0 || number >= count
            }
            "cursor" => {
                let next_cursor = reply
                    .body
                    .pointer(
                        pagination["nextCursorPointer"]
                            .as_str()
                            .ok_or_else(catalog_invalid)?,
                    )
                    .ok_or_else(mismatch)?
                    .clone();
                if next_cursor.is_null() || next_cursor == "" {
                    true
                } else {
                    let text = next_cursor
                        .as_str()
                        .filter(|v| {
                            v.len() <= crate::constants::query::DIRECT_CURSOR_BYTES
                                && !v.chars().any(|c| c < ' ')
                        })
                        .ok_or_else(mismatch)?;
                    if !next.seen_cursors.insert(text.to_owned()) {
                        return Err(mismatch());
                    }
                    next.cursor = next_cursor;
                    false
                }
            }
            _ => return Err(catalog_invalid()),
        };
        if let Some(pointer) = pagination["totalPointer"].as_str() {
            let total = reply
                .body
                .pointer(pointer)
                .and_then(Value::as_u64)
                .ok_or_else(mismatch)?;
            if total > plan.limits.max_records as u64
                || total < next.source_rows as u64
                || next
                    .expected_total
                    .is_some_and(|previous| previous != total)
                || next.complete && total != next.source_rows as u64
            {
                return Err(mismatch());
            }
            next.expected_total = Some(total);
        }
        next.next_page = page_num + 1;
        next.spent_millis = previous_millis.saturating_add(started.elapsed().as_millis() as u64);
        next.as_of = Some(chrono::Utc::now().to_rfc3339());
        *progress = next;
        checkpoint(progress)?;
        if progress.complete {
            break;
        }
    }
    if !progress.complete {
        return Err(limit());
    }
    let result = json!(progress.rows);
    if !cache.validators(operation)?.output.is_valid(&result) {
        return Err(mismatch());
    }
    Ok(
        json!({"items":result, "complete":true, "executionLocation":"local", "permissionVerified":true, "onlineVerified":true, "taskView":task_view,
        "_meta":{"contractVersion":operation["operationVersion"], "contractDigest":operation["contractDigest"],
        "state":"succeeded", "complete":true, "sideEffect":"none", "itemsReturned":result.as_array().map(Vec::len),
        "asOf":progress.as_of, "readStartedAt":progress.read_started_at,
        "snapshotConsistent":false, "pageCount":progress.pages}}),
    )
}
