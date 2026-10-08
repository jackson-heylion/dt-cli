use crate::{
    Runtime, http,
    output::{Executed, Failure, Result},
    pagination::{self, Bounds, Cancel, Limits},
    profile,
    profiles::Binding,
};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

fn parts(mut result: Executed) -> (Value, Value, Option<Failure>) {
    if let Some(error) = result.error.as_mut() {
        let data = error.partial_data.take().map(|v| *v).unwrap_or(result.data);
        let meta = error.partial_meta.take().map(|v| *v).unwrap_or(result.meta);
        (data, meta, result.error)
    } else {
        (result.data, result.meta, None)
    }
}

fn not_read(failure: Failure, binding: &Binding, params: &Value, kinds: &[&str]) -> Failure {
    let counts: Map<String, Value> = kinds
        .iter()
        .map(|kind| (kind.to_string(), Value::Null))
        .collect();
    let sources: Vec<_> = kinds
        .iter()
        .map(|kind| json!({"kind":kind,"state":"not-read","complete":false}))
        .collect();
    failure.with_partial(json!({"taskId":"inbox.brief","runId":Value::Null,"result":{"counts":counts,"items":[],"groups":[],"filters":params,"complete":false}}),
        json!({"taskSchemaVersion":1,"recipeVersion":1,"effect":"read","complete":false,"observedAt":chrono::Utc::now().to_rfc3339(),"sources":sources,"binding":binding}))
}

