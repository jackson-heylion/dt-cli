use super::*;

#[tokio::test]
async fn concurrent_governed_reads_wait_for_the_inflight_refresh() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let key = h.store.governed_key();
    let mut expired = h.store.get(&key);
    expired.access_expires_at =
        Some((chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339());
    h.store.write(&key, &expired).unwrap();
    let start = iam.count();
    let (first, second) = tokio::join!(
        h.run(&["auth", "check", "--profile", "supply"]),
        h.run(&["auth", "check", "--profile", "supply"]),
    );
    assert_eq!(first.1, 0, "{}", first.0);
    assert_eq!(second.1, 0, "{}", second.0);
    assert_eq!(h.store.get(&key).generation, 1);
    assert_eq!(
        iam.paths_since(start)
            .iter()
            .filter(|path| path.ends_with("/token"))
            .count(),
        1
    );
}

#[tokio::test]
async fn refresh_rotates_once_and_replay_revokes_the_authorization() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let key = h.store.governed_key();
    let original = h.store.get(&key);
    let mut expired = original.clone();
    expired.access_expires_at =
        Some((chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339());
    h.store.write(&key, &expired).unwrap();
    let start = iam.count();
    let (value, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(
        iam.paths_since(start),
        vec!["POST /cli-api/oauth2/token", "GET /cli-api/v1/catalog"]
    );
    let refresh = iam.since(start)[0].form();
    assert_eq!(refresh["grant_type"], "refresh_token");
    assert_eq!(refresh["refresh_token"], original.refresh_token);
    let rotated = h.store.get(&key);
    assert_eq!(rotated.generation, 1);
    assert_eq!(rotated.refresh_state, "ready");
    assert_ne!(rotated.access_token, original.access_token);
    assert_eq!(
        iam.since(start)[1].authorization.as_deref(),
        Some(format!("Bearer {}", rotated.access_token).as_str())
    );
    // An explicit 401 on a locally valid token allows exactly one refresh and one resend.
    iam.with(|f| f.catalog_401_once = true);
    let start = iam.count();
    let (value, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(
        iam.paths_since(start),
        vec![
            "GET /cli-api/v1/catalog",
            "POST /cli-api/oauth2/token",
            "GET /cli-api/v1/catalog",
        ]
    );
    // A stale copy replays a retired refresh token: the authorization is revoked.
    h.store.write(&key, &expired).unwrap();
    let (value, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_failure(&value, exit, 4, "AUTHORIZATION_REVOKED");
    assert_eq!(h.store.get(&key).refresh_state, "ready");
    let mut fresh = h.store.get(&key);
    fresh.access_token = iam.with(|f| f.access.clone());
    fresh.access_expires_at = Some((chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339());
    h.store.write(&key, &fresh).unwrap();
    let (value, exit) = h
        .run(&[
            "api",
            "call",
            READ,
            "--profile",
            "supply",
            "--params",
            READ_PARAMS,
        ])
        .await;
    assert_failure(&value, exit, 4, "AUTHORIZATION_REVOKED");
    // An older in-flight record has no recovery nonce and never resends the token.
    let mut pending = fresh.clone();
    pending.refresh_state = "pending".into();
    h.store.write(&key, &pending).unwrap();
    let before = iam.count();
    let (value, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_failure(&value, exit, 4, "AUTH_REFRESH_OUTCOME_UNKNOWN");
    assert_eq!(iam.count(), before);
}

// 撤销：本地清理与远端撤销分别报告。
#[tokio::test]
async fn lost_refresh_response_recovers_saved_nonce_and_stale_copy_cannot_recover() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let key = h.store.governed_key();
    let mut original = h.store.get(&key);
    original.access_expires_at =
        Some((chrono::Utc::now() - chrono::Duration::minutes(1)).to_rfc3339());
    h.store.write(&key, &original).unwrap();
    iam.with(|f| f.drop_refresh_reply_once = true);
    let (_, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_ne!(exit, 0);
    let pending = h.store.get(&key);
    assert_eq!(pending.refresh_state, "pending");
    let nonce = pending.refresh_request_id.as_ref().unwrap().clone();
    let (value, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_eq!(exit, 0, "{value}");
    let requests = iam.with(|f| {
        f.seen
            .iter()
            .filter(|r| {
                r.path.ends_with("/token")
                    && r.form().get("grant_type").map(String::as_str) == Some("refresh_token")
            })
            .map(Seen::form)
            .collect::<Vec<_>>()
    });
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["request_id"], nonce);
    assert_eq!(requests[1]["request_id"], nonce);
    let rotated = h.store.get(&key);
    assert_eq!(rotated.generation, 1);
    assert!(rotated.refresh_request_id.is_none());
    h.store.write(&key, &original).unwrap();
    let (value, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_failure(&value, exit, 4, "AUTHORIZATION_REVOKED");
    let replay = iam.with(|f| f.seen.last().unwrap().form());
    assert_ne!(replay["request_id"], nonce);
}
