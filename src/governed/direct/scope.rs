//! Registered, same-origin ownership reads. AUTH shop identifiers never stand in for business identifiers.
use super::*;

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Preflight {
    shop_directory: Option<Directory>,
    warehouse: Option<Warehouse>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Directory {
    path: String,
    page_size: u32,
    max_pages: u32,
    read_only_evidence: Evidence,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Warehouse {
    path: String,
    shop_path: String,
    read_only_evidence: Evidence,
    shop_read_only_evidence: Evidence,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Evidence {
    source_commit: String,
    source_path: String,
    notes: String,
}
impl Evidence {
    fn valid(&self) -> bool {
        self.source_commit.len() == 40
            && self
                .source_commit
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            && !self.source_path.trim().is_empty()
            && !self.notes.trim().is_empty()
    }
}
fn configuration(plan: &Plan) -> Result<Preflight> {
    plan.execution
        .get("preflight")
        .map(|value| serde_json::from_value(value.clone()).map_err(|_| catalog_invalid()))
        .unwrap_or_else(|| Ok(Preflight::default()))
}
fn safe_path(path: &str, identifier: bool) -> bool {
    path.starts_with('/')
        && !path.contains("//")
        && !path.contains("..")
        && !path.contains(['%', '?', '#', '\\'])
        && if identifier {
            path.ends_with("/{id}")
                && path.matches("{id}").count() == 1
                && !path.replace("{id}", "1").contains(['{', '}'])
        } else {
            !path.contains(['{', '}'])
        }
}
pub(super) fn validate(plan: &Plan) -> Result<()> {
    let preflight = configuration(plan)?;
    let directory = plan.execution["scope"]["kind"] == "authorized-shop-tree";
    let warehouse = plan.execution["scope"].get("warehouseParameter").is_some();
    if preflight.shop_directory.is_some() != directory || preflight.warehouse.is_some() != warehouse
    {
        return Err(catalog_invalid());
    }
    if let Some(value) = preflight.shop_directory {
        if !safe_path(&value.path, false)
            || value.page_size == 0
            || value.page_size > crate::constants::query::BUSINESS_PAGE_SIZE
            || value.max_pages == 0
            || value.max_pages > crate::constants::query::SCOPE_MAX_PAGES
            || !value.read_only_evidence.valid()
        {
            return Err(catalog_invalid());
        }
    }
    if let Some(value) = preflight.warehouse {
        if !safe_path(&value.path, true)
            || !safe_path(&value.shop_path, true)
            || !value.read_only_evidence.valid()
            || !value.shop_read_only_evidence.valid()
        {
            return Err(catalog_invalid());
        }
    }
    Ok(())
}
fn denied() -> Failure {
    Failure::new(
        "DATA_SCOPE_DENIED",
        4,
        "业务门店编号或仓库归属不符合本人原任务范围。",
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShopPage {
    list: Vec<Shop>,
    page_num: u32,
    pages: u32,
    total: u64,
}
#[derive(Deserialize)]
struct Shop {
    id: i32,
    code: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WarehouseRow {
    id: i32,
    owner_id: i32,
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Deserialize)]
struct ShopCode {
    code: String,
}

async fn get(
    rt: &Runtime,
    plan: &Plan,
    client: &reqwest::Client,
    params: &Value,
    credential: &Credential,
    path: &str,
    query: Value,
    started: tokio::time::Instant,
    spent: u64,
) -> Result<Value> {
    let remaining = plan
        .limits
        .max_seconds
        .saturating_mul(1000)
        .saturating_sub(spent.saturating_add(started.elapsed().as_millis() as u64));
    if remaining == 0 {
        return Err(limit());
    }
    let mut lookup = plan.clone();
    lookup.path = path.into();
    lookup.method = "GET".into();
    lookup.execution["request"] = json!({"query":query});
    let request = lookup
        .request(client, params, credential, &Value::Null)?
        .timeout(Duration::from_millis(
            remaining.min(plan.limits.timeout_seconds * 1000),
        ));
    let response = tokio::time::timeout(
        Duration::from_millis(remaining),
        pacing::send(rt, plan, request),
    )
    .await
    .map_err(|_| limit())??;
    match response.status {
        200 => Ok(response.body),
        401 => Err(Failure::new(
            "BUSINESS_CREDENTIAL_EXPIRED",
            3,
            "核对数据范围时临时业务凭证未被接受。",
        )),
        403 | 404 => Err(denied()),
        429 => Err(Failure::new(
            "RATE_LIMITED",
            5,
            "业务系统要求降低核对频率。",
        )),
        _ => Err(Failure::new(
            "DEPENDENCY_UNAVAILABLE",
            5,
            "业务范围核对未取得可用响应。",
        )),
    }
}

pub(super) async fn verify(
    rt: &Runtime,
    plan: &Plan,
    client: &reqwest::Client,
    params: &Value,
    credential: &mut Credential,
    started: tokio::time::Instant,
    spent: u64,
) -> Result<()> {
    let preflight = configuration(plan)?;
    if let Some(directory) = preflight.shop_directory {
        let allowed = credential.trusted["authorizedShopCodes"]
            .as_array()
            .ok_or_else(http::protocol)?;
        let mut ids = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut count = 0u64;
        let mut expected: Option<(u32, u64)> = None;
        let mut complete = false;
        for number in 1..=directory.max_pages {
            let body=get(rt,plan,client,params,credential,&directory.path,json!({
                "pageNum":{"source":"fixed","value":number},"pageSize":{"source":"fixed","value":directory.page_size}
            }),started,spent).await?;
            let page: ShopPage = serde_json::from_value(body).map_err(|_| mismatch())?;
            if page.page_num != number
                || page.pages > directory.max_pages
                || page.list.len() > directory.page_size as usize
                || page.pages != 0 && number > page.pages
                || page.pages == 0 && !page.list.is_empty()
                || expected.is_some_and(|value| value != (page.pages, page.total))
            {
                return Err(mismatch());
            }
            expected = Some((page.pages, page.total));
            count = count.saturating_add(page.list.len() as u64);
            if count > plan.limits.max_records as u64 || page.total > plan.limits.max_records as u64
            {
                return Err(limit());
            }
            for shop in page.list {
                if shop.id <= 0 || shop.code.is_empty() || !seen.insert(shop.id) {
                    return Err(mismatch());
                }
                if allowed.iter().any(|code| code == &shop.code) {
                    ids.insert(shop.id);
                }
            }
            if number >= page.pages {
                if count != page.total {
                    return Err(mismatch());
                }
                complete = true;
                break;
            }
        }
        if !complete {
            return Err(limit());
        }
        credential.trusted["authorizedShopIds"] = json!(ids);
    }
    if let Some(warehouse) = preflight.warehouse {
        let id = credential.trusted["warehouseId"]
            .as_i64()
            .filter(|id| *id > 0)
            .ok_or_else(denied)?;
        let path = warehouse.path.replace("{id}", &id.to_string());
        let row: WarehouseRow = serde_json::from_value(
            get(
                rt,
                plan,
                client,
                params,
                credential,
                &path,
                json!({}),
                started,
                spent,
            )
            .await?,
        )
        .map_err(|_| mismatch())?;
        if i64::from(row.id) != id || row.owner_id <= 0 || row.kind != "SHOP" {
            return Err(denied());
        }
        let path = warehouse
            .shop_path
            .replace("{id}", &row.owner_id.to_string());
        let shop: ShopCode = serde_json::from_value(
            get(
                rt,
                plan,
                client,
                params,
                credential,
                &path,
                json!({}),
                started,
                spent,
            )
            .await?,
        )
        .map_err(|_| mismatch())?;
        for field in ["selectedShopCodes", "authorizedShopCodes"] {
            if !credential.trusted[field]
                .as_array()
                .is_some_and(|codes| codes.iter().any(|code| code == &shop.code))
            {
                return Err(denied());
            }
        }
        credential.trusted["warehouseOwnerId"] = json!(row.owner_id);
        credential.trusted["warehouseShopCode"] = json!(shop.code);
    }
    Ok(())
}

pub(super) fn tree(
    node: &Value,
    rules: &Value,
    trusted: &Value,
    depth: u32,
) -> Result<Option<Value>> {
    if !node.is_object() || depth > crate::constants::query::SCOPE_TREE_DEPTH {
        return Err(mismatch());
    }
    let allowed = trusted["authorizedShopIds"].as_array().ok_or_else(denied)?;
    let mut output = Map::new();
    for field in rules["nodeFields"].as_array().ok_or_else(catalog_invalid)? {
        let name = field.as_str().ok_or_else(catalog_invalid)?;
        output.insert(name.into(), node.get(name).cloned().unwrap_or(Value::Null));
    }
    let shops_name = rules["shopsField"].as_str().ok_or_else(catalog_invalid)?;
    let id_name = rules["shopIdField"].as_str().ok_or_else(catalog_invalid)?;
    let mut shops = Vec::new();
    if let Some(source) = node.get(shops_name).filter(|value| !value.is_null()) {
        for shop in source.as_array().ok_or_else(mismatch)? {
            let id = shop[id_name]
                .as_i64()
                .filter(|id| *id > 0)
                .ok_or_else(mismatch)?;
            if !allowed.iter().any(|allowed| allowed.as_i64() == Some(id)) {
                continue;
            }
            let mut visible = Map::new();
            for field in rules["shopFields"].as_array().ok_or_else(catalog_invalid)? {
                let name = field.as_str().ok_or_else(catalog_invalid)?;
                visible.insert(name.into(), shop.get(name).cloned().unwrap_or(Value::Null));
            }
            shops.push(Value::Object(visible));
        }
    }
    let children_name = rules["childrenField"]
        .as_str()
        .ok_or_else(catalog_invalid)?;
    let mut children = Vec::new();
    for child in node[children_name].as_array().ok_or_else(mismatch)? {
        if let Some(visible) = tree(child, rules, trusted, depth + 1)? {
            children.push(visible);
        }
    }
    if shops.is_empty() && children.is_empty() {
        return Ok(None);
    }
    output.insert(shops_name.into(), json!(shops));
    output.insert(children_name.into(), json!(children));
    Ok(Some(Value::Object(output)))
}
