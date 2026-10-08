use super::*;

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pagination {
    pub mode: String,
    pub page: i64,
    pub page_size: i64,
    pub pages_read: i64,
    pub matched_total: Option<i64>,
    pub items_fetched: i64,
    pub items_returned: i64,
    pub has_more: Option<bool>,
    pub next_page: Option<i64>,
    pub page_complete: bool,
    pub scope_complete: Option<bool>,
    pub truncated: bool,
    pub reason: Option<String>,
    pub content_complete: bool,
}

const PAGINATION_FIELDS: [&str; 14] = [
    "mode",
    "page",
    "pageSize",
    "pagesRead",
    "matchedTotal",
    "itemsFetched",
    "itemsReturned",
    "hasMore",
    "nextPage",
    "pageComplete",
    "scopeComplete",
    "truncated",
    "reason",
    "contentComplete",
];

pub(super) fn json_value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub(super) fn safe_pagination_value(field: &str, value: &Value) -> Option<(String, Value)> {
    match field {
        "mode" => {
            let mode = value.as_str()?;
            Some((
                "mode".to_owned(),
                json!(if ["page", "all"].contains(&mode) {
                    mode
                } else {
                    "other"
                }),
            ))
        }
        "page" | "pageSize" | "pagesRead" | "itemsFetched" | "itemsReturned" => {
            Some((field.to_owned(), json!(value.as_i64()?)))
        }
        "matchedTotal" | "nextPage" => Some((
            field.to_owned(),
            value.as_i64().map_or_else(
                || value.is_null().then_some(Value::Null),
                |number| Some(json!(number)),
            )?,
        )),
        "hasMore" | "pageComplete" | "truncated" | "contentComplete" => {
            Some((field.to_owned(), json!(value.as_bool()?)))
        }
        "scopeComplete" => Some((
            field.to_owned(),
            value.as_bool().map_or_else(
                || value.is_null().then_some(Value::Null),
                |flag| Some(json!(flag)),
            )?,
        )),
        "reason" if value.is_null() || value.is_string() => {
            Some(("reasonPresent".to_owned(), json!(!value.is_null())))
        }
        _ => None,
    }
}

/// A failure-only, allowlisted view of pagination metadata; never includes records or raw strings.
pub fn pagination_diagnostic(
    raw_meta: &Value,
    stage: &str,
    requested_page: i64,
    requested_page_size: i64,
    parsed_items: usize,
) -> Value {
    let raw_pagination = raw_meta.get("pagination");
    let object = raw_pagination.and_then(Value::as_object);
    let mut field_types = Map::new();
    let mut values = Map::new();
    let mut missing_fields = Vec::new();
    let unexpected_field_count = object.map_or(0, |fields| {
        fields
            .keys()
            .filter(|name| !PAGINATION_FIELDS.contains(&name.as_str()))
            .count()
    });

    for field in PAGINATION_FIELDS {
        match object.and_then(|fields| fields.get(field)) {
            Some(value) => {
                field_types.insert(field.to_owned(), json!(json_value_type(value)));
                if let Some((safe_field, safe_value)) = safe_pagination_value(field, value) {
                    values.insert(safe_field, safe_value);
                }
            }
            None => missing_fields.push(field),
        }
    }

    json!({
        "stage": stage,
        "paginationType": raw_pagination.map_or("missing", json_value_type),
        "fieldTypes": field_types,
        "missingFields": missing_fields,
        "unexpectedFieldCount": unexpected_field_count,
        "values": values,
        "request": {
            "page": requested_page,
            "pageSize": requested_page_size,
            "parsedItems": parsed_items,
        },
    })
}

