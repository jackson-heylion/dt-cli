use super::support::Harness;
use serde_json::Value;
use std::{
    io::{BufRead, BufReader},
    process::{Child, Stdio},
    sync::mpsc::{Receiver, channel},
    time::Duration,
};

struct Subscription {
    child: Child,
    records: Receiver<Value>,
}

impl Subscription {
    fn record(&self) -> Value {
        self.records
            .recv_timeout(Duration::from_secs(25))
            .expect("SSE 未在期限内输出记录")
    }

    fn exit(&mut self) -> i32 {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.code().unwrap();
            }
            assert!(std::time::Instant::now() < deadline, "SSE 进程未退出");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn subscribe(harness: &Harness, mode: &str, cursor: Option<&str>) -> Subscription {
    let case = harness.case("fixture-stream");
    let mut command = harness.command(&[
        "call",
        "--stream",
        "--profile",
        "fixture",
        "--operation",
        &case.operation_id,
        "--version",
        &case.version.to_string(),
    ]);
    if let Some(cursor) = cursor {
        let file = harness.root.join("stream-input.json");
        std::fs::write(
            &file,
            serde_json::json!({
                "path": [{"name":"mode","value":mode}],
                "headers": [{"name":"Last-Event-ID","value":cursor}]
            })
            .to_string(),
        )
        .unwrap();
        command.arg("--input-file").arg(file);
    } else {
        command.args(["--path", &format!("/execution/stream/{mode}")]);
    }
    let mut child = command.stdout(Stdio::piped()).spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (send, records) = channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            assert!(!line.contains("dtp_cli_at_") && !line.contains("dtp_cli_rt_"));
            if send.send(serde_json::from_str(&line).unwrap()).is_err() {
                break;
            }
        }
    });
    Subscription { child, records }
}

pub(super) async fn run(harness: &Harness) {
    let mut live = subscribe(harness, "gated", None);
    let first = live.record();
    assert_eq!(first["data"]["id"], "run:1", "{first}");
    assert_eq!(first["data"]["body"], "{\n\"type\":\"progress\"}");
    harness.stats("stream-release").await;
    assert_eq!(live.record()["data"]["event"], "done");
    assert_eq!(live.exit(), 0);

    let mut early = subscribe(harness, "early", None);
    assert_eq!(early.record()["data"]["id"], "run:1");
    let failure = early.record();
    assert_eq!(failure["error"]["code"], "STREAM_INTERRUPTED");
    assert_eq!(failure["meta"]["lastEventId"], "run:1");
    assert_ne!(early.exit(), 0);

    let mut resumed = subscribe(harness, "resume", Some("run:1"));
    assert_eq!(resumed.record()["data"]["id"], "run:2");
    assert_eq!(resumed.exit(), 0);

    for (mode, status, code) in [
        ("expired", 410, "EVENT_CURSOR_EXPIRED"),
        ("unavailable", 503, "EVENT_REPLAY_UNAVAILABLE"),
    ] {
        let mut stream = subscribe(harness, mode, None);
        let failure = stream.record();
        assert_eq!(
            failure["error"]["details"]["httpStatus"], status,
            "{failure}"
        );
        assert_eq!(failure["error"]["details"]["code"], code, "{failure}");
        assert_ne!(stream.exit(), 0);
    }

    let mut pending = subscribe(harness, "pending", None);
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    loop {
        let stats = harness.stats("status").await;
        if stats["observations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|call| call["path"] == "/execution/stream/pending")
        {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "上游未收到待建连请求");
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    let mut revoked = subscribe(harness, "revoke", None);
    assert_eq!(revoked.record()["data"]["id"], "run:1");
    harness.grant(harness.case("fixture-stream"), false).await;
    let failure = revoked.record();
    assert_eq!(failure["ok"], false);
    assert_eq!(failure["meta"]["lastEventId"], "run:1");
    assert_ne!(revoked.exit(), 0);
    let before_headers = pending.record();
    assert_eq!(
        before_headers["error"]["details"]["httpStatus"], 403,
        "{before_headers}"
    );
    assert_ne!(pending.exit(), 0);
    let denied = reqwest::Client::new()
        .get(format!(
            "{}/platform-gateway/fixture/fixture-stream/1/execution/stream/revoke",
            harness.gateway
        ))
        .bearer_auth(harness.access())
        .header("X-Platform-Response-Mode", "sse")
        .header("Accept", "text/event-stream")
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), 403);
    let error: Value = denied.json().await.unwrap();
    assert_eq!(error["code"], "INTERFACE_PERMISSION_REQUIRED");
}
