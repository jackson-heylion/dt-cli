use super::support::Harness;
use serde_json::{Value, json};

fn write(h: &Harness, operation: &str, parameters: Value, status: u16) -> Value {
    let input = h.root.join(format!("{operation}.json"));
    std::fs::write(&input, parameters.to_string()).unwrap();
    let prepared = h.ok(&[
        "writes",
        "prepare",
        "--profile",
        "marketing",
        "--operation",
        operation,
        "--version",
        "1",
        "--input-file",
        input.to_str().unwrap(),
        "--instruction",
        "隔离实例的合成问答验收",
    ]);
    let id = prepared["recordId"].as_str().unwrap();
    h.ok(&[
        "writes",
        "confirm",
        "--profile",
        "marketing",
        "--id",
        id,
        "--fingerprint",
        prepared["intent"]["fingerprint"].as_str().unwrap(),
    ]);
    let result = h.ok(&["writes", "dispatch", "--profile", "marketing", "--id", id]);
    assert_eq!(result["intent"]["state"], "SUCCEEDED", "{result}");
    assert_eq!(result["result"]["status"], status, "{result}");
    serde_json::from_str(result["result"]["body"].as_str().unwrap()).unwrap()
}

fn read(h: &Harness, operation: &str, path: &str) -> Value {
    let result = h.ok(&[
        "call",
        "--profile",
        "marketing",
        "--operation",
        operation,
        "--version",
        "1",
        "--path",
        path,
    ]);
    serde_json::from_str(result["body"].as_str().unwrap()).unwrap()
}

pub(super) async fn run(h: &Harness) {
    h.login_system("marketing", 'C', "1024", "marketing-agent-service")
        .await;
    let conversation = write(
        h,
        "marketing-create",
        json!({
            "bodyJson": json!({"title":"隔离 SSE 验收"}).to_string()
        }),
        201,
    );
    let conversation = conversation["id"].as_str().unwrap();
    let accepted = write(
        h,
        "marketing-send",
        json!({
            "path": [{"name":"id","value":conversation}],
            "headers": [{"name":"Idempotency-Key","value":"cli-sse-local-question"}],
            "bodyJson": json!({"content":"解释会员数据能力"}).to_string()
        }),
        202,
    );
    let id = accepted["executionId"].as_str().unwrap();
    let events_path = format!("/api/v1/agent-executions/{id}/events");
    let output = h
        .command(&[
            "call",
            "--stream",
            "--profile",
            "marketing",
            "--operation",
            "marketing-events",
            "--version",
            "1",
            "--path",
            &events_path,
        ])
        .output()
        .unwrap();
    let records: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(output.status.success(), "{records:?}");
    let mut last_sequence = None;
    for record in &records {
        let event: Value = serde_json::from_str(record["data"]["body"].as_str().unwrap()).unwrap();
        assert_eq!(event["executionId"], id);
        assert_eq!(event["type"], record["data"]["event"]);
        if let Some(sequence) = event["sequence"].as_u64() {
            assert_eq!(record["data"]["id"], format!("{id}:{sequence}"));
            assert_eq!(event["eventId"], record["data"]["id"]);
            assert!(last_sequence.is_none_or(|previous| sequence == previous + 1));
            last_sequence = Some(sequence);
        }
    }
    let terminal = records.last().unwrap();
    assert_eq!(terminal["data"]["event"], "message.completed", "{terminal}");
    let final_event: Value =
        serde_json::from_str(terminal["data"]["body"].as_str().unwrap()).unwrap();
    assert_eq!(final_event["executionId"], id);
    assert_eq!(final_event["terminal"], true);
    assert!(
        final_event["payload"]["content"]
            .as_str()
            .unwrap()
            .contains("隔离 SSE 测试模型")
    );
    assert!(
        records
            .iter()
            .any(|record| record["data"]["event"] == "progress.updated")
    );

    let state = read(
        h,
        "marketing-status",
        &format!("/api/v1/agent-executions/{id}"),
    );
    assert_eq!(state["status"], "SUCCEEDED", "{state}");
    let history = read(
        h,
        "marketing-messages",
        &format!("/api/v1/conversations/{conversation}/messages"),
    );
    let messages = history["items"].as_array().unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|message| message["role"] == "USER")
            .count(),
        1
    );
    let answer = messages
        .iter()
        .find(|message| message["role"] == "ASSISTANT" && message["agentExecutionId"] == id)
        .unwrap();
    assert!(
        answer["content"]
            .as_str()
            .unwrap()
            .contains("隔离 SSE 测试模型")
    );
}
