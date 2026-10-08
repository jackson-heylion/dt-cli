use super::*;

pub(super) fn reply(status: u16, body: Value) -> Reply {
    (status, Vec::new(), serde_json::to_vec(&body).unwrap())
}
pub(super) fn ok(operation: &str, data: Value) -> Value {
    json!({"schemaVersion":"1.0","operationId":operation,"ok":true,"profile":null,
        "data":data,"error":null,"meta":{"traceId":"trace-ok"}})
}
pub(super) fn failure(status: u16, code: &str) -> Reply {
    reply(
        status,
        json!({"schemaVersion":"1.0","operationId":null,"ok":false,"profile":null,"data":null,
            "error":{"code":code,"category":"auth","message":"denied","retryable":false,"hint":"none"},
            "meta":{"traceId":"trace-denied"}}),
    )
}
pub(super) fn oauth_error(status: u16, error: &str, code: &str) -> Reply {
    reply(status, json!({"error":error,"code":code}))
}

pub(super) fn digest(seed: &str) -> String {
    format!("{:x}", Sha256::digest(seed.as_bytes()))
}
pub(super) fn items() -> Value {
    json!([{"shopCode":"2021003","itemCode":"101010009","itemName":"白芝麻(嘉亮)25kg/件",
        "specification":"25kg/件","orderUnit":"件","minOrderNum":"1","maxOrderNum":null,
        "orderNumMultiplier":"1","enable":true,"orderable":true}])
}
pub(super) fn output_schema() -> Value {
    json!({"type":"array","items":{"type":"object","additionalProperties":false,
        "required":["shopCode","itemCode","itemName","minOrderNum"],
        "properties":{"shopCode":{"type":"string"},"itemCode":{"type":"string"},
            "itemName":{"type":"string"},"specification":{"type":["string","null"]},
            "orderUnit":{"type":["string","null"]},"minOrderNum":{"type":["string","null"]},
            "maxOrderNum":{"type":["string","null"]},"orderNumMultiplier":{"type":["string","null"]},
            "enable":{"type":"boolean"},"orderable":{"type":"boolean"}}}})
}
pub(super) fn operation(id: &str, version: &str, consented: bool) -> Value {
    let input = if id == BATCH {
        json!({"type":"object","additionalProperties":false,
            "properties":{"locatedPartitionId":{"type":"integer","minimum":1},
                "itemCode":{"type":"string","pattern":"^[0-9]{1,10}$"},
                "itemName":{"type":"string","minLength":1,"maxLength":100}},
            "oneOf":[{"required":["itemCode"]},{"required":["itemName"]}]})
    } else {
        json!({"type":"object","additionalProperties":false,"required":["shopCode"],
            "properties":{"shopCode":{"type":"string","pattern":"^[A-Za-z0-9_-]{1,32}$"},
                "itemCode":{"type":"string","pattern":"^[0-9]{1,10}$"},
                "itemName":{"type":"string","minLength":1,"maxLength":100},
                "pageNum":{"type":"integer","minimum":1},
                "pageSize":{"type":"integer","minimum":1,"maximum":100}},
            "oneOf":[{"required":["itemCode"]},{"required":["itemName"]}]})
    };
    json!({"operationId":id,"operationVersion":version,"contractDigest":digest(&format!("{id}@{version}")),
        "effect":"read","summary":format!("{id} 摘要"),"risk":"read","contractSchemaVersion":1,
        "minimumCliVersion":"0.1.0","consented":consented,"inputSchema":input,
        "outputSchema":output_schema()})
}
pub(super) fn supply_operations() -> Vec<Value> {
    vec![
        operation(READ, "1.0.0", true),
        operation(BATCH, "1.0.0", true),
        operation(SHOPS, "1.0.0", false),
    ]
}
pub(super) fn key(operation: &Value) -> String {
    format!(
        "{}@{}#{}",
        operation["operationId"].as_str().unwrap(),
        operation["operationVersion"].as_str().unwrap(),
        operation["contractDigest"].as_str().unwrap()
    )
}
