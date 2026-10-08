use super::*;

pub(super) fn hrmp() -> Iam {
    let mut like = operation(LIKE, "1.0.0", true);
    like["effect"] = json!("write");
    like["confirmation"] = json!({"required":true,"channel":"agent-cli","singleDispatch":true});
    like["risk"] = json!("write");
    let id = json!({"type":"string","pattern":"^[1-9][0-9]{0,17}$"});
    let text = |max: u64| json!({"type":"string","minLength":1,"maxLength":max});
    like["inputSchema"] = json!({"type":"object","additionalProperties":false,
        "required":["beLikedPersonId","likeTypeId","likeTypeName","likeBehaviorContent","specificDeeds"],
        "properties":{"beLikedPersonId":id,"likeTypeId":id,"likeTypeName":text(20),
            "likeBehaviorContent":text(100),"specificDeeds":text(500)}});
    like["outputSchema"] = json!({"type":"object"});
    Iam::start("hrmp", ENV, vec![like])
}
pub(super) fn approve(iam: &Iam, id: &str) {
    iam.with(|f| f.set_intent(id, "approved", "none"));
}
pub(super) fn intents(iam: &Iam, start: usize) -> Vec<String> {
    iam.paths_since(start)
        .into_iter()
        .filter(|p| p.contains("/intents") || p.ends_with("/invoke"))
        .collect()
}

// B3/B5 写入：api call 不执行写；意图先准备、本人批准后单次发送；状态查询不发送。
