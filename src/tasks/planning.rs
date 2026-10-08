use super::definitions::{self, Definition};
use crate::{
    Runtime, governed,
    output::{Failure, Result},
    profiles::{self, Binding},
};
use serde_json::{Value, json};

fn context(
    rt: &Runtime,
    task: &Definition,
    profile: Option<&str>,
    version: Option<&str>,
    params: Option<&Value>,
) -> Result<(Binding, Option<Value>)> {
    let binding = profiles::resolve(
        rt,
        profile,
        &task.provider,
        task.system_id.as_deref(),
        task.effect == "write",
    )?;
    let contract = if task.provider == "governed" {
        let api_params = params.map(|p| {
            let mut value = p.clone();
            if task.task_id == "order-config.compare" {
                value.as_object_mut().unwrap().remove("shopCodes");
            }
            value
        });
        let contract = governed::task_contract(
            rt,
            &binding.profile,
            &task.operation_id,
            version,
            &task.effect,
            api_params.as_ref(),
        )?;
        compatible(task, &contract)?;
        Some(contract)
    } else {
        None
    };
    Ok((binding, contract))
}
pub(super) fn compatible(task: &Definition, contract: &Value) -> Result<()> {
    let fields = contract["outputSchema"]["items"]["properties"]
        .as_object()
        .or_else(|| contract["outputSchema"]["properties"].as_object());
    if fields.is_none_or(|fields| {
        task.required_output_fields
            .iter()
            .any(|key| !fields.contains_key(key))
    }) {
        return Err(Failure::new(
            "TASK_CONTRACT_UNSUPPORTED",
            6,
            "当前合同不能提供此任务所需的输出字段。",
        ));
    }
    Ok(())
}
fn availability(
    rt: &Runtime,
    task: &Definition,
    profile: Option<&str>,
    version: Option<&str>,
) -> Value {
    match context(rt, task, profile, version, None) {
        Ok((binding, contract)) => {
            json!({"state":"unknown","reason":"offline-not-verified","binding":binding,
            "operationVersion":contract.as_ref().map(|c|&c["operationVersion"]),"permissionVerified":false})
        }
        Err(error) => {
            json!({"state":if matches!(error.code,"CLIENT_UPGRADE_REQUIRED"|"TASK_CONTRACT_UNSUPPORTED"){"unsupported"}else{"blocked"},"reason":error.code,"permissionVerified":false})
        }
    }
}
pub(super) fn list(rt: &Runtime, query: Option<&str>, profile: Option<&str>) -> Result<Value> {
    let query = query.unwrap_or("").to_lowercase();
    let mut matches: Vec<_> = definitions::all()
        .iter()
        .filter_map(|task| task.rank(&query).map(|rank| (rank, task)))
        .collect();
    matches.sort_by(|(a, x), (b, y)| a.cmp(b).then(x.task_id.cmp(&y.task_id)));
    let tasks: Vec<_> = matches
        .into_iter()
        .map(|(_, task)| {
            let mut v = task.view();
            v["availability"] = availability(rt, task, profile, None);
            v
        })
        .collect();
    let suggestion = if tasks.is_empty() {
        json!("试试待办、门店配置或点赞")
    } else {
        Value::Null
    };
    Ok(
        json!({"taskSchemaVersion":1,"tasks":tasks,"onlineVerified":false,"credentialReads":0,"httpRequests":0,"suggestion":suggestion}),
    )
}
pub(super) fn show(
    rt: &Runtime,
    id: &str,
    profile: Option<&str>,
    version: Option<&str>,
) -> Result<Value> {
    let task = definitions::get(id)?;
    let mut value = task.view();
    value["availability"] = availability(rt, task, profile, version);
    Ok(value)
}
pub(super) fn plan(
    rt: &Runtime,
    id: &str,
    profile: Option<&str>,
    version: Option<&str>,
    raw: &str,
) -> Result<Value> {
    let task = definitions::get(id)?;
    let params = task.validate(raw)?;
    let (binding, contract) = context(rt, task, profile, version, Some(&params))?;
    let limits=contract.as_ref().map(|c|c["limits"].clone()).unwrap_or_else(||json!({"maxPages":params["maxPages"].as_u64().unwrap_or(20),"maxItems":params["maxItems"].as_u64().unwrap_or(1000),"pageBytes":2097152,"aggregateBytes":10485760,"seconds":30}));
    let steps = match id {
        "inbox.brief" => vec!["读取本人流程（共享总预算）", "按来源与类别整理，保留完整性"],
        "order-config.compare" => vec![
            "提交一次有权范围的批量读取",
            "有限等待并取完整结果",
            "筛选选定门店并比较真实字段",
        ],
        _ => vec![
            "准备本次意图并保存ID",
            "授权固定参数和合同摘要（不发送）",
            "单次派发，未知只查询原ID",
        ],
    };
    Ok(
        json!({"taskId":id,"runId":Value::Null,"result":{"preview":true,"steps":steps,"binding":binding,
        "operationId":task.operation_id,"operationVersion":contract.as_ref().map(|c|&c["operationVersion"]),
        "contractDigest":contract.as_ref().map(|c|&c["contractDigest"]),"effect":task.effect,
        "limits":limits,"permissionVerified":false,"credentialReads":0,
        "httpRequests":0,"browserOpened":false,"recordWrites":0},
        "_meta":{"taskSchemaVersion":task.task_schema_version,"recipeVersion":task.recipe_version,"effect":task.effect,
            "complete":true,"observedAt":chrono::Utc::now().to_rfc3339(),"sources":[],"actions":[]}}),
    )
}
