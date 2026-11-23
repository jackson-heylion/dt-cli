//! 本地 Docker 的真实平台、Gateway 和 CLI 链路。由 .scratch 中的隔离场景启动。
#![cfg(feature = "local-dev")]
#[path = "common/platform_browser.rs"]
#[allow(dead_code)]
mod browser_fixture;
#[path = "platform_execution/support.rs"]
#[allow(dead_code)]
mod support;

use dt_cli::{Runtime, credentials::FileStore};
use reqwest::{Client, Response};
use serde_json::{Value, json};
use std::{path::Path, process::Stdio, time::Duration};
use support::Harness;

#[test]
#[ignore = "需要先启动本地 Docker 限流场景"]
fn platform_rate_limit_docker() {
    tokio::runtime::Runtime::new().unwrap().block_on(run());
    #[cfg(unix)]
    {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".scratch/platform-api-rate-limiting/docker/check-write-pacing.py");
        assert!(
            std::process::Command::new("python3")
                .arg(script)
                .arg(env!("CARGO_BIN_EXE_dt-cli"))
                .status()
                .unwrap()
                .success()
        );
    }
}

async fn run() {
    let dir = tempfile::tempdir().unwrap();
    let h = Harness::new(dir.path());
    let client = Client::builder()
        .timeout(Duration::from_secs(25))
        .build()
        .unwrap();
    h.login("fixture").await;
    let employee = h.access();
    let machine = std::env::var("DT_RATE_MACHINE_TOKEN").unwrap();
    let env = "/platform/registry/rate-settings/fixture";
    let read = "/platform-gateway/fixture/rate-read/1/business/read";
    let read_v2 = "/platform-gateway/fixture/rate-read/2/business/read";
    let other = "/platform-gateway/fixture/rate-other/1/business/other";
    let denied = "/platform-gateway/fixture/rate-denied/1/business/denied";
    let read_cfg = operation_settings(&h, "rate-read");
    let quota = |enabled, rate, burst, interval, allow| {
        json!({
            "enabled": enabled, "requestsPerSecond":rate, "burst":burst,
            "cliIntervalMillis":interval, "continueOnFailure":allow,
        })
    };
    let unlimited = json!({"mode":"UNLIMITED"});
    let normal = json!({"mode":"INHERIT"});
    let meta = operation_settings(&h, "rate-metadata");
    let pacing = json!({"mode":"UNLIMITED", "cliIntervalMillis":650});
    let tokens = quota(true, 1, 1, 1, false);
    let input = h.root.join("empty.json");
    std::fs::write(&input, "{}").unwrap();
    let input = input.to_str().unwrap();

    let initial = client
        .get(format!("{}{env}", h.platform))
        .header("Cookie", &h.admin_cookie)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(
        initial,
        json!({"enabled":false,"requestsPerSecond":null,"burst":null,"cliIntervalMillis":100,"continueOnFailure":false})
    );
    let detail = client
        .get(format!("{}/platform/cli/catalog/rate-read/1", h.platform))
        .bearer_auth(&employee)
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(detail["cliIntervalMillis"], 100);
    execution_limits(&h, &client, &employee, input).await;
    assert_eq!(
        configure(&h, &client, env, &quota(true, 0, 1, 200, false))
            .await
            .status(),
        400
    );
    assert_eq!(configure(&h, &client, env, &tokens).await.status(), 200);
    let warmup = quota(true, 100, 100, 1, false);
    assert_eq!(configure(&h, &client, env, &warmup).await.status(), 200);
    warm_gateway(&client, 19647, other, &employee).await;
    warm_gateway(&client, 19648, read_v2, &machine).await;
    assert_eq!(configure(&h, &client, env, &tokens).await.status(), 200);

    let before = calls(&h, &client, "/business/read").await.len();
    let (first, second) = tokio::join!(
        gateway(&client, 19647, read, &employee),
        gateway(&client, 19648, read_v2, &machine)
    );
    let statuses = [first.status().as_u16(), second.status().as_u16()];
    assert!(
        statuses.contains(&200) && statuses.contains(&429),
        "{statuses:?}"
    );
    let (limited, allowed) = if first.status() == 429 {
        (first, second)
    } else {
        (second, first)
    };
    assert_eq!(
        limited.headers()["X-Platform-Rate-Limit-Source"],
        "platform"
    );
    assert_eq!(limited.headers()["Retry-After"], "1");
    assert!(limited.headers().contains_key("X-Trace-Id"));
    assert!(
        allowed.headers()["Server-Timing"]
            .to_str()
            .unwrap()
            .contains("platform_rate_limit;dur=")
    );
    assert_eq!(calls(&h, &client, "/business/read").await.len(), before + 1);
    assert_eq!(
        gateway(&client, 19648, other, &employee).await.status(),
        200
    );
    assert_eq!(
        gateway(&client, 19648, denied, &employee).await.status(),
        403
    );
    assert_eq!(
        configure(
            &h,
            &client,
            &read_cfg,
            &json!({"mode":"CUSTOM","requestsPerSecond":1,"burst":1})
        )
        .await
        .status(),
        200
    );
    for token in [&employee, &machine] {
        let response = client
            .put(format!("{}{env}", h.platform))
            .header("Origin", &h.platform)
            .bearer_auth(token)
            .json(&warmup)
            .send()
            .await
            .unwrap();
        assert!(matches!(response.status().as_u16(), 401 | 403));
    }
    assert_eq!(
        configure(
            &h,
            &client,
            &read_cfg,
            &json!({"mode":"CUSTOM","requestsPerSecond":0,"burst":1})
        )
        .await
        .status(),
        400
    );
    assert_eq!(
        configure(&h, &client, &read_cfg, &pacing).await.status(),
        200
    );
    assert_eq!(
        configure(&h, &client, &meta, &unlimited).await.status(),
        200
    );
    let runtime = Runtime {
        root: h.root.clone(),
        store: Box::new(FileStore::new(&h.root)),
        browser: Box::new(browser_fixture::PlatformBrowser),
        interactive: false,
        aggregate_budget: Duration::from_secs(30),
    };
    let meta_args = [
        "call",
        "--profile",
        "fixture",
        "--operation",
        "rate-metadata",
        "--version",
        "1",
        "--input-file",
        input,
    ];
    let (cached_ok, status) = library_call(&runtime, &meta_args).await;
    assert_eq!(status, 0, "{cached_ok}");
    interval_fault(&h, &client, "missing").await;
    let (cached_ok, status) = library_call(&runtime, &meta_args).await;
    assert_eq!(status, 0, "{cached_ok}");
    let legacy = h.ok(&meta_args);
    assert_eq!(legacy["businessSuccess"], true);
    interval_fault(&h, &client, "invalid").await;
    let (rejected, status) = h.run(&meta_args);
    assert_ne!(status, 0, "{rejected}");
    assert_eq!(
        rejected["error"]["code"], "CLI_INTERVAL_UNAVAILABLE",
        "{rejected}"
    );
    interval_fault(&h, &client, "unavailable").await;
    assert_eq!(
        configure(
            &h,
            &client,
            &meta,
            &json!({"mode":"UNLIMITED", "cliIntervalMillis":350})
        )
        .await
        .status(),
        200
    );
    let before = calls(&h, &client, "/business/read").await.len();
    let args = [
        "call",
        "--profile",
        "fixture",
        "--operation",
        "rate-read",
        "--version",
        "1",
        "--input-file",
        input,
    ];
    let (a, b) = std::thread::scope(|scope| {
        let first = scope.spawn(|| h.ok(&args));
        let second = scope.spawn(|| h.ok(&args));
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_eq!(a["businessSuccess"], true);
    assert_eq!(b["businessSuccess"], true);
    let reads = calls(&h, &client, "/business/read").await;
    let interval = millis(&reads[before + 1]) - millis(&reads[before]);
    assert!(
        interval >= 590,
        "configured 650 ms, upstream observed {interval} ms"
    );
    println!("员工与外部应用通过两个 Gateway 跨版本共用额度，CLI 两进程上游观测间隔 {interval} ms");

    let (paged, code) = h.run(&[
        "jobs",
        "start",
        "--profile",
        "fixture",
        "--operation",
        "rate-pages",
        "--version",
        "1",
        "--input-file",
        input,
    ]);
    assert_ne!(code, 0, "{paged}");
    assert_limit(&paged, "platform", 1);
    let job = paged["data"]["jobId"].as_str().unwrap();
    let _ = h.ok(&["jobs", "results", "--id", job, "--page", "1"]);
    let pages = calls(&h, &client, "/business/pages").await;
    assert_eq!(
        pages
            .iter()
            .map(|p| p["page"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1]
    );
    assert_eq!(
        configure(
            &h,
            &client,
            &operation_settings(&h, "rate-pages"),
            &unlimited
        )
        .await
        .status(),
        200
    );
    let _ = h.ok(&["jobs", "resume", "--profile", "fixture", "--id", job]);
    let pages = calls(&h, &client, "/business/pages").await;
    assert_eq!(
        pages
            .iter()
            .map(|p| p["page"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    println!("分页先保存第 1 页，热改为不限流后只发送第 2 页");

    let left = authorize(&h, "rate-write", "left");
    let right = authorize(&h, "rate-write", "right");
    let ((one, one_code), (two, two_code)) = std::thread::scope(|scope| {
        let first = scope.spawn(|| h.run(&dispatch(&left)));
        let second = scope.spawn(|| h.run(&dispatch(&right)));
        (first.join().unwrap(), second.join().unwrap())
    });
    assert!(
        matches!((one_code, two_code), (0, 5) | (5, 0)),
        "{one} {two}"
    );
    let (blocked, id, nonce) = if one_code == 5 {
        (one, left, "left")
    } else {
        (two, right, "right")
    };
    assert_limit(&blocked, "platform", 1);
    assert_pending_write(&blocked);
    assert_eq!(write_count(&h, &client, nonce).await, 0);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let (first, second) = std::thread::scope(|scope| {
        let a = scope.spawn(|| h.ok(&dispatch(&id)));
        let b = scope.spawn(|| h.ok(&dispatch(&id)));
        (a.join().unwrap(), b.join().unwrap())
    });
    assert_eq!(first["intent"]["state"], "SUCCEEDED");
    assert_eq!(second["intent"]["state"], "SUCCEEDED");
    assert_eq!(write_count(&h, &client, nonce).await, 1);
    println!("被挡原单到达上游 0 次，两个进程并发手动继续后只写入 1 次");

    let busy = authorize(&h, "rate-busy-write", "busy");
    let (blocked, exit) = h.run(&dispatch(&busy));
    assert_ne!(exit, 0, "{blocked}");
    assert_limit(&blocked, "upstream", 7);
    assert_eq!(blocked["data"]["stage"], "result_unknown");
    assert!(
        blocked["data"]["result"]["body"]
            .as_str()
            .unwrap()
            .contains("UPSTREAM_BUSY")
    );
    assert_eq!(
        blocked["meta"]["recovery"]["actions"][0]["argv"][2],
        "status"
    );
    let _ = h.ok(&dispatch(&busy));
    assert_eq!(calls(&h, &client, "/business/upstream-429").await.len(), 1);
    println!("上游伪造未派发标记无效，429 原单只能查状态");

    let outage = authorize(&h, "rate-write", "outage");
    redis("stop");
    assert_eq!(
        configure(&h, &client, &read_cfg, &normal).await.status(),
        200
    );
    let (failed, exit) = h.run(&dispatch(&outage));
    assert_ne!(exit, 0);
    assert_eq!(
        failed["error"]["code"], "RATE_LIMIT_UNAVAILABLE",
        "{failed}"
    );
    assert_pending_write(&failed);
    assert_eq!(write_count(&h, &client, "outage").await, 0);
    assert_eq!(gateway(&client, 19648, read, &employee).await.status(), 503);
    let direct = client
        .get(format!("{}/business/read", h.control))
        .send()
        .await
        .unwrap();
    assert_eq!(direct.status(), 200);
    assert_eq!(
        configure(&h, &client, &read_cfg, &unlimited).await.status(),
        200
    );
    assert_eq!(gateway(&client, 19648, read, &employee).await.status(), 200);
    assert_eq!(
        configure(&h, &client, env, &quota(true, 1, 1, 1, true))
            .await
            .status(),
        200
    );
    assert_eq!(
        gateway(&client, 19648, other, &employee).await.status(),
        200
    );
    assert_eq!(
        gateway(&client, 19648, denied, &employee).await.status(),
        403
    );
    let completed = h.ok(&dispatch(&outage));
    assert_eq!(completed["intent"]["state"], "SUCCEEDED");
    assert_eq!(write_count(&h, &client, "outage").await, 1);
    let legacy = client
        .get("http://127.0.0.1:19647/legacy-probe")
        .send()
        .await
        .unwrap();
    assert_eq!(legacy.status(), 200);
    assert!(
        !legacy
            .headers()
            .contains_key("X-Platform-Rate-Limit-Source")
    );
    redis("start");
    println!("Redis 停止后热改故障策略成功，不限流、旧路由和正常权限检查照常");
    let events = client
        .get(format!(
            "{}/platform/audit?eventTypes=GOVERNED_RATE_SETTINGS_CHANGED",
            h.platform
        ))
        .header("Cookie", &h.admin_cookie)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let event = &events["items"][0];
    assert_eq!(event["actorId"], "EMPLOYEE:1024");
    assert!(event["recordedAt"].is_string());

    let before = calls(&h, &client, "/business/metadata").await.len();
    let (rejected, status) = h.run(&meta_args);
    assert_ne!(status, 0, "{rejected}");
    assert_eq!(calls(&h, &client, "/business/metadata").await.len(), before);
    interval_fault(&h, &client, "off").await;
    let (resumed, status) = h.run(&meta_args);
    assert_eq!(status, 0, "{resumed}");
    let (resumed, status) = h.run(&meta_args);
    assert_eq!(status, 0, "{resumed}");
    let sent = calls(&h, &client, "/business/metadata").await;
    assert_eq!(sent.len(), before + 2);
    let interval = millis(&sent[before + 1]) - millis(&sent[before]);
    assert!(
        interval >= 300,
        "configured 350 ms, upstream observed {interval} ms"
    );
    println!("旧平台缺少间隔时使用默认值；有效设置读取失败时暂停，恢复后使用新间隔 {interval} ms");
}

async fn library_call(rt: &Runtime, args: &[&str]) -> (Value, u8) {
    let argv = std::iter::once("dt-cli")
        .chain(args.iter().copied())
        .map(str::to_owned)
        .collect();
    let (reply, status, _) = dt_cli::execute(rt, argv).await;
    (reply, status)
}

async fn interval_fault(h: &Harness, client: &Client, mode: &str) {
    let response = client
        .get(format!(
            "{}/__fixture/interval-fault?mode={mode}",
            h.control
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
}

fn dispatch(id: &str) -> [&str; 6] {
    ["writes", "dispatch", "--profile", "fixture", "--id", id]
}

fn authorize(h: &Harness, operation: &str, nonce: &str) -> String {
    let path = h.root.join(format!("{nonce}.json"));
    std::fs::write(
        &path,
        json!({"bodyJson": json!({"nonce":nonce}).to_string()}).to_string(),
    )
    .unwrap();
    authorize_input(h, operation, &path)
}

fn authorize_input(h: &Harness, operation: &str, path: &Path) -> String {
    let row = h.ok(&[
        "writes",
        "prepare",
        "--profile",
        "fixture",
        "--operation",
        operation,
        "--version",
        "1",
        "--input-file",
        path.to_str().unwrap(),
        "--instruction",
        "隔离环境写入测试",
    ]);
    let id = row["recordId"].as_str().unwrap();
    let confirmed = h.ok(&[
        "writes",
        "confirm",
        "--profile",
        "fixture",
        "--id",
        id,
        "--fingerprint",
        row["intent"]["fingerprint"].as_str().unwrap(),
    ]);
    assert_eq!(confirmed["intent"]["state"], "AUTHORIZED");
    id.into()
}

async fn execution_limits(h: &Harness, client: &Client, employee: &str, empty_input: &str) {
    let url = format!(
        "{}/platform-gateway/fixture/rate-large/1/business/large",
        h.gateway
    );
    let response = client
        .post(&url)
        .bearer_auth(employee)
        .body("x".repeat(512 * 1024 + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);
    let query_value_bytes = 65_534 - url.len() - "?q=".len();
    let raw = json!({"blob": "x".repeat(512 * 1024 - r#"{"blob":""}"#.len())}).to_string();
    let oversized_uri = h.root.join("oversized-uri.json");
    std::fs::write(
        &oversized_uri,
        json!({
            "bodyJson": "{}",
            "query": [{"name":"q", "value": "x".repeat(query_value_bytes + 1)}]
        })
        .to_string(),
    )
    .unwrap();
    let oversized_id = authorize_input(h, "rate-large", &oversized_uri);
    let (rejected, status) = h.run(&dispatch(&oversized_id));
    assert_eq!(status, 2, "{rejected}");
    assert_eq!(rejected["error"]["code"], "INVALID_ARGUMENT");
    let input = h.root.join("large-input.json");
    std::fs::write(
        &input,
        json!({
            "bodyJson": raw,
            "query": [{"name":"q", "value": "x".repeat(query_value_bytes)}]
        })
        .to_string(),
    )
    .unwrap();
    let id = authorize_input(h, "rate-large", &input);
    let started = std::time::Instant::now();
    let result = h.ok(&dispatch(&id));
    assert!(started.elapsed() >= Duration::from_secs(41));
    assert_eq!(result["intent"]["state"], "SUCCEEDED");
    let body = result["result"]["body"].as_str().unwrap();
    assert_eq!(body.len(), 16 * 1024 * 1024);
    let returned: Value = serde_json::from_str(body).unwrap();
    assert_eq!(returned["requestBytes"], 512 * 1024);
    assert_eq!(returned["queryBytes"], query_value_bytes + "q=".len());
    let saved = h.ok(&["writes", "status", "--profile", "fixture", "--id", &id]);
    assert_eq!(
        saved["result"]["body"].as_str().map(str::len),
        Some(body.len())
    );

    let (rejected, status) = h.run(&[
        "call",
        "--profile",
        "fixture",
        "--operation",
        "rate-response-limit",
        "--version",
        "1",
        "--input-file",
        empty_input,
    ]);
    assert_ne!(status, 0);
    assert_eq!(rejected["error"]["code"], "RESPONSE_LIMIT", "{rejected}");
    println!(
        "实测 512 KiB 正文、65,534 字节完整 URI 和 16 MiB 响应，超过旧超时仍成功；三个大小上限各加一字节均被拒绝"
    );
}

fn assert_limit(reply: &Value, source: &str, seconds: u64) {
    assert_eq!(reply["error"]["code"], "RATE_LIMITED", "{reply}");
    assert_eq!(reply["error"]["details"]["rateLimitSource"], source);
    assert_eq!(reply["error"]["details"]["retryAfterSeconds"], seconds);
}

fn assert_pending_write(reply: &Value) {
    assert_eq!(reply["error"]["retryable"], false);
    assert_eq!(reply["data"]["stage"], "authorized", "{reply}");
    assert_eq!(reply["data"]["intent"]["state"], "AUTHORIZED");
    let action = &reply["meta"]["recovery"]["actions"][0];
    assert_eq!(action["requiresInteraction"], true);
    assert_eq!(action["argv"][2], "dispatch");
}

fn operation_settings(h: &Harness, operation: &str) -> String {
    format!(
        "/platform/registry/drafts/{}:fixture/rate-settings",
        h.case(operation).api_id
    )
}

async fn configure(h: &Harness, client: &Client, path: &str, value: &Value) -> Response {
    client
        .put(format!("{}{path}", h.platform))
        .header("Cookie", &h.admin_cookie)
        .header("Origin", &h.platform)
        .json(value)
        .send()
        .await
        .unwrap()
}

async fn gateway(client: &Client, port: u16, path: &str, token: &str) -> Response {
    client
        .get(format!("http://127.0.0.1:{port}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

async fn warm_gateway(client: &Client, port: u16, path: &str, token: &str) {
    // 两个 JVM 的第一笔核权会初始化各自的 Redis 连接与 HTTP 客户端。
    for attempt in 1..=3 {
        let response = gateway(client, port, path, token).await;
        let status = response.status().as_u16();
        if status == 200 {
            return;
        }
        let body = response.text().await.unwrap();
        assert!(
            status == 503 && attempt < 3,
            "warmup {port}: {status} {body}"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn calls(h: &Harness, client: &Client, path: &str) -> Vec<Value> {
    let all: Vec<Value> = client
        .get(format!("{}/__fixture/calls", h.control))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    all.into_iter()
        .filter(|call| call["path"] == path)
        .collect()
}

fn millis(call: &Value) -> u64 {
    call["atMillis"].as_u64().unwrap()
}

async fn write_count(h: &Harness, client: &Client, nonce: &str) -> usize {
    calls(h, client, "/business/write")
        .await
        .iter()
        .filter(|call| {
            let body: Value = serde_json::from_str(call["body"].as_str().unwrap()).unwrap();
            body["nonce"] == nonce
        })
        .count()
}

fn redis(action: &str) {
    let compose = std::env::var_os("DT_RATE_COMPOSE").unwrap();
    let mut command = std::process::Command::new("docker");
    command
        .args(["compose", "-f"])
        .arg(Path::new(&compose))
        .arg(action);
    if action == "stop" {
        command.args(["-t", "0"]);
    }
    assert!(
        command
            .arg("redis")
            .stdout(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
}