/// All kinds use one identity, connection pool, cancellation listener and budget.
pub(super) async fn run(rt: &Runtime, binding: &Binding, params: &Value) -> Result<Value> {
    let deadline = Instant::now() + rt.aggregate_budget.min(Duration::from_secs(30));
    let mut cancel = Cancel::signal();
    let p = profile::read(&rt.root, &binding.profile)?
        .ok_or_else(|| Failure::new("PROFILE_NOT_CONFIGURED", 2, "profile尚未配置。"))?;
    if p.subject_id != binding.subject_id
        || p.environment != binding.environment
        || p.authorization_id != binding.authorization_id
    {
        return Err(Failure::new(
            "IDENTITY_MISMATCH",
            4,
            "执行前账号绑定发生变化；未读取凭证。",
        ));
    }
    let env = rt.environment(&p.environment)?;
    let client = http::client()?;
    let kinds: Vec<_> = params["kinds"]
        .as_array()
        .map(|k| k.iter().filter_map(Value::as_str).collect())
        .unwrap_or_else(|| vec!["todo"]);
    let mut credentials = match cancel
        .race(deadline, rt.credentials(&p, env, &client))
        .await
    {
        Ok(Ok(credentials)) => credentials,
        Ok(Err(failure)) | Err(failure) => return Err(not_read(failure, binding, params, &kinds)),
    };
    let max_pages = params["maxPages"].as_u64().unwrap_or(20) as u32;
    let max_items = params["maxItems"].as_u64().unwrap_or(1000) as usize;
    let mut remaining = Bounds {
        limits: Limits {
            page_size: 50,
            max_pages,
            max_items,
        },
        bytes: http::AGGREGATE_BYTE_LIMIT,
    };
    let mut items = Vec::new();
    let mut sources = Vec::new();
    let mut counts = Map::new();
    let mut failure = None;
    let mut successful_kinds = 0;
    let mut complete = true;
    let mut filters = params.clone();
    filters["kinds"] = json!(kinds);
    filters["maxPages"] = json!(max_pages);
    filters["maxItems"] = json!(max_items);
    for kind in kinds {
        if failure.is_some()
            || remaining.limits.max_pages == 0
            || remaining.limits.max_items == 0
            || remaining.bytes == 0
            || Instant::now() >= deadline
        {
            counts.insert(kind.into(), Value::Null);
            sources.push(json!({"kind":kind,"state":"not-read","complete":false}));
            complete = false;
            continue;
        }
        let mut request = params.clone();
        let obj = request.as_object_mut().unwrap();
        for field in ["kinds", "maxPages", "maxItems"] {
            obj.remove(field);
        }
        obj.insert("kind".into(), json!(kind));
        obj.insert("sort".into(), json!("arrival-desc"));
        let (data, meta, mut error) = parts(
            pagination::aggregate_bounded(
                rt,
                &mut cancel,
                deadline,
                &client,
                &p,
                env,
                &mut credentials,
                &request,
                remaining,
            )
            .await,
        );
        let pages = meta["pagination"]["pagesRead"].as_u64().unwrap_or(0) as u32;
        remaining.limits.max_pages = remaining.limits.max_pages.saturating_sub(pages);
        remaining.bytes = remaining
            .bytes
            .saturating_sub(meta["budget"]["bytesRead"].as_u64().unwrap_or(0) as usize);
        let mut records = data.as_array().cloned().unwrap_or_default();
        if records.iter().any(|item| item["kind"] != kind) {
            records.retain(|item| item["kind"] == kind);
            error = Some(http::protocol());
        }
        remaining.limits.max_items = remaining.limits.max_items.saturating_sub(records.len());
        let kind_complete = error.is_none() && meta["pagination"]["scopeComplete"] == true;
        if error.is_none() {
            successful_kinds += 1;
        }
        counts.insert(
            kind.into(),
            if data.is_null() {
                Value::Null
            } else {
                json!(records.len())
            },
        );
        for mut item in records {
            if let Some(id) = item["id"]
                .as_str()
                .filter(|id| crate::open::canonical_id(id).is_ok())
            {
                item["openAction"] = json!({"id":"open-selected-workflow","actor":"employee","kind":"command",
                    "argv":["dt-cli","workflow","open","--id",id,"--profile",binding.profile],"requiresInteraction":true,"reason":"仅在用户明确选中此流程后打开"});
            }
            items.push(item);
        }
        sources.push(json!({"kind":kind,"state":if kind_complete{"complete"}else{"partial"},"complete":kind_complete,
            "pagination":meta["pagination"],"source":meta["source"],"traceId":meta["traceId"],"errorCode":error.as_ref().map(|e|e.code)}));
        complete &= kind_complete;
        if let Some(error) = error {
            failure = Some(error);
        }
    }
    let mut groups: BTreeMap<(String, String), Vec<Value>> = BTreeMap::new();
    for item in &items {
        groups
            .entry((
                item["applicationId"].as_str().unwrap_or("unknown").into(),
                item["kind"].as_str().unwrap_or("unknown").into(),
            ))
            .or_default()
            .push(item["id"].clone());
    }
    let groups:Vec<_>=groups.into_iter().map(|((application_id,kind),records)|json!({"applicationId":application_id,"kind":kind,"count":records.len(),"itemIds":records})).collect();
    let result = json!({"counts":counts,"items":items,"groups":groups,"filters":filters,"sort":"arrival-desc-within-kind",
        "complete":complete,"limitations":["有界查询，非同一时点快照","缺失字段保持未知，不推断逾期或审批优先级"]});
    let data = json!({"taskId":"inbox.brief","runId":Value::Null,"result":result});
    let meta = json!({"taskSchemaVersion":1,"recipeVersion":1,"effect":"read","complete":complete,"onlineVerified":true,"permissionVerified":complete,"availability":{"state":if complete{"ready"}else{"unknown"},"reason":if complete{"verified-current-permissions"}else{"partial-result"}},"observedAt":chrono::Utc::now().to_rfc3339(),
        "sources":sources,"actions":[],"binding":binding,"budget":{"pagesRead":max_pages-remaining.limits.max_pages,
            "itemsReturned":max_items-remaining.limits.max_items,"bytesRead":http::AGGREGATE_BYTE_LIMIT-remaining.bytes}});
    if !complete {
        let error = if successful_kinds == 0
            && data["result"]["items"]
                .as_array()
                .is_some_and(Vec::is_empty)
        {
            failure.unwrap_or_else(|| Failure::new("RESULT_LIMIT", 7, "工作简报达到共享预算上限。"))
        } else {
            Failure::new(
                "PARTIAL_RESULT",
                7,
                "工作简报未完整，已保留取得的清单并标明未读取类别。",
            )
        };
        return Err(error.with_partial(data, meta));
    }
    let mut data = data;
    data["_meta"] = meta;
    Ok(data)
}
