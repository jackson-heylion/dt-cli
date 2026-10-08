use super::*;
use std::time::Instant;

struct StoreFailsAfterStatus(MemoryStore, std::sync::atomic::AtomicUsize);
impl CredentialStore for StoreFailsAfterStatus {
    fn read(&self, key: &str) -> Result<Option<Credentials>> {
        if self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            self.0.read(key)
        } else {
            Err(dt_cli::output::storage())
        }
    }
    fn write(&self, key: &str, value: &Credentials) -> Result<()> {
        self.0.write(key, value)
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.delete(key)
    }
}

#[tokio::test]
async fn wait_stops_before_result_when_secure_store_fails_after_status() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let id = "S".repeat(43);
    iam.with(|f| {
        f.jobs.insert(id.clone(), "succeeded".into());
    });
    let mut rt = h.sealed();
    rt.store = Box::new(StoreFailsAfterStatus(h.store.clone(), Default::default()));
    let start = iam.count();
    let (v, e) = h
        .run_with(&rt, &["jobs", "wait", &id, "--profile", "supply"])
        .await;
    assert_failure(&v, e, 1, "CREDENTIAL_STORE_UNAVAILABLE");
    assert_eq!(v["data"]["jobId"], id);
    assert_eq!(v["data"]["state"], "succeeded");
    assert_eq!(v["meta"]["complete"], false);
    assert_eq!(v["meta"]["traceId"], "trace-ok");
    assert_eq!(v["meta"]["actions"][0]["actor"], "operator");
    assert_eq!(
        iam.paths_since(start),
        vec![format!("GET /cli-api/v1/jobs/{id}")]
    );
}

fn expire(h: &Harness) -> String {
    let key = h.store.governed_key();
    let mut c = h.store.get(&key);
    c.access_expires_at = Some("2000-01-01T00:00:00Z".into());
    h.store.write(&key, &c).unwrap();
    key
}
#[tokio::test]
async fn wait_deadline_covers_authorization_lock_and_never_refreshes_after_timeout() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let key = expire(&h);
    let p: Value =
        serde_json::from_slice(&std::fs::read(h.file("supply.profile.json")).unwrap()).unwrap();
    let lock =
        dt_cli::profile::authorization_lock(h.root.path(), p["authorizationId"].as_str().unwrap())
            .await
            .unwrap();
    let start = iam.count();
    let now = Instant::now();
    let id = "L".repeat(43);
    let (v, e) = h
        .run(&["jobs", "wait", &id, "--profile", "supply", "--timeout", "1"])
        .await;
    assert_failure(&v, e, 7, "JOB_WAIT_TIMEOUT");
    assert!(now.elapsed() < Duration::from_secs(2));
    assert_eq!(iam.count(), start);
    assert_eq!(h.store.get(&key).refresh_state, "ready");
    drop(lock);
}
#[tokio::test]
async fn wait_deadline_covers_inflight_refresh_and_keeps_pending_receipt() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let key = expire(&h);
    iam.with(|f| {
        f.overrides.insert("token", (1, vec![], vec![]));
    });
    let start = iam.count();
    let id = "R".repeat(43);
    let now = Instant::now();
    let (v, e) = h
        .run(&["jobs", "wait", &id, "--profile", "supply", "--timeout", "1"])
        .await;
    assert_failure(&v, e, 7, "JOB_WAIT_TIMEOUT");
    assert!(now.elapsed() < Duration::from_secs(2));
    let seen = iam.since(start);
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/cli-api/oauth2/token");
    let c = h.store.get(&key);
    assert_eq!(c.refresh_state, "pending");
    assert!(c.refresh_request_id.is_some());
    assert_eq!(c.generation, 0);
}
#[tokio::test]
async fn wait_download_uses_remaining_deadline_and_preserves_last_trace() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let id = "D".repeat(43);
    iam.with(|f| {
        f.jobs.insert(id.clone(), "succeeded".into());
        f.overrides.insert("jobs.result", (1, vec![], vec![]));
    });
    let start = iam.count();
    let now = Instant::now();
    let (v, e) = h
        .run(&["jobs", "wait", &id, "--profile", "supply", "--timeout", "1"])
        .await;
    assert_failure(&v, e, 7, "JOB_WAIT_TIMEOUT");
    assert!(now.elapsed() < Duration::from_secs(2));
    assert_eq!(v["data"]["state"], "succeeded");
    assert_eq!(v["meta"]["traceId"], "trace-ok");
    assert!(v["data"]["items"].is_null());
    assert_eq!(iam.since(start).len(), 2);
}
#[tokio::test]
async fn wait_download_permission_or_contract_change_returns_no_result_and_no_resubmit() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let id = "F".repeat(43);
    for (status, code, exit, actor) in [
        (403, "GRANT_EXPIRED", 4, "admin"),
        (409, "CONTRACT_CHANGED", 6, "employee"),
    ] {
        iam.with(|f| {
            f.jobs.insert(id.clone(), "succeeded".into());
            f.overrides.insert("jobs.result", failure(status, code));
        });
        let start = iam.count();
        let (v, e) = h
            .run(&["jobs", "wait", &id, "--profile", "supply", "--timeout", "1"])
            .await;
        assert_failure(&v, e, exit, code);
        assert_eq!(v["meta"]["actions"][0]["actor"], actor);
        assert_eq!(v["meta"]["complete"], false);
        assert!(v["data"]["items"].is_null());
        assert_eq!(v["data"]["jobId"], id);
        assert!(iam.since(start).iter().all(|r| r.method == "GET"));
    }
}
