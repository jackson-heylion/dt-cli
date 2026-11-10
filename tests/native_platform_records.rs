#![cfg(feature = "local-dev")]
use dt_cli::credentials::{CredentialStore, FileStore};
use serde_json::Value;
use std::{path::Path, process::Command};
const ID: &str = "JJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJJ";
fn copy(root: &Path, file: &str, destination: &str) {
    let path = root.join(destination);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/platform-records")
            .join(file),
        &path,
    )
    .unwrap();
    privacy::protect(path.parent().unwrap());
    privacy::protect(&path);
}
fn command(root: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dt-cli"));
    cmd.env("DT_CLI_LOCAL_TEST_CONFIG", root);
    cmd
}
fn invoke(root: &Path, args: &[&str]) -> Value {
    let out = command(root).args(args).output().unwrap();
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        out.status.success(),
        "{value} {}",
        String::from_utf8_lossy(&out.stderr)
    );
    value["data"].clone()
}
fn setup(root: &Path) {
    FileStore::new(root)
        .write_integrity_key("metadata/workpack-signing-v1", &[42; 32])
        .unwrap();
}
#[test]
fn original_platform_signatures_pages_and_number_precision_remain_readable() {
    let root = tempfile::tempdir().unwrap();
    setup(root.path());
    copy(
        root.path(),
        "succeeded.json",
        &format!("platform/jobs/{ID}.json"),
    );
    copy(
        root.path(),
        "page.json",
        &format!("platform/jobs/{ID}/00000001.json"),
    );
    let status = invoke(root.path(), &["jobs", "status", "--id", ID]);
    assert_eq!(status["state"], "succeeded");
    assert_eq!(status["progress"]["pages"], 1);
    let result = invoke(root.path(), &["jobs", "results", "--id", ID]);
    assert_eq!(result["checkpointCommitted"], true);
    assert!(
        result["page"]["result"]["body"]
            .as_str()
            .unwrap()
            .contains("123.0000000000000000000000001")
    );
    assert!(
        result["page"]["result"]["body"]
            .as_str()
            .unwrap()
            .contains("null")
    );
}
#[cfg(unix)]
#[test]
fn status_rechecks_completion_after_acquiring_task_lock() {
    use fs2::FileExt;
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    setup(root.path());
    copy(
        root.path(),
        "running.json",
        &format!("platform/jobs/{ID}.json"),
    );
    std::fs::create_dir_all(root.path().join("workpacks")).unwrap();
    let gate = root.path().join("workpacks/.key.lock");
    let key_lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&gate)
        .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(
        root.path().join("workpacks"),
        std::fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    std::fs::set_permissions(&gate, std::fs::Permissions::from_mode(0o600)).unwrap();
    key_lock.lock_exclusive().unwrap();
    let child = command(root.path())
        .args(["jobs", "status", "--id", ID])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    // 观察原生进程已打开完整性锁，证明它已读取旧记录并在等待验证。
    loop {
        let state = Command::new("lsof")
            .args(["-p", &child.id().to_string(), "-Fn"])
            .output()
            .expect("原生竞态测试需要 lsof 观察已打开的锁");
        if String::from_utf8_lossy(&state.stdout).contains(gate.to_str().unwrap()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "状态进程未到达已读取旧快照的受控等待点"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    copy(
        root.path(),
        "succeeded.json",
        &format!("platform/jobs/{ID}.json"),
    );
    FileExt::unlock(&key_lock).unwrap();
    let out = child.wait_with_output().unwrap();
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["data"]["state"], "succeeded", "{value}");
    assert_eq!(
        invoke(root.path(), &["jobs", "status", "--id", ID])["progress"]["pages"],
        1
    );
}

#[path = "common/privacy.rs"]
mod privacy;
