use dt_cli::credentials::{CredentialStore, Credentials, SystemStore};

#[test]
#[ignore = "explicit OS secure-store smoke test; writes and deletes only a random synthetic test entry"]
fn system_store_round_trip() {
    let key = format!("fixture/native-store/{}", rand::random::<u128>());
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = SystemStore.delete(&self.0);
        }
    }
    let cleanup = Cleanup(key.clone());
    let value = Credentials {
        access_token: "synthetic-not-a-real-access-token".into(),
        refresh_token: "synthetic-not-a-real-refresh-token".into(),
        version: 1,
        access_expires_at: None,
        authorization_expires_at: None,
        generation: 0,
        refresh_state: "ready".into(),
        refresh_request_id: None,
    };
    assert!(SystemStore.read(&key).unwrap().is_none());
    SystemStore.write(&key, &value).unwrap();
    let result = SystemStore.read(&key).unwrap().unwrap();
    assert_eq!(result.access_token, value.access_token);
    assert_eq!(result.refresh_token, value.refresh_token);
    SystemStore.delete(&key).unwrap();
    assert!(SystemStore.read(&key).unwrap().is_none());
    drop(cleanup);
}

#[cfg(target_os = "macos")]
#[test]
#[ignore = "explicit Keychain denial test using a disposable, restricted synthetic entry"]
fn system_store_denied_access_returns_without_a_prompt() {
    let key = format!("fixture/restricted-store/{}", rand::random::<u128>());
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::process::Command::new("security")
                .args([
                    "delete-generic-password",
                    "-s",
                    "com.datousoft.dt-cli",
                    "-a",
                    &self.0,
                ])
                .output();
        }
    }
    let cleanup = Cleanup(key.clone());
    let added = std::process::Command::new("security")
        .args([
            "add-generic-password",
            "-s",
            "com.datousoft.dt-cli",
            "-a",
            &key,
            "-w",
            "synthetic-restricted-fixture",
            "-T",
            "/usr/bin/security",
        ])
        .output()
        .unwrap();
    assert!(
        added.status.success(),
        "could not create restricted fixture"
    );
    let start = std::time::Instant::now();
    let result = SystemStore.read(&key);
    assert!(matches!(result,Err(ref e) if e.code == "CREDENTIAL_STORE_UNAVAILABLE"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    drop(cleanup);
}

#[test]
#[ignore = "explicit malformed-record test using a disposable synthetic entry"]
fn malformed_record_is_distinguished_and_retained() {
    let key = format!("fixture/malformed-store/{}", rand::random::<u128>());
    let entry = keyring::Entry::new("com.datousoft.dt-cli", &key).unwrap();
    entry.set_password("synthetic-malformed-record").unwrap();
    let result = SystemStore.read(&key);
    let retained = entry.get_password().unwrap();
    entry.delete_credential().unwrap();
    assert!(matches!(result,Err(ref e) if e.code == "CREDENTIAL_DECODE_FAILED"));
    assert_eq!(retained, "synthetic-malformed-record");
}

#[test]
#[ignore = "explicit OS integrity-key smoke test; creates and deletes only a random synthetic entry"]
fn integrity_key_is_opaque_and_kept_separate_from_oauth_credentials() {
    use zeroize::Zeroizing;
    let key = format!("fixture/native-integrity/{}", rand::random::<u128>());
    struct Cleanup(String);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = SystemStore.delete(&self.0);
        }
    }
    let _cleanup = Cleanup(key.clone());
    let value = Zeroizing::new(vec![0x93u8; 32]);
    assert!(SystemStore.read_integrity_key(&key).unwrap().is_none());
    SystemStore.write_integrity_key(&key, &value).unwrap();
    let read = Zeroizing::new(SystemStore.read_integrity_key(&key).unwrap().unwrap());
    assert_eq!(*read, *value);
    assert!(SystemStore.read(&key).is_err());
    assert_eq!(
        SystemStore.read_integrity_key(&key).unwrap().unwrap(),
        *value
    );
    SystemStore.delete(&key).unwrap();
    assert!(SystemStore.read_integrity_key(&key).unwrap().is_none());
}
