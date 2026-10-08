use super::*;
use crate::{
    credentials::{CredentialStore, Credentials},
    login::Browser,
    profile::Environment,
};
use std::{collections::BTreeMap, sync::Mutex, time::Duration};
struct Store(Mutex<Option<Vec<u8>>>);
impl CredentialStore for Store {
    fn read(&self, _: &str) -> Result<Option<Credentials>> {
        panic!("OAuth access")
    }
    fn write(&self, _: &str, _: &Credentials) -> Result<()> {
        panic!("OAuth access")
    }
    fn delete(&self, _: &str) -> Result<()> {
        panic!("OAuth access")
    }
    fn read_integrity_key(&self, _: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn write_integrity_key(&self, _: &str, key: &[u8]) -> Result<()> {
        *self.0.lock().unwrap() = Some(key.to_vec());
        Ok(())
    }
}
impl Browser for Store {
    fn open(&self, _: &str) -> Result<()> {
        panic!("browser access")
    }
}
fn rt(root: &std::path::Path) -> Runtime {
    Runtime {
        root: root.into(),
        environments: BTreeMap::from([(
            "fixture".into(),
            Environment {
                api_origin: "https://fixture.invalid".into(),
                portal_origin: "https://fixture.invalid".into(),
                recovery_url: "https://fixture.invalid/recovery".into(),
                launch_path: "/cli/launch".into(),
            },
        )]),
        store: Box::new(Store(Mutex::new(None))),
        browser: Box::new(Store(Mutex::new(None))),
        interactive: false,
        aggregate_budget: Duration::from_secs(30),
    }
}
async fn create(rt: &Runtime) -> Journal {
    let binding = Binding {
        profile: "fixture".into(),
        provider: "governed".into(),
        issuer: "https://fixture.invalid".into(),
        environment: "fixture".into(),
        system_id: Some("hrmp".into()),
        subject_id: "1".into(),
        authorization_id: "auth".into(),
        authorization_expires_at: "2030-01-01T00:00:00Z".into(),
    };
    Journal::create(rt,"like.send",binding,&json!({"operationId":"hrmp.like.send","operationVersion":"1.0.0","contractDigest":"a".repeat(64)}),&json!({"x":"固定"}),&json!({"x":"固定"})).await.unwrap()
}
#[tokio::test]
async fn cleanup_only_removes_verified_terminal_records_after_24_hours() {
    let dir = tempfile::tempdir().unwrap();
    let rt = rt(dir.path());
    let mut terminal = create(&rt).await;
    terminal.record.remote_id = Some("I".repeat(43));
    terminal.record.created_at = Utc::now() - chrono::Duration::hours(48);
    terminal.record.observed_at = terminal.record.created_at;
    terminal.record.confirmed_terminal_at = Some(Utc::now() - chrono::Duration::hours(25));
    terminal.save().unwrap();
    let terminal_path = terminal.path.clone();
    drop(terminal);
    // An unresolved record is old but never eligible for time-based cleanup.
    let mut unknown = create(&rt).await;
    assert!(!terminal_path.exists());
    unknown.record.remote_id = Some("U".repeat(43));
    unknown.record.created_at = Utc::now() - chrono::Duration::hours(48);
    unknown.record.observed_at = unknown.record.created_at;
    unknown.step("dispatch-attempted").unwrap();
    let unknown_path = unknown.path.clone();
    drop(unknown);
    let mut missing = create(&rt).await;
    missing.record.created_at = Utc::now() - chrono::Duration::hours(48);
    missing.record.observed_at = missing.record.created_at;
    missing.record.confirmed_terminal_at = Some(Utc::now() - chrono::Duration::hours(25));
    missing.save().unwrap();
    let missing_path = missing.path.clone();
    drop(missing);
    let latest = create(&rt).await;
    assert!(unknown_path.exists());
    assert!(missing_path.exists());
    assert!(latest.path.exists());
    let sealed: Sealed = private_store::read(&unknown_path).unwrap().unwrap();
    verify(&latest.key, &sealed).unwrap();
}
#[tokio::test]
async fn a_locked_terminal_record_is_not_cleaned_and_lock_wait_does_not_block_executor() {
    let dir = tempfile::tempdir().unwrap();
    let rt = rt(dir.path());
    let mut active = create(&rt).await;
    active.record.remote_id = Some("I".repeat(43));
    active.record.created_at = Utc::now() - chrono::Duration::hours(48);
    active.record.observed_at = active.record.created_at;
    active.record.confirmed_terminal_at = Some(Utc::now() - chrono::Duration::hours(25));
    active.save().unwrap();
    let path = active.path.clone();
    let latest = create(&rt).await;
    assert!(path.exists());
    drop(latest);
    let lock_path = path.with_extension("lock");
    let waiting = lock::acquire(&lock_path);
    let release = async {
        tokio::time::sleep(Duration::from_millis(100)).await;
        drop(active);
    };
    let (lock, ()) = tokio::join!(waiting, release);
    drop(lock.unwrap());
    let latest = create(&rt).await;
    assert!(!path.exists());
    drop(latest);
}
#[test]
fn invalid_signature_cannot_turn_local_stage_into_dispatch_permission() {
    // A valid MAC on a serializable receipt is required; changes to any bound field fail.
    let record = Record {
        schema_version: 1,
        run_id: "R".repeat(43),
        task_id: "like.send".into(),
        recipe_version: 1,
        binding: Binding {
            profile: "x".into(),
            provider: "governed".into(),
            issuer: "https://x.invalid".into(),
            environment: "stg".into(),
            system_id: Some("hrmp".into()),
            subject_id: "1".into(),
            authorization_id: "auth".into(),
            authorization_expires_at: "2030-01-01T00:00:00Z".into(),
        },
        operation_id: "hrmp.like.send".into(),
        operation_version: "1.0.0".into(),
        contract_digest: "a".repeat(64),
        request_digest: "b".repeat(64),
        arguments_digest: "b".repeat(64),
        idempotency_key: Some("K".repeat(43)),
        remote_id: Some("I".repeat(43)),
        stage: "dispatch-attempted".into(),
        created_at: Utc::now(),
        observed_at: Utc::now(),
        confirmed_terminal_at: None,
    };
    let key = [7u8; 32];
    let signature = URL_SAFE_NO_PAD.encode(mac(&key, &record).unwrap().finalize().into_bytes());
    let mut sealed = Sealed { record, signature };
    verify(&key, &sealed).unwrap();
    sealed.record.stage = "created".into();
    assert!(verify(&key, &sealed).is_err());
    assert!(verify(&[9u8; 32], &sealed).is_err());
}
