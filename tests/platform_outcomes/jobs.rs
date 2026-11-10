//! 从原生 CLI 观察读取失败诊断及同一工作包检查点。
use super::{run, setup};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_failure_preserves_gateway_diagnostics_and_original_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    setup(root.path(), &origin, "CLI", "employee", 'S');
    let profile: Value =
        serde_json::from_slice(&std::fs::read(root.path().join("platform/employee.json")).unwrap())
            .unwrap();
    let me = json!({"sessionId":"S".repeat(43),"subjectId":"1024","domain":"EMPLOYEE","displayName":"夹具员工","purpose":"CLI","current":true,"revoked":false,"expiresAt":profile["expiresAt"]});
    let original = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/platform-records/running.json"),
    )
    .unwrap();
    let start = original.find("\"contract\":").unwrap() + 11;
    let end = original[start..].find(",\"digest\":").unwrap() + start;
    let contract =
        original[start..end].replace("\"environment\":\"prod\"", "\"environment\":\"fixture\"");
    let digest = format!("{:x}", Sha256::digest(contract.as_bytes()));
    let published =
        json!({"contract":serde_json::from_str::<Value>(&contract).unwrap(),"digest":digest});
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let origin = origin.clone();
            let me = me.clone();
            let published = published.clone();
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                while !bytes.windows(4).any(|v| v == b"\r\n\r\n") {
                    let mut buffer = [0; 8192];
                    let size = socket.read(&mut buffer).await.unwrap();
                    assert!(size > 0 && bytes.len() < 16384);
                    bytes.extend_from_slice(&buffer[..size]);
                }
                let head = String::from_utf8(bytes).unwrap();
                let (status, body, headers) = if head.starts_with("GET /platform-gateway/") {
                    (
                        503,
                        json!({}),
                        "X-Platform-Error-Code: DEPENDENCY_UNAVAILABLE\r\nX-Platform-Error-Stage: UPSTREAM\r\nX-Platform-Trace-Id: trace-read-001\r\n",
                    )
                } else if head.starts_with("GET /platform/cli/catalog/runtime ") {
                    (200, json!({"gatewayBaseUrl":origin}), "")
                } else if head.starts_with("GET /platform/cli/catalog/orders/1 ") {
                    (200, published, "")
                } else if head.starts_with("GET /platform/cli/me ") {
                    (200, me, "")
                } else {
                    panic!("未知夹具请求");
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
    let (failure, code) = run(
        root.path(),
        &[
            "jobs",
            "start",
            "--profile",
            "employee",
            "--operation",
            "orders",
            "--version",
            "1",
            "--input-file",
            input.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 5, "{failure}");
    assert_eq!(failure["error"]["code"], "DEPENDENCY_UNAVAILABLE");
    assert_eq!(failure["error"]["details"]["httpStatus"], 503);
    assert_eq!(failure["error"]["details"]["traceId"], "trace-read-001");
    assert_eq!(failure["data"]["state"], "paused");
    let id = failure["data"]["jobId"].as_str().unwrap();
    assert_eq!(failure["meta"]["recovery"]["recordId"], id);
    assert_eq!(
        failure["meta"]["recovery"]["actions"][0]["id"],
        "resume-original-job"
    );
    let (status, code) = run(root.path(), &["jobs", "status", "--id", id]);
    assert_eq!(code, 0, "{status}");
    assert_eq!(status["data"]["progress"]["pages"], 0);
    server.abort();
}