#[derive(Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub fetched_at: String,
    pub source_synced_at: Option<String>,
    pub freshness: String,
    pub consistency: String,
}
pub(super) struct RawBody {
    pub(super) bytes: usize,
    pub(super) value: Option<Value>,
    pub(super) failure: Option<Failure>,
    pub(super) truncated: bool,
}
/// Reads at most `allowance` raw response bytes; the response is never parsed in part.
pub(super) async fn read_body(mut r: reqwest::Response, allowance: usize) -> RawBody {
    if r.content_length().is_some_and(|n| n as usize > allowance) {
        return RawBody {
            bytes: 0,
            value: None,
            failure: Some(over_limit()),
            truncated: true,
        };
    }
    let mut bytes = Vec::new();
    loop {
        match r.chunk().await {
            Ok(Some(c)) => {
                if bytes.len() + c.len() > allowance {
                    return RawBody {
                        bytes: bytes.len(),
                        value: None,
                        failure: Some(over_limit()),
                        truncated: true,
                    };
                }
                bytes.extend_from_slice(&c);
            }
            Ok(None) => break,
            Err(_) => {
                return RawBody {
                    bytes: bytes.len(),
                    value: None,
                    failure: Some(network()),
                    truncated: !bytes.is_empty(),
                };
            }
        }
    }
    match serde_json::from_slice(&bytes) {
        Ok(value) => RawBody {
            bytes: bytes.len(),
            value: Some(value),
            failure: None,
            truncated: false,
        },
        Err(_) => RawBody {
            bytes: bytes.len(),
            value: None,
            failure: Some(protocol()),
            truncated: true,
        },
    }
}
pub(super) fn retry_after(status: u16, r: &reqwest::Response) -> Option<Option<u64>> {
    if !matches!(status, 429 | 503) {
        return None;
    }
    Some(
        r.headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok()),
    )
}
pub(super) fn message(value: &Value) -> bool {
    value
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|v| !v.is_empty())
        && value
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|v| ["todo", "done", "cc"].contains(&v))
}
/// One structured business response: body, pagination, source, trace, bytes and normalised error.
#[derive(Default)]
pub struct PageResponse {
    pub status: u16,
    pub bytes: usize,
    pub data: Vec<Value>,
    pub pagination: Option<Pagination>,
    pub source: Option<Source>,
    pub raw_meta: Value,
    pub trace_id: Option<String>,
    pub failure: Option<Failure>,
    /// `Some(seconds)` when 429/503 carried a usable Retry-After, `Some(None)` when unusable.
    pub retry_after: Option<Option<u64>>,
    pub expired: bool,
    /// True when the obtained content is itself incomplete (cut short or not convertible).
    pub content_truncated: bool,
}
impl PageResponse {
    pub fn transported(&self) -> bool {
        self.status != 0
    }
}
/// Sends one workflow page request and interprets it up to the shared byte allowance.
pub async fn workflow_page(
    client: &reqwest::Client,
    env: &Environment,
    c: &Credentials,
    params: &Value,
    allowance: usize,
) -> PageResponse {
    let r = match client
        .post(format!(
            "{}{}",
            env.api_origin.trim_end_matches('/'),
            WORKFLOW_QUERY_ROUTE
        ))
        .bearer_auth(&c.access_token)
        .json(params)
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            return PageResponse {
                failure: Some(network()),
                ..Default::default()
            };
        }
    };
    let status = r.status().as_u16();
    let retry = retry_after(status, &r);
    let body = read_body(r, allowance).await;
    let mut response = PageResponse {
        status,
        bytes: body.bytes,
        retry_after: retry,
        ..Default::default()
    };
    let Some(mut value) = body.value else {
        response.failure = body.failure;
        response.content_truncated = body.truncated;
        return response;
    };
    response.trace_id = safe_trace(&value);
    response.raw_meta = value["meta"].clone();
    if status != 200 || value["ok"] != true {
        response.expired = status == 401 && value["error"]["code"] == "ACCESS_TOKEN_EXPIRED";
        response.failure = Some(with_trace(failure(status, &value), &response.trace_id));
        return response;
    }
    if value["schemaVersion"] != "1.0"
        || value["operationId"] != WORKFLOW_LIST_OPERATION
        || !value["error"].is_null()
    {
        response.failure = Some(with_trace(protocol(), &response.trace_id));
        response.content_truncated = true;
        return response;
    }
    let Some(items) = value["data"].as_array() else {
        response.failure = Some(with_trace(protocol(), &response.trace_id));
        response.content_truncated = true;
        return response;
    };
    if !items.iter().all(message) {
        response.failure = Some(with_trace(protocol(), &response.trace_id));
        response.content_truncated = true;
        return response;
    }
    if PAGINATION_FIELDS
        .iter()
        .any(|field| value["meta"]["pagination"].get(field).is_none())
    {
        response.failure = Some(with_trace(pagination_unknown(), &response.trace_id));
        response.content_truncated = true;
        return response;
    }
    let pagination = match serde_json::from_value::<Pagination>(value["meta"]["pagination"].clone())
    {
        Ok(pagination) => pagination,
        Err(_) => {
            response.failure = Some(with_trace(pagination_unknown(), &response.trace_id));
            response.content_truncated = true;
            return response;
        }
    };
    if !value["meta"]["source"].is_null() {
        match serde_json::from_value::<Source>(value["meta"]["source"].clone()) {
            Ok(source) => response.source = Some(source),
            Err(_) => {
                response.failure = Some(with_trace(protocol(), &response.trace_id));
                response.content_truncated = true;
                return response;
            }
        }
    }
    if let Value::Array(items) = value["data"].take() {
        response.data = items;
    }
    response.pagination = Some(pagination);
    response
}
