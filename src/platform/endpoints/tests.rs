//! 使用原生私有存储核对旧记录校验；无凭证读取或真实浏览器操作。
use super::*;
use crate::{
    Runtime,
    credentials::{CredentialStore, Credentials},
    login::Browser,
    platform::model::{IdentityDomain, PlatformProfile, SessionPurpose},
    private_store,
};

struct UnusedStore;

impl CredentialStore for UnusedStore {
    fn read(&self, _: &str) -> Result<Option<Credentials>> {
        panic!("入口校验前读取了凭证")
    }
    fn write(&self, _: &str, _: &Credentials) -> Result<()> {
        panic!("入口校验前写入了凭证")
    }
    fn delete(&self, _: &str) -> Result<()> {
        panic!("入口校验前删除了凭证")
    }
}

struct UnusedBrowser;

impl Browser for UnusedBrowser {
    fn open(&self, _: &str) -> Result<()> {
        panic!("入口校验前请求了浏览器")
    }
}

fn runtime(root: &std::path::Path) -> Runtime {
    Runtime {
        root: root.to_owned(),
        store: Box::new(UnusedStore),
        browser: Box::new(UnusedBrowser),
        interactive: false,
        aggregate_budget: std::time::Duration::from_secs(30),
    }
}

fn record(origin: &str, environment: &str, purpose: SessionPurpose) -> PlatformProfile {
    PlatformProfile {
        purpose,
        schema_version: 1,
        origin: origin.to_owned(),
        environment: environment.to_owned(),
        system_id: "platform".into(),
        subject_id: "fixture-employee".into(),
        domain: IdentityDomain::Employee,
        session_id: "A".repeat(43),
        expires_at: "2099-01-01T00:00:00Z".into(),
        credential_key: format!(
            "{}:{}:existing:{}",
            purpose.directory(),
            super::super::transport::challenge(origin),
            "K".repeat(43)
        ),
    }
}

fn arguments(purpose: SessionPurpose, action: &str) -> Vec<String> {
    let mut args = vec!["dt-cli"];
    if purpose == SessionPurpose::Management {
        args.push("manage");
    }
    args.extend([action, "--profile", "existing"]);
    args.into_iter().map(str::to_owned).collect()
}

#[tokio::test]
async fn stored_origin_is_checked_before_credentials_and_remote_requests() {
    for purpose in [SessionPurpose::Cli, SessionPurpose::Management] {
        for (origin, environment) in [
            ("https://iadmin.jmj1995.com", "prod"),
            (PRODUCTION_ORIGIN, "stg"),
            (STAGING_ORIGIN, "prod"),
            ("https://gateway.jmj1995.com", "stg"),
            (PRODUCTION_ORIGIN, "fixture"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let rt = runtime(root.path());
            let path = crate::platform::profile_path_for(&rt, "existing", purpose);
            private_store::write(&path, &record(origin, environment, purpose)).unwrap();
            let original = std::fs::read(&path).unwrap();
            let (reply, exit, _) = crate::execute(&rt, arguments(purpose, "check")).await;
            assert_eq!(exit, 2, "{reply}");
            assert_eq!(reply["error"]["code"], "PLATFORM_PROFILE_ORIGIN_MISMATCH");
            assert_eq!(std::fs::read(path).unwrap(), original);
        }
    }
}

#[tokio::test]
async fn default_production_does_not_reuse_an_existing_staging_account() {
    for purpose in [SessionPurpose::Cli, SessionPurpose::Management] {
        let root = tempfile::tempdir().unwrap();
        let rt = runtime(root.path());
        let path = crate::platform::profile_path_for(&rt, "existing", purpose);
        private_store::write(&path, &record(STAGING_ORIGIN, "stg", purpose)).unwrap();
        let original = std::fs::read(&path).unwrap();
        let (reply, exit, _) = crate::execute(&rt, arguments(purpose, "login")).await;
        assert_eq!(exit, 2, "{reply}");
        assert_eq!(reply["error"]["code"], "PROFILE_EXISTS");
        assert_eq!(std::fs::read(path).unwrap(), original);
    }
}

#[test]
fn local_fixture_cannot_override_production_or_staging() {
    for environment in ["prod", "stg"] {
        assert_eq!(
            resolve(environment, Some("http://127.0.0.1:49152"))
                .err()
                .unwrap()
                .code,
            "PLATFORM_ORIGIN_MISMATCH",
        );
    }
    let fixture = resolve("fixture", Some("http://127.0.0.1:49152")).unwrap();
    assert_eq!(fixture.environment(), "fixture");
    assert_eq!(fixture.origin(), "http://127.0.0.1:49152");
    assert_eq!(
        resolve("fixture", Some("https://iadmin.jmj1995.com"))
            .err()
            .unwrap()
            .code,
        "PLATFORM_ENVIRONMENT_UNSUPPORTED",
    );
}

#[test]
fn production_and_staging_select_the_fixed_origin() {
    for (environment, expected) in [("prod", PRODUCTION_ORIGIN), ("stg", STAGING_ORIGIN)] {
        for record_origin in [None, Some(expected)] {
            let endpoint = resolve(environment, record_origin).unwrap();
            assert_eq!(endpoint.origin(), expected);
            assert_eq!(endpoint.environment(), environment);
        }
    }
}
