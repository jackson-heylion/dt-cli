use super::*;
use std::path::Path;

struct DeleteFailureStore(FileStore);

impl CredentialStore for DeleteFailureStore {
    fn read(&self, key: &str) -> Result<Option<Credentials>> {
        self.0.read(key)
    }
    fn write(&self, key: &str, value: &Credentials) -> Result<()> {
        self.0.write(key, value)
    }
    fn delete(&self, _: &str) -> Result<()> {
        Err(crate::output::storage())
    }
}

#[tokio::test]
async fn replacing_a_session_succeeds_even_when_previous_credential_cleanup_fails() {
    for purpose in [SessionPurpose::Cli, SessionPurpose::Management] {
        let server = Server::start(Scenario::default(), purpose).await;
        let endpoint = endpoints::resolve("fixture", Some(&server.origin)).unwrap();
        let root = tempfile::tempdir().unwrap();
        let mut rt = runtime(root.path(), browser(true, false, 0), false);
        let client = local_client();
        let name = "returning";
        login_with_client(
            &rt,
            name,
            &endpoint,
            "platform",
            false,
            purpose,
            LoginIo::new(&client),
        )
        .await
        .unwrap();
        let mut prior = read_profile_for(&rt, name, purpose).unwrap();
        prior.expires_at = "2000-01-01T00:00:00Z".into();
        private_store::write(&profile_path_for(&rt, name, purpose), &prior).unwrap();
        rt.store = Box::new(DeleteFailureStore(FileStore::new(root.path())));

        let result = login_with_client(
            &rt,
            name,
            &endpoint,
            "platform",
            false,
            purpose,
            LoginIo::new(&client),
        )
        .await;
        let (out, exit) =
            crate::output::envelope_result("login", Some(name), result.and_then(serialize));
        assert_eq!(exit, 0, "{out}");
        assert_eq!(out["ok"], true);
        assert_eq!(out["data"]["purpose"], purpose.text());
        assert_eq!(out["data"]["warning"], "previous_credential_cleanup_failed");
        let saved = read_profile_for(&rt, name, purpose).unwrap();
        assert_ne!(saved.credential_key, prior.credential_key);
        assert!(rt.store.read(&saved.credential_key).unwrap().is_some());
        assert!(rt.store.read(&prior.credential_key).unwrap().is_some());
    }
}

fn write_error() -> Failure {
    private_store::diagnostic("sync_directory", "io_error", None)
}

fn before_replace(_: &Path, _: &PlatformProfile) -> Result<()> {
    Err(write_error())
}

fn after_replace(path: &Path, record: &PlatformProfile) -> Result<()> {
    private_store::write(path, record)?;
    Err(write_error())
}

fn uncertain_record(path: &Path, _: &PlatformProfile) -> Result<()> {
    std::fs::create_dir(path).unwrap();
    Err(write_error())
}

#[tokio::test]
async fn save_error_only_compensates_when_the_account_record_did_not_switch() {
    type Writer = fn(&Path, &PlatformProfile) -> Result<()>;
    for purpose in [SessionPurpose::Cli, SessionPurpose::Management] {
        for (writer, compensates, record_visible) in [
            (before_replace as Writer, true, false),
            (after_replace as Writer, false, true),
            (uncertain_record as Writer, false, false),
        ] {
            let server = Server::start(
                Scenario {
                    revocation_status: 502,
                    ..Default::default()
                },
                purpose,
            )
            .await;
            let endpoint = endpoints::resolve("fixture", Some(&server.origin)).unwrap();
            let root = tempfile::tempdir().unwrap();
            let mut rt = runtime(root.path(), browser(true, false, 0), false);
            if compensates {
                rt.store = Box::new(DeleteFailureStore(FileStore::new(root.path())));
            }
            let client = local_client();
            let result = login_with_client(
                &rt,
                "pending",
                &endpoint,
                "platform",
                false,
                purpose,
                LoginIo {
                    client: &client,
                    write_profile: writer,
                },
            )
            .await;
            let (out, exit) = crate::output::envelope_result(
                "login",
                Some("pending"),
                result.and_then(serialize),
            );
            assert_eq!(exit, 1, "{out}");
            assert_eq!(out["ok"], false);
            assert_eq!(out["error"]["code"], "LOCAL_STATE_UNAVAILABLE");
            assert_eq!(out["error"]["details"]["stage"], "sync_directory");
            assert_eq!(
                out["error"]["details"]["loginStage"],
                "saving_local_session"
            );
            assert_eq!(out["error"]["retryable"], false);
            assert_eq!(
                out["meta"]["loginSave"],
                if compensates {
                    "not_saved"
                } else {
                    "unconfirmed"
                }
            );
            assert_eq!(
                out["meta"]["remoteRevocation"],
                if compensates {
                    "unknown"
                } else {
                    "not-attempted"
                }
            );
            assert_eq!(
                out["meta"]["credentialCleanup"],
                if compensates {
                    "failed"
                } else {
                    "not_attempted"
                }
            );
            let requests = server.requests.lock().unwrap();
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.path.ends_with("/oauth2/token"))
                    .count(),
                1
            );
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.path.ends_with("/oauth2/revoke"))
                    .count(),
                usize::from(compensates)
            );
            assert_eq!(
                std::fs::read_dir(root.path().join("credentials"))
                    .unwrap()
                    .count(),
                1
            );
            if record_visible {
                let saved = read_profile_for(&rt, "pending", purpose).unwrap();
                assert!(rt.store.read(&saved.credential_key).unwrap().is_some());
            }
            if !compensates {
                let mut command = vec!["dt-cli"];
                if purpose == SessionPurpose::Management {
                    command.push("manage");
                }
                command.extend(["check", "--profile=pending"]);
                assert_eq!(
                    out["meta"]["recovery"]["actions"][0]["argv"],
                    serde_json::json!(command)
                );
                assert_eq!(
                    out["meta"]["recovery"]["actions"][0]["requiresInteraction"],
                    false
                );
            }
        }
    }
}
