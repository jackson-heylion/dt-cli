use super::support::{BODY, Harness};
use serde_json::Value;

fn id(value: &Value) -> String {
    value["recordId"].as_str().unwrap().to_owned()
}
fn fingerprint(value: &Value) -> String {
    value["intent"]["fingerprint"].as_str().unwrap().to_owned()
}
fn prepare(h: &Harness, operation: &str, input: &str) -> Value {
    h.ok(&[
        "writes",
        "prepare",
        "--profile",
        "fixture",
        "--operation",
        operation,
        "--version",
        "1",
        "--input-file",
        input,
        "--instruction",
        "在隔离HTTP夹具中明确记录一笔测试写入",
    ])
}
pub async fn run(h: &Harness) {
    let input = h.input("write-input.json", "JSON");
    let input = input.to_str().unwrap();
    let prepared = prepare(h, "fixture-write-order", input);
    let local = id(&prepared);
    let digest = fingerprint(&prepared);
    assert_eq!(prepared["intent"]["state"], "PREPARED");
    assert_eq!(h.stats("status").await["writes"], 0);
    let _ = h.run(&["writes", "dispatch", "--profile", "fixture", "--id", &local]);
    assert_eq!(h.stats("status").await["writes"], 0);
    let (_, status) = h.run(&[
        "writes",
        "confirm",
        "--profile",
        "fixture",
        "--id",
        &local,
        "--fingerprint",
        &"a".repeat(64),
    ]);
    assert_ne!(status, 0);
    assert_eq!(h.stats("status").await["writes"], 0);
    let review = h.ok(&["writes", "inspect", "--id", &local]);
    assert_eq!(review["request"]["body"], BODY);
    assert_eq!(review["fingerprint"], digest);
    let record = h.root.join(format!("platform/writes/{local}.json"));
    let original = std::fs::read(&record).unwrap();
    let mut changed: Value = serde_json::from_slice(&original).unwrap();
    changed["record"]["request"]["body"] = Value::String("{\"note\":null,\"amount\":999}".into());
    std::fs::write(&record, serde_json::to_vec(&changed).unwrap()).unwrap();
    let (_, status) = h.run(&[
        "writes",
        "confirm",
        "--profile",
        "fixture",
        "--id",
        &local,
        "--fingerprint",
        &digest,
    ]);
    assert_ne!(status, 0);
    std::fs::write(&record, original).unwrap();
    let confirmed = h.ok(&[
        "writes",
        "confirm",
        "--profile",
        "fixture",
        "--id",
        &local,
        "--fingerprint",
        &digest,
    ]);
    assert_eq!(confirmed["intent"]["state"], "AUTHORIZED");
    let remote = confirmed["intent"]["id"].as_str().unwrap();
    let credential = h.access();
    let url = format!(
        "{}/platform-gateway/fixture/fixture-write-order/1/execution/write/42?tags=a+b&tags=%E4%B8%AD",
        h.gateway
    );
    let client = reqwest::Client::new();
    for (body, language) in [("{\"note\":null,\"amount\":999}", "zh-CN"), (BODY, "en-US")] {
        let denied = client
            .post(&url)
            .bearer_auth(&credential)
            .header("X-Platform-Intent-Id", remote)
            .header("Language", language)
            .header("Content-Type", "application/json")
            .body(body.to_owned())
            .send()
            .await
            .unwrap();
        assert_eq!(denied.status(), 403);
        assert_eq!(h.stats("status").await["writes"], 0);
    }
    // 修改原输入文件不会覆盖已准备的原请求；派发必须使用签名记录。
    std::fs::write(input, "{}").unwrap();
    let completed = h.ok(&["writes", "dispatch", "--profile", "fixture", "--id", &local]);
    assert_eq!(completed["intent"]["state"], "SUCCEEDED", "{completed}");
    assert_eq!(completed["result"]["businessSuccess"], true);
    assert_eq!(h.stats("status").await["writes"], 1);
    let again = h.ok(&["writes", "dispatch", "--profile", "fixture", "--id", &local]);
    assert_eq!(again["intent"]["state"], "SUCCEEDED");
    assert_eq!(h.stats("status").await["writes"], 1);
    let unknown_input = h.input("unknown-write-input.json", "JSON");
    let unknown_input = unknown_input.to_str().unwrap();
    let unknown = prepare(h, "fixture-unknown-write", unknown_input);
    let unknown_id = id(&unknown);
    let unknown_fingerprint = fingerprint(&unknown);
    h.ok(&[
        "writes",
        "confirm",
        "--profile",
        "fixture",
        "--id",
        &unknown_id,
        "--fingerprint",
        &unknown_fingerprint,
    ]);
    let (lost, code) = h.run(&[
        "writes",
        "dispatch",
        "--profile",
        "fixture",
        "--id",
        &unknown_id,
    ]);
    assert_ne!(code, 0);
    assert_eq!(lost["data"]["stage"], "result_unknown");
    let observed = h.ok(&[
        "writes",
        "status",
        "--profile",
        "fixture",
        "--id",
        &unknown_id,
    ]);
    assert_eq!(observed["intent"]["state"], "RESULT_UNKNOWN");
    assert_eq!(h.stats("status").await["unknownWrites"], 1);
    h.ok(&[
        "writes",
        "dispatch",
        "--profile",
        "fixture",
        "--id",
        &unknown_id,
    ]);
    assert_eq!(h.stats("status").await["unknownWrites"], 1);
    let (switched, code) = h.run(&[
        "writes",
        "status",
        "--profile",
        "switched",
        "--id",
        &unknown_id,
    ]);
    assert_ne!(code, 0);
    assert_eq!(switched["error"]["code"], "TASK_CONTEXT_CHANGED");
    assert_eq!(h.stats("status").await["unknownWrites"], 1);
    h.login("write-renewed").await;
    let renewed = h.ok(&[
        "writes",
        "status",
        "--profile",
        "write-renewed",
        "--id",
        &unknown_id,
    ]);
    assert_eq!(renewed["intent"]["state"], "RESULT_UNKNOWN");
    assert_eq!(
        renewed["intent"]["sessionId"],
        observed["intent"]["sessionId"]
    );
    let (_, denied) = h.run(&[
        "writes",
        "dispatch",
        "--profile",
        "write-renewed",
        "--id",
        &unknown_id,
    ]);
    assert_ne!(denied, 0);
    assert_eq!(h.stats("status").await["unknownWrites"], 1);
    let different_key = prepare(h, "fixture-unknown-write", unknown_input);
    assert_eq!(different_key["intent"]["id"], observed["intent"]["id"]);
    assert_eq!(different_key["stage"], "result_unknown");
    h.ok(&[
        "writes",
        "dispatch",
        "--profile",
        "fixture",
        "--id",
        &id(&different_key),
    ]);
    assert_eq!(h.stats("status").await["unknownWrites"], 1);
    let listing = h.ok(&["writes", "list"]);
    assert_eq!(listing["total"], 3);
}
