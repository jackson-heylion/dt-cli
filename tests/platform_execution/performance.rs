//! 固定旧版与当前原生进程，实际平台/Gateway/上游的读写端到端基准。
use super::support::{BODY, Harness};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Instant,
};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Profile {
    /// 本地公开格式版本。
    schema_version: u8,
    /// 当前隔离平台。
    origin: String,
    /// 固定环境。
    environment: String,
    /// 旧版所需的固定系统，不改变服务端会话。
    system_id: String,
    /// 稳定本人标识。
    subject_id: String,
    /// 固定身份域。
    domain: String,
    /// 独立会话标识。
    session_id: String,
    /// 绝对期限。
    expires_at: String,
    /// 私有存储引用，不写报告。
    credential_key: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Measurement {
    /// 固定二进制版本标签。
    version: String,
    /// READ或WRITE。
    effect: String,
    /// 新进程冷元数据，warm仅指服务端预热。
    phase: String,
    /// 实际任务数。
    samples: usize,
    /// 并行原生进程数。
    concurrency: usize,
    /// 任务端到端P50毫秒，包含进程启动与写准备确认。
    p50_millis: f64,
    /// 任务端到端P95毫秒。
    p95_millis: f64,
    /// 每秒完整任务数。
    throughput: f64,
    /// 失败任务数。
    errors: usize,
    /// 业务响应累计字节，各READ任务含三页。
    response_bytes: u64,
}
fn command(binary: &str, root: &Path, args: &[&str]) -> Value {
    // 固定旧版的clap不接受以减号开头的分离值，使用等号形式保持同一合法标识。
    let mut arguments = Vec::new();
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--id" {
            arguments.push(format!("--id={}", args[index + 1]));
            index += 2;
        } else {
            arguments.push(args[index].to_owned());
            index += 1;
        }
    }
    let output = std::process::Command::new(binary)
        .arg("platform")
        .args(arguments)
        .env("DT_CLI_LOCAL_TEST_CONFIG", root)
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(output.status.success(), "原生基准失败: {value}");
    value["data"].clone()
}
fn task(binary: &str, root: &Path, write: bool, sequence: usize) -> u64 {
    let input = root.join(format!("input-{sequence}.json"));
    let body = if write {
        format!("{{\"note\":\"benchmark-{sequence}\",\"amount\":123.0000000000000000000000001}}")
    } else {
        BODY.into()
    };
    let raw = if write {
        format!(
            "{{\"path\":[{{\"name\":\"id\",\"value\":42}}],\"query\":[{{\"name\":\"tags\",\"value\":[\"a b\",\"中\"]}}],\"headers\":[{{\"name\":\"Language\",\"value\":\"zh-CN\"}}],\"bodyJson\":{}}}",
            serde_json::to_string(&body).unwrap()
        )
    } else {
        format!("{{\"bodyJson\":{}}}", serde_json::to_string(&body).unwrap())
    };
    std::fs::write(&input, raw).unwrap();
    if !write {
        let result = command(
            binary,
            root,
            &[
                "jobs",
                "start",
                "--profile",
                "fixture",
                "--operation",
                "fixture-before-pages",
                "--version",
                "1",
                "--input-file",
                input.to_str().unwrap(),
            ],
        );
        assert_eq!(result["complete"], true);
        assert_eq!(result["progress"]["records"], 5);
        return result["progress"]["bytes"].as_u64().unwrap();
    }
    let prepared = command(
        binary,
        root,
        &[
            "writes",
            "prepare",
            "--profile",
            "fixture",
            "--operation",
            "fixture-write-order",
            "--version",
            "1",
            "--input-file",
            input.to_str().unwrap(),
            "--instruction",
            "隔离基准独立写入",
        ],
    );
    let id = prepared["recordId"].as_str().unwrap();
    let fingerprint = prepared["intent"]["fingerprint"].as_str().unwrap();
    command(
        binary,
        root,
        &[
            "writes",
            "confirm",
            "--profile",
            "fixture",
            "--id",
            id,
            "--fingerprint",
            fingerprint,
        ],
    );
    let result = command(
        binary,
        root,
        &["writes", "dispatch", "--profile", "fixture", "--id", id],
    );
    assert_eq!(result["intent"]["state"], "SUCCEEDED");
    result["result"]["body"].as_str().unwrap().len() as u64
}
fn measure(
    binary: &str,
    root: &Path,
    version: &str,
    write: bool,
    phase: &str,
    n: usize,
    concurrency: usize,
    sequence: &mut usize,
) -> Measurement {
    let started = Instant::now();
    let mut elapsed = Vec::new();
    let mut bytes = 0;
    let mut errors = 0;
    for chunk in (0..n).collect::<Vec<_>>().chunks(concurrency) {
        std::thread::scope(|scope| {
            let mut workers = Vec::new();
            for _ in chunk {
                *sequence += 1;
                let current = *sequence;
                workers.push(scope.spawn(move || {
                    let start = Instant::now();
                    let result = std::panic::catch_unwind(|| task(binary, root, write, current));
                    (start.elapsed().as_secs_f64() * 1000., result)
                }));
            }
            for worker in workers {
                let (ms, result) = worker.join().unwrap();
                elapsed.push(ms);
                match result {
                    Ok(value) => bytes += value,
                    Err(_) => errors += 1,
                }
            }
        });
    }
    elapsed.sort_by(f64::total_cmp);
    Measurement {
        version: version.into(),
        effect: if write { "WRITE" } else { "READ" }.into(),
        phase: phase.into(),
        samples: n,
        concurrency,
        p50_millis: elapsed[(n - 1) / 2],
        p95_millis: elapsed[(n as f64 * 0.95).ceil() as usize - 1],
        throughput: n as f64 / started.elapsed().as_secs_f64(),
        errors,
        response_bytes: bytes,
    }
}
pub async fn run() {
    let folder = tempfile::tempdir().unwrap();
    let h = Harness::new(folder.path());
    h.login("fixture").await;
    let path = h.root.join("platform/fixture.json");
    let mut profile: Profile = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    profile.system_id = "supply-chain-server".into();
    std::fs::write(path, serde_json::to_vec(&profile).unwrap()).unwrap();
    let binaries = [
        ("before", std::env::var("DT_CLI_BASELINE_BINARY").unwrap()),
        ("after", env!("CARGO_BIN_EXE_dt-cli").into()),
    ];
    let mut rows = Vec::new();
    let mut sequence = 0;
    for (label, binary) in binaries {
        for write in [false, true] {
            rows.push(measure(
                &binary,
                &h.root,
                label,
                write,
                "cold-process",
                1,
                1,
                &mut sequence,
            ));
            for _ in 0..2 {
                sequence += 1;
                task(&binary, &h.root, write, sequence);
            }
            rows.push(measure(
                &binary,
                &h.root,
                label,
                write,
                "server-warm-process-cold",
                8,
                1,
                &mut sequence,
            ));
            rows.push(measure(
                &binary,
                &h.root,
                label,
                write,
                "server-warm-process-cold",
                16,
                4,
                &mut sequence,
            ));
        }
    }
    assert!(rows.iter().all(|r| r.errors == 0));
    std::fs::write(
        PathBuf::from(std::env::var("DT_CLI_PERF_REPORT").unwrap()),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}
