//! 原生写入命令的隔离协议夹具，副作用计数只来自 Gateway 接收事实。
use super::{run, setup};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
async fn request(socket: &mut tokio::net::TcpStream) -> (String, Value) {
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 8192];
        let size = socket.read(&mut buffer).await.unwrap();
        assert!(size > 0 && bytes.len() < 65536);
        bytes.extend_from_slice(&buffer[..size]);
        if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
            let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let length = head
                .lines()
                .filter_map(|v| v.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("Content-Length"))
                .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            if bytes.len() < end + 4 + length {
                continue;
            }
            return (
                head,
                serde_json::from_slice(&bytes[end + 4..]).unwrap_or(Value::Null),
            );
        }
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_write_never_redispatches_even_after_stale_authorized_readback() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    setup(root.path(), &origin, "CLI", "employee", 'S');
    let stored: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("platform/employee.json")).unwrap())
            .unwrap();
    let me = json!({"sessionId":"S".repeat(43),"subjectId":"1024","domain":"EMPLOYEE","displayName":"夹具员工","current":true,"purpose":"CLI","environment":"fixture","systemId":"platform","expiresAt":stored["expiresAt"],"revoked":false});
    // 固定夹具的契约字段序列用于独立协议摘要；响应可采用任意对象字段顺序。
    let original = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/platform-records/running.json"),
    )
    .unwrap();
    let start = original.find("\"contract\":").unwrap() + 11;
    let end = original[start..].find(",\"digest\":").unwrap() + start;
    let contract_raw = original[start..end]
        .replace("\"environment\":\"prod\"", "\"environment\":\"fixture\"")
        .replace("\"method\":\"GET\"", "\"method\":\"POST\"")
        .replace("\"effect\":\"READ\"", "\"effect\":\"WRITE\"");
    let contract: Value =
        serde_json::from_str(&contract_raw).unwrap_or_else(|e| panic!("{e}: {contract_raw}"));
    let digest = sha(contract_raw.as_bytes());
    let published = json!({"contract":contract,"digest":digest});
    let fingerprint=sha(format!("{{\"subject\":\"1024\",\"domain\":\"EMPLOYEE\",\"system\":\"orders\",\"environment\":\"fixture\",\"operation\":\"orders\",\"version\":1,\"digest\":\"{digest}\",\"request\":{{\"path\":\"/orders\",\"query\":\"\",\"headersDigest\":\"{}\",\"bodyDigest\":\"{}\",\"view\":null}}}}",sha(b"[]"),sha(b"")).as_bytes());
    let me = Arc::new(Mutex::new(me));
    let latest_identity = me.clone();
    let fixture_origin = origin.clone();
    let dispatches = Arc::new(AtomicUsize::new(0));
    let count = dispatches.clone();
    let intent = Arc::new(Mutex::new(Value::Null));
    let current = intent.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let published = published.clone();
            let me = me.clone();
            let origin = origin.clone();
            let fingerprint = fingerprint.clone();
            let intent = current.clone();
            let count = count.clone();
            tokio::spawn(async move {
                let (head, input) = request(&mut socket).await;
                let (status, body, headers) = if head.starts_with("POST /platform-gateway/") {
                    count.fetch_add(1, Ordering::SeqCst);
                    (
                        503,
                        json!({}),
                        "X-Platform-Error-Code: DEPENDENCY_UNAVAILABLE\r\nX-Platform-Error-Stage: UPSTREAM\r\nX-Platform-Trace-Id: trace-write-001\r\n",
                    )
                } else if head.starts_with("GET /platform/cli/catalog/runtime ") {
                    (200, json!({"gatewayBaseUrl":origin}), "")
                } else if head.starts_with("GET /platform/cli/catalog/orders/1 ") {
                    (200, published, "")
                } else if head.starts_with("GET /platform/cli/me ") {
                    (200, me.lock().unwrap().clone(), "")
                } else if head.starts_with("POST /platform/cli/writes ") {
                    let value = json!({"id":"I".repeat(43),"clientRequestId":input["clientRequestId"],"instruction":input["instruction"],"subjectId":"1024","sessionId":"S".repeat(43),"systemId":"orders","environment":"fixture","operationId":"orders","operationVersion":1,"contractDigest":published["digest"],"fingerprint":fingerprint,"view":null,"state":"PREPARED","expiresAt":chrono::Utc::now().timestamp()+600,"httpStatus":null,"responseDigest":null});
                    *intent.lock().unwrap() = value.clone();
                    (200, value, "")
                } else if head.starts_with("POST /platform/cli/writes/") {
                    let mut value = intent.lock().unwrap();
                    value["state"] = json!("AUTHORIZED");
                    (200, value.clone(), "")
                } else if head.starts_with("GET /platform/cli/writes/") {
                    (200, intent.lock().unwrap().clone(), "")
                } else {
                    panic!("未知夹具请求：{}", head.lines().next().unwrap());
                };
                let body = body.to_string();
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",body.len()).as_bytes()).await.unwrap();
            });
        }
    });
    let input = root.path().join("input.json");
    std::fs::write(
        &input,
        r#"{"path":[],"query":[],"headers":[],"form":[],"bodyJson":null,"view":null}"#,
    )
    .unwrap();
    let (prepared, code) = run(
        root.path(),
        &[
            "writes",
            "prepare",
            "--profile",
            "employee",
            "--operation",
            "orders",
            "--version",
            "1",
            "--input-file",
            input.to_str().unwrap(),
            "--instruction",
            "保存本次明确对象",
        ],
    );
    assert_eq!(code, 0, "{prepared}");
    let id = prepared["data"]["recordId"].as_str().unwrap();
    assert_eq!(dispatches.load(Ordering::SeqCst), 0);
    let (review, code) = run(root.path(), &["writes", "inspect", "--id", id]);
    assert_eq!(code, 0, "{review}");
    let fingerprint = review["data"]["fingerprint"].as_str().unwrap();
    let (confirmed, code) = run(
        root.path(),
        &[
            "writes",
            "confirm",
            "--profile",
            "employee",
            "--id",
            id,
            "--fingerprint",
            fingerprint,
        ],
    );
    assert_eq!(code, 0, "{confirmed}");
    let (unknown, code) = run(
        root.path(),
        &["writes", "dispatch", "--profile", "employee", "--id", id],
    );
    assert_eq!(code, 5, "{unknown}");
    assert_eq!(unknown["error"]["retryable"], false);
    assert_eq!(unknown["error"]["category"], "dependency");
    assert_eq!(unknown["error"]["details"]["httpStatus"], 503);
    assert_eq!(unknown["error"]["details"]["traceId"], "trace-write-001");
    for _ in 0..3 {
        let (observed, code) = run(
            root.path(),
            &["writes", "dispatch", "--profile", "employee", "--id", id],
        );
        assert_eq!(code, 0, "{observed}");
        assert_eq!(observed["data"]["stage"], "result_unknown");
    }
    assert_eq!(dispatches.load(Ordering::SeqCst), 1);
    setup(root.path(), &fixture_origin, "CLI", "employee", 'N');
    let renewed: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("platform/employee.json")).unwrap())
            .unwrap();
    latest_identity.lock().unwrap()["sessionId"] = json!("N".repeat(43));
    latest_identity.lock().unwrap()["expiresAt"] = renewed["expiresAt"].clone();
    let (reconfirmed, code) = run(
        root.path(),
        &[
            "writes",
            "confirm",
            "--profile",
            "employee",
            "--id",
            id,
            "--fingerprint",
            fingerprint,
        ],
    );
    assert_eq!(code, 4, "{reconfirmed}");
    assert_eq!(reconfirmed["error"]["code"], "WRITE_SESSION_CHANGED");
    let (original, code) = run(
        root.path(),
        &["writes", "status", "--profile", "employee", "--id", id],
    );
    assert_eq!(code, 0, "{original}");
    assert_eq!(original["data"]["stage"], "result_unknown");
    assert_eq!(dispatches.load(Ordering::SeqCst), 1);
    server.abort();
}
