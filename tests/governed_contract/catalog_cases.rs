use super::*;

#[tokio::test]
async fn login_creates_an_isolated_governed_profile_and_catalog() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    let value = h.login("supply", ENV, SYSTEM).await;
    let data = &value["data"];
    assert_eq!(data["provider"], "governed");
    assert_eq!(data["systemId"], SYSTEM);
    assert_eq!(data["environment"], ENV);
    assert_eq!(data["subjectId"], "1");
    assert_eq!(data["operationCount"], 3);
    assert_eq!(data["consentedCount"], 2);
    assert_eq!(data["permissionVerified"], true);
    let authorize = iam.with(|f| f.authorize.clone());
    assert_eq!(authorize.len(), 1);
    for (field, expected) in [
        ("systemId", SYSTEM),
        ("environment", ENV),
        ("client_id", "dt-cli"),
        ("response_type", "code"),
        ("code_challenge_method", "S256"),
    ] {
        assert_eq!(authorize[0][field], expected, "{field}");
    }
    assert_eq!(
        iam.paths_since(0),
        vec!["POST /cli-api/oauth2/token", "GET /cli-api/v1/catalog"]
    );
    assert!(h.file("supply.profile.json").exists());
    assert!(h.file("supply.catalog.json").exists());
    assert!(!h.root.path().join("supply.json").exists());
    let key = h.store.governed_key();
    assert!(key.contains("/fixture/supply-chain-server/1/"), "{key}");
    assert!(key.ends_with("/dt-cli"), "{key}");
    let stored = h.store.get(&key);
    assert!(stored.access_token.starts_with("dtcli_g_a_"));
    assert!(stored.refresh_token.starts_with("dtcli_g_r_"));
    for file in ["supply.profile.json", "supply.catalog.json"] {
        let text = std::fs::read_to_string(h.file(file)).unwrap();
        assert!(!text.contains("dtcli_g_"), "{file} stored a credential");
    }
    let (status, exit) = h.run(&["auth", "status", "--profile", "supply"]).await;
    assert_eq!(exit, 0, "{status}");
    assert_eq!(status["data"]["provider"], "governed");
    assert_eq!(status["data"]["credentialPresent"], true);
    assert_eq!(status["data"]["onlineVerified"], false);
    let (synced, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_eq!(exit, 0, "{synced}");
    assert_eq!(synced["data"]["catalogRevision"], "7");
    assert_eq!(synced["data"]["onlineVerified"], true);
}

// A2 离线目录 + 参数先验：发现/Schema/dry-run 与参数错误均不读凭证、不发请求。
#[tokio::test]
async fn offline_catalog_and_argument_errors_have_no_side_effects() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let sealed = h.sealed();
    let before = iam.count();
    let (found, exit) = h
        .run_with(&sealed, &["discover", "--profile", "supply"])
        .await;
    assert_eq!(exit, 0, "{found}");
    assert_eq!(found["data"]["permissionVerified"], false);
    assert_eq!(found["data"]["onlineVerified"], false);
    assert_eq!(found["data"]["operations"].as_array().unwrap().len(), 3);
    assert_eq!(found["data"]["operations"][0]["cliCompatible"], true);
    let (batch, _) = h
        .run_with(
            &sealed,
            &["discover", "--profile", "supply", "--query", "batch"],
        )
        .await;
    assert_eq!(batch["data"]["operations"].as_array().unwrap().len(), 1);
    let (schema, exit) = h
        .run_with(&sealed, &["schema", "--profile", "supply", READ])
        .await;
    assert_eq!(exit, 0, "{schema}");
    assert_eq!(schema["data"]["schema"]["operationId"], READ);
    assert_eq!(schema["data"]["onlineVerified"], false);
    let (preview, exit) = h
        .run_with(
            &sealed,
            &[
                "api",
                "call",
                READ,
                "--profile",
                "supply",
                "--dry-run",
                "--params",
                READ_PARAMS,
            ],
        )
        .await;
    assert_eq!(exit, 0, "{preview}");
    assert_eq!(preview["data"]["preview"], true);
    assert_eq!(preview["data"]["credentialReads"], 0);
    assert_eq!(preview["data"]["httpRequests"], 0);
    assert_eq!(
        preview["data"]["contractDigest"],
        digest(&format!("{READ}@1.0.0"))
    );
    for args in [
        vec!["auth", "login", "--profile", "supply", "--dry-run"],
        vec!["auth", "logout", "--profile", "supply", "--dry-run"],
        vec!["whoami", "--profile", "supply", "--dry-run"],
    ] {
        let (value, exit) = h.run_with(&sealed, &args).await;
        assert_eq!(exit, 0, "{args:?}: {value}");
        assert_eq!(value["data"]["preview"], true, "{args:?}");
        assert_eq!(value["data"]["httpRequests"], 0, "{args:?}");
    }
    let oversize = format!(r#"{{"shopCode":"{}","itemCode":"1"}}"#, "A".repeat(70_000));
    for params in [
        r#"{"itemCode":"101010009"}"#,
        r#"{"shopCode":2021003,"itemCode":"101010009"}"#,
        r#"{"shopCode":"2021003","itemCode":"101010009","url":"https://evil.invalid"}"#,
        r#"{"shopCode":"2021003","shopCode":"9","itemCode":"101010009"}"#,
        r#"{"shopCode":"2021003","itemCode":"101010009","itemName":"白芝麻"}"#,
        r#"{"shopCode":"../../admin","itemCode":"101010009"}"#,
        "[]",
        "{",
        oversize.as_str(),
    ] {
        let (value, exit) = h
            .run_with(
                &sealed,
                &[
                    "api",
                    "call",
                    READ,
                    "--profile",
                    "supply",
                    "--params",
                    params,
                ],
            )
            .await;
        assert_failure(&value, exit, 2, "INVALID_ARGUMENT");
    }
    let (value, exit) = h
        .run_with(
            &sealed,
            &[
                "jobs",
                "submit",
                BATCH,
                "--profile",
                "supply",
                "--params",
                r#"{"itemCode":"101010009","itemName":"x"}"#,
            ],
        )
        .await;
    assert_failure(&value, exit, 2, "INVALID_ARGUMENT");
    let (value, exit) = h
        .run_with(
            &sealed,
            &["jobs", "status", "not-a-job-id", "--profile", "supply"],
        )
        .await;
    assert_failure(&value, exit, 2, "INVALID_ARGUMENT");
    assert_eq!(iam.count(), before, "no request may be sent");
}

// A2 调用 + A3 固定合同：在线目录复核、精确合同换证、Schema 投影、系统 token 不外泄。
