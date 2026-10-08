use super::*;

#[tokio::test]
async fn logout_reports_remote_and_local_outcomes_separately() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (value, exit) = h.run(&["auth", "logout", "--profile", "supply"]).await;
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["data"]["remoteRevocation"], "confirmed");
    assert_eq!(value["data"]["localCleanup"], "confirmed");
    assert!(iam.with(|f| f.revoked));
    assert!(!h.file("supply.profile.json").exists());
    assert!(!h.file("supply.catalog.json").exists());
    assert!(h.store.keys().is_empty());
    let (value, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_failure(&value, exit, 2, "PROFILE_NOT_CONFIGURED");
    h.login("supply", ENV, SYSTEM).await;
    iam.with(|f| {
        f.overrides.insert(
            "revoke",
            oauth_error(503, "temporarily_unavailable", "DEPENDENCY_UNAVAILABLE"),
        )
    });
    let (value, exit) = h.run(&["auth", "logout", "--profile", "supply"]).await;
    assert_failure(&value, exit, 7, "LOGOUT_PARTIAL");
    assert_eq!(value["meta"]["recovery"]["remoteRevocation"], "unknown");
    assert_eq!(value["meta"]["recovery"]["localCleanup"], "confirmed");
    assert!(!h.file("supply.profile.json").exists());
    assert!(h.store.keys().is_empty());
    iam.with(|f| {
        f.overrides.remove("revoke");
    });
    h.login("supply", ENV, SYSTEM).await;
    iam.with(|f| {
        f.overrides.insert("revoke", reply(202, json!({})));
    });
    let (value, exit) = h.run(&["auth", "logout", "--profile", "supply"]).await;
    assert_failure(&value, exit, 7, "LOGOUT_PARTIAL");
    assert_eq!(value["meta"]["recovery"]["remoteRevocation"], "unknown");
}

// 登录失败路径：拒绝、校验失败、非交互与参数错误都不留下 profile 或凭证。
#[tokio::test]
async fn failed_logins_leave_no_profile_or_credentials() {
    let iam = supply();
    let denied = Harness::build(&[(ENV, &iam)], true, true);
    let (value, exit) = denied
        .run(&[
            "auth",
            "login",
            "--profile",
            "supply",
            "--environment",
            ENV,
            "--system",
            SYSTEM,
        ])
        .await;
    assert_failure(&value, exit, 4, "AUTH_DENIED");
    assert!(!denied.file("supply.profile.json").exists());
    assert!(denied.store.keys().is_empty());
    let h = Harness::new(&[(ENV, &iam)]);
    iam.with(|f| {
        f.overrides
            .insert("catalog", failure(503, "DEPENDENCY_UNAVAILABLE"))
    });
    let (value, exit) = h
        .run(&[
            "auth",
            "login",
            "--profile",
            "supply",
            "--environment",
            ENV,
            "--system",
            SYSTEM,
        ])
        .await;
    assert_failure(&value, exit, 5, "DEPENDENCY_UNAVAILABLE");
    assert_eq!(value["meta"]["recovery"]["remoteRevocation"], "confirmed");
    assert_eq!(
        value["meta"]["recovery"]["newCredentialCleanup"],
        "not-created"
    );
    assert_eq!(iam.seen().last().unwrap().path, "/cli-api/oauth2/revoke");
    assert!(!h.file("supply.profile.json").exists());
    assert!(!h.file("supply.catalog.json").exists());
    assert!(h.store.keys().is_empty());
    iam.with(|f| f.overrides.clear());
    let before = iam.count();
    let quiet = Harness::build(&[(ENV, &iam)], false, false);
    let (value, exit) = quiet
        .run(&[
            "auth",
            "login",
            "--profile",
            "supply",
            "--environment",
            ENV,
            "--system",
            SYSTEM,
        ])
        .await;
    assert_failure(&value, exit, 8, "INTERACTION_REQUIRED");
    for args in [
        vec![
            "auth",
            "login",
            "--profile",
            "x",
            "--environment",
            ENV,
            "--system",
            "bad/system",
        ],
        vec!["auth", "login", "--profile", "x", "--system", SYSTEM],
    ] {
        let (value, exit) = h.run(&args).await;
        assert_failure(&value, exit, 2, "INVALID_ARGUMENT");
    }
    let (value, exit) = h
        .run(&[
            "auth",
            "login",
            "--profile",
            "x",
            "--environment",
            "unknown-env",
            "--system",
            SYSTEM,
        ])
        .await;
    assert_failure(&value, exit, 2, "ENVIRONMENT_NOT_CONFIGURED");
    assert_eq!(iam.count(), before);
}
