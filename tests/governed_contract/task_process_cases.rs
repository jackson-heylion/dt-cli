//! Real subprocess signals/crashes, with controlled IAM and secure-store boundaries.
use super::*;
use std::{
    fs,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Instant,
};
const KEY_NAME: &str = "metadata/workpack-signing-v1";
fn iam() -> Iam {
    let iam = hrmp();
    iam.with(|f|f.operations[0]["outputSchema"]=json!({"type":"object","properties":{"intentId":{"type":"string"},"state":{"type":"string"},"sideEffect":{"type":"string"}}}));
    iam
}
fn spawn(h: &Harness, iam: &Iam) -> Child {
    // Test-only shared key crosses the process boundary; no real credentials are involved.
    let key = vec![0x71u8; 32];
    h.store.write_integrity_key(KEY_NAME, &key).unwrap();
    let profile: Value =
        serde_json::from_slice(&fs::read(h.file("hr.profile.json")).unwrap()).unwrap();
    let credentials = h.store.get(profile["credentialKey"].as_str().unwrap());
    fs::write(h.root.path().join("params.json"), LIKE_PARAMS).unwrap();
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "task_process_cases::task_process_child",
            "--ignored",
            "--nocapture",
        ])
        .env("DT_TEST_TASK_ROOT", h.root.path())
        .env("DT_TEST_TASK_ORIGIN", &iam.origin)
        .env(
            "DT_TEST_TASK_CREDENTIALS",
            serde_json::to_string(&credentials).unwrap(),
        )
        .env("DT_TEST_TASK_INTEGRITY", URL_SAFE_NO_PAD.encode(key))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}
fn record(h: &Harness) -> Value {
    let path = fs::read_dir(h.root.path().join("workpacks"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|e| e == "json"))
        .unwrap();
    serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap()["record"].clone()
}
async fn entered(iam: &Iam, phase: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let ready = iam.with(|f| match phase {
            "prepare" => !f.intents.is_empty(),
            "authorize" => f.intents.values().any(|i| i["state"] == "approved"),
            _ => f.writes == 1,
        });
        if ready {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("child did not enter {phase}");
}
async fn finished(child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if child.try_wait().unwrap().is_none() {
        child.kill().unwrap();
        panic!("child did not stop");
    }
}
fn parse(output: &std::process::Output) -> Value {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|l| serde_json::from_str::<Value>(l).ok())
        .unwrap()
}
fn resume_args<'a>(path: &'a str, id: &'a str) -> Vec<&'a str> {
    vec![
        "tasks",
        "run",
        "like.send",
        "--profile",
        "hr",
        "--version",
        "1.0.0",
        "--params-file",
        path,
        "--execute",
        "--run-id",
        id,
    ]
}

#[tokio::test]
async fn ctrl_c_during_dispatch_preserves_receipt_and_never_dispatches_again() {
    let iam = iam();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hr", ENV, "hrmp").await;
    iam.with(|f| f.dispatch = Some("hold"));
    let mut child = spawn(&h, &iam);
    entered(&iam, "dispatch").await;
    let original = record(&h);
    assert_eq!(original["stage"], "dispatch-attempted");
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    finished(&mut child).await;
    let output = child.wait_with_output().unwrap();
    let value = parse(&output);
    assert_eq!(output.status.code(), Some(6));
    assert_eq!(value["error"]["code"], "SIDE_EFFECT_UNKNOWN");
    assert_eq!(value["meta"]["recovery"]["runId"], original["runId"]);
    assert_eq!(value["meta"]["recovery"]["intentId"], original["remoteId"]);
    let id = original["runId"].as_str().unwrap();
    let path = h.root.path().join("params.json");
    let start = iam.count();
    let (v, e) = h.run(&resume_args(path.to_str().unwrap(), id)).await;
    assert_eq!(e, 0, "{v}");
    assert_eq!(v["meta"]["complete"], false);
    assert!(iam.since(start).iter().all(|r| r.method == "GET"));
    iam.with(|f| assert_eq!(f.writes, 1));
}

#[tokio::test]
async fn process_crashes_at_prepare_authorize_and_dispatch_recover_original_identity_only() {
    for phase in ["prepare", "authorize", "dispatch"] {
        let iam = iam();
        let h = Harness::new(&[(ENV, &iam)]);
        h.login("hr", ENV, "hrmp").await;
        iam.with(|f| match phase {
            "prepare" => f.hold_prepare_reply_once = true,
            "authorize" => f.hold_authorize_reply_once = true,
            _ => f.dispatch = Some("hold"),
        });
        let mut child = spawn(&h, &iam);
        entered(&iam, phase).await;
        child.kill().unwrap();
        finished(&mut child).await;
        let output = child.wait_with_output().unwrap();
        assert!(!output.status.success());
        let original = record(&h);
        let id = original["runId"].as_str().unwrap();
        assert_eq!(
            original["stage"],
            match phase {
                "prepare" => "prepare-attempted",
                "authorize" => "authorize-attempted",
                _ => "dispatch-attempted",
            }
        );
        let start = iam.count();
        let (status, exit) = h.run(&["tasks", "status", id]).await;
        if phase == "prepare" {
            assert_failure(&status, exit, 6, "RUN_UNLOCATED");
        } else {
            assert_eq!(exit, 0, "{status}");
        }
        let path = h.root.path().join("params.json");
        let (v, e) = h.run(&resume_args(path.to_str().unwrap(), id)).await;
        assert_eq!(e, 0, "{v}");
        assert_eq!(v["meta"]["complete"], false);
        assert_eq!(record(&h)["idempotencyKey"], original["idempotencyKey"]);
        assert!(
            !iam.paths_since(start)
                .iter()
                .any(|p| p.ends_with("/authorize") || p.ends_with("/invoke"))
        );
        iam.with(|f| {
            assert_eq!(f.intents.len(), 1);
            assert_eq!(f.intent_keys.len(), 1);
            assert_eq!(f.writes, u32::from(phase == "dispatch"));
        });
    }
}

#[tokio::test]
#[ignore = "spawned by task process tests with synthetic process-local credentials"]
async fn task_process_child() {
    let root = PathBuf::from(std::env::var_os("DT_TEST_TASK_ROOT").unwrap());
    let origin = std::env::var("DT_TEST_TASK_ORIGIN").unwrap();
    let profile: Value =
        serde_json::from_slice(&fs::read(root.join("governed/hr.profile.json")).unwrap()).unwrap();
    let credentials: Credentials =
        serde_json::from_str(&std::env::var("DT_TEST_TASK_CREDENTIALS").unwrap()).unwrap();
    let store = MemoryStore::default();
    store
        .write(profile["credentialKey"].as_str().unwrap(), &credentials)
        .unwrap();
    store
        .write_integrity_key(
            KEY_NAME,
            &URL_SAFE_NO_PAD
                .decode(std::env::var("DT_TEST_TASK_INTEGRITY").unwrap())
                .unwrap(),
        )
        .unwrap();
    let path = root.join("params.json");
    let rt = Runtime {
        root,
        environments: BTreeMap::from([(ENV.into(), environment(&origin))]),
        store: Box::new(store),
        browser: Box::new(NoBrowser),
        interactive: false,
        aggregate_budget: Duration::from_secs(30),
    };
    let (value, exit, _) = dt_cli::execute(
        &rt,
        [
            "dt-cli",
            "tasks",
            "run",
            "like.send",
            "--profile",
            "hr",
            "--version",
            "1.0.0",
            "--params-file",
            path.to_str().unwrap(),
            "--execute",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
    )
    .await;
    println!("{value}");
    std::process::exit(exit.into());
}
