use crate::{
    catalog::{self, Catalog, Operation, flag, string},
    input,
    output::{Failure, Result, invalid},
    pagination::{Limits, MAX_ITEMS_LIMIT, MAX_PAGES_LIMIT},
};
use clap::parser::ValueSource;
use serde_json::{Map, Value, json};
pub(crate) fn api_parameters(catalog: &Catalog, operation: &str, raw: &str) -> Result<Value> {
    let operation = catalog.operation(operation)?;
    let parsed = input::strict_json(raw)?;
    let object = parsed.as_object().ok_or_else(invalid)?;
    let catalog_raw = catalog::raw();
    let schema = catalog_raw["operations"]
        .as_array()
        .and_then(|operations| {
            operations
                .iter()
                .find(|item| item["operationId"] == operation.operation_id)
        })
        .and_then(|item| item["apiParametersSchema"].as_object())
        .ok_or_else(invalid)?;
    let properties = schema["properties"].as_object().ok_or_else(invalid)?;
    if object.keys().any(|key| !properties.contains_key(key)) {
        return Err(invalid());
    }
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for item in required {
            if !item.as_str().is_some_and(|key| object.contains_key(key)) {
                return Err(invalid());
            }
        }
    }
    for (key, value) in object {
        let kind = properties[key]["type"].as_str().ok_or_else(invalid)?;
        let valid = match (kind, value) {
            ("string", Value::String(_)) => true,
            ("integer", Value::Number(number)) => number.is_i64(),
            ("boolean", Value::Bool(_)) => true,
            _ => false,
        };
        if !valid {
            return Err(invalid());
        }
    }
    Ok(parsed)
}
/// A bounded aggregation request, validated before any profile, credential or network access.
pub struct WorkflowPlan {
    pub(crate) params: Value,
    pub(crate) limits: Limits,
    pub(crate) all: bool,
    pub(crate) page: i64,
}
/// Full client-side preparation for one invocation.
pub(crate) struct Prepared {
    pub(crate) plan: Option<WorkflowPlan>,
    pub(crate) api: Option<Value>,
    pub(crate) api_raw: Option<String>,
    /// A validated message id when the generic call targets `iam.workflow.open`.
    pub(crate) open_id: Option<String>,
}
impl Prepared {
    pub(crate) fn none() -> Self {
        Self {
            plan: None,
            api: None,
            api_raw: None,
            open_id: None,
        }
    }
}
fn explicit(m: &clap::ArgMatches, key: &str) -> bool {
    matches!(m.value_source(key), Some(ValueSource::CommandLine))
}
fn integer(object: &Map<String, Value>, key: &str) -> Option<i64> {
    object.get(key).and_then(Value::as_i64)
}
/// Splits client-only aggregation fields from the API body and applies the fixed ceilings.
fn workflow_plan(
    mut object: Map<String, Value>,
    all: bool,
    page_explicit: bool,
) -> Result<WorkflowPlan> {
    let given_pages = object.contains_key("maxPages");
    let given_items = object.contains_key("maxItems");
    if !all && (given_pages || given_items) {
        return Err(invalid());
    }
    let page = integer(&object, "page").unwrap_or(1);
    if !(1..=i32::MAX as i64).contains(&page) {
        return Err(invalid());
    }
    if all && page_explicit && page != 1 {
        return Err(invalid());
    }
    let page_size = integer(&object, "pageSize").unwrap_or(50);
    if !(1..=200).contains(&page_size) {
        return Err(invalid());
    }
    let max_pages = match integer(&object, "maxPages") {
        Some(value) => u32::try_from(value).map_err(|_| invalid())?,
        None => MAX_PAGES_LIMIT,
    };
    let max_items = match integer(&object, "maxItems") {
        Some(value) => usize::try_from(value).map_err(|_| invalid())?,
        None => MAX_ITEMS_LIMIT,
    };
    if !(1..=MAX_PAGES_LIMIT).contains(&max_pages) || !(1..=MAX_ITEMS_LIMIT).contains(&max_items) {
        return Err(invalid());
    }
    if all {
        object.remove("all");
        object.remove("maxPages");
        object.remove("maxItems");
        object.insert("page".into(), json!(1));
    }
    object.insert("pageSize".into(), json!(page_size));
    Ok(WorkflowPlan {
        params: Value::Object(object),
        limits: Limits {
            page_size,
            max_pages,
            max_items,
        },
        all,
        page,
    })
}
/// Builds the workflow API body from dedicated command flags.
fn workflow_flags(op: &Operation, leaf: &clap::ArgMatches) -> Result<Map<String, Value>> {
    let mut params = Map::new();
    for p in &op.parameters {
        if p.name != "profile"
            && p.name != "dry-run"
            && p.name != "all"
            && p.name != "max-pages"
            && p.name != "max-items"
            && let Some(v) = string(leaf, &p.name)
        {
            params.insert(
                p.name.replace('-', "_"),
                if p.kind == "integer" {
                    json!(v.parse::<i64>().map_err(|_| invalid())?)
                } else {
                    json!(v)
                },
            );
        }
    }
    for (a, b) in [
        ("application_id", "applicationId"),
        ("form_type", "formType"),
        ("started_from", "startedFrom"),
        ("started_to", "startedTo"),
        ("page_size", "pageSize"),
    ] {
        if let Some(v) = params.remove(a) {
            params.insert(b.into(), v);
        }
    }
    let all = flag(leaf, "all");
    if all {
        params.insert("all".into(), json!(true));
    }
    for (flag_name, field) in [("max-pages", "maxPages"), ("max-items", "maxItems")] {
        if explicit(leaf, flag_name)
            && let Some(value) = string(leaf, flag_name)
        {
            params.insert(
                field.into(),
                json!(value.parse::<i64>().map_err(|_| invalid())?),
            );
        }
    }
    Ok(params)
}
/// Validates parameters with no side effects: no profile, keyring, refresh or HTTP access.
pub(crate) fn prepare(
    catalog: &Catalog,
    op: &Operation,
    leaf: &clap::ArgMatches,
) -> Result<Prepared> {
    let selected = string(leaf, "operation");
    if op.operation_id == "iam.workflow.open" {
        let mut prepared = Prepared::none();
        prepared.open_id = Some(string(leaf, "id").ok_or_else(invalid)?.to_owned());
        // The identifier is validated against the same canonical rule the command uses.
        crate::open::canonical_id(prepared.open_id.as_deref().unwrap())?;
        return Ok(prepared);
    }
    if op.operation_id == "iam.workflow.list" {
        let all = flag(leaf, "all");
        let plan = workflow_plan(workflow_flags(op, leaf)?, all, explicit(leaf, "page"))?;
        let mut prepared = Prepared::none();
        prepared.plan = Some(plan);
        return Ok(prepared);
    }
    if op.operation_id != "api.call" {
        return Ok(Prepared::none());
    }
    let business = catalog.operation(selected.ok_or_else(invalid)?)?;
    if business.route.is_none() || Some(business.operation_id.as_str()) != selected {
        return Err(Failure::new(
            "UNKNOWN_OPERATION",
            2,
            "该操作不支持通用调用。",
        ));
    }
    let raw = Some(input::read_parameters(leaf)?.unwrap_or_else(|| "{}".to_owned()));
    let mut api = None;
    let mut plan = None;
    let mut open_id = None;
    if let Some(text) = raw.as_deref() {
        let parsed = api_parameters(catalog, selected.unwrap(), text)?;
        if business.operation_id == "iam.workflow.list" {
            let object = parsed.as_object().cloned().ok_or_else(invalid)?;
            let all = object.get("all").and_then(Value::as_bool).unwrap_or(false);
            let page_explicit = object.contains_key("page");
            plan = Some(workflow_plan(object, all, page_explicit)?);
        }
        if business.operation_id == "iam.workflow.open" {
            let id = parsed["id"].as_str().ok_or_else(invalid)?;
            crate::open::canonical_id(id)?;
            open_id = Some(id.to_owned());
        }
        api = Some(parsed);
    }
    Ok(Prepared {
        plan,
        api,
        api_raw: raw,
        open_id,
    })
}
