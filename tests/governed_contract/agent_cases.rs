use super::*;

#[tokio::test]
async fn legacy_confirmation_contract_is_refused_before_credentials_or_http() {
    let iam = hrmp();
    iam.with(|f| f.operations[0]["confirmation"]["channel"] = json!("iam-browser"));
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("hrmp", ENV, "hrmp").await;
    let before = iam.count();
    let (value, exit) = h
        .run_with(
            &h.sealed(),
            &[
                "intents",
                "prepare",
                LIKE,
                "--profile",
                "hrmp",
                "--params",
                LIKE_PARAMS,
            ],
        )
        .await;
    assert_failure(&value, exit, 6, "CONTRACT_CHANGED");
    assert_eq!(iam.count(), before);
}

#[tokio::test]
async fn cli_authorization_binds_digests_and_never_opens_a_browser_or_sends() {
    let iam = hrmp();
    let mut h = Harness::new(&[(ENV, &iam)]);
    h.login("hrmp", ENV, "hrmp").await;
    h.rt.browser = Box::new(NoBrowser);
    let (prepared, exit) = h
        .run(&[
            "intents",
            "prepare",
            LIKE,
            "--profile",
            "hrmp",
            "--params",
            LIKE_PARAMS,
        ])
        .await;
    assert_eq!(exit, 0, "{prepared}");
    let id = prepared["data"]["intentId"].as_str().unwrap();
    let arguments = prepared["data"]["argumentsDigest"].as_str().unwrap();
    let contract = prepared["data"]["contractDigest"].as_str().unwrap();
    let (bad, exit) = h
        .run(&[
            "intents",
            "authorize",
            id,
            "--profile",
            "hrmp",
            "--arguments-digest",
            &"0".repeat(64),
            "--contract-digest",
            contract,
        ])
        .await;
    assert_failure(&bad, exit, 6, "CONTRACT_CHANGED");
    let (approved, exit) = h
        .run(&[
            "intents",
            "authorize",
            id,
            "--profile",
            "hrmp",
            "--arguments-digest",
            arguments,
            "--contract-digest",
            contract,
        ])
        .await;
    assert_eq!(exit, 0, "{approved}");
    assert_eq!(approved["data"]["state"], "approved");
    assert_eq!(approved["data"]["sideEffect"], "none");
    assert_eq!(iam.with(|f| f.writes), 0);
    let (sent, exit) = h.run(&["intents", "invoke", id, "--profile", "hrmp"]).await;
    assert_eq!(exit, 0, "{sent}");
    assert_eq!(iam.with(|f| f.writes), 1);
}

#[tokio::test]
async fn wait_observes_cli_authorization_and_dispatches_without_browser() {
    let iam = hrmp();
    let mut h = Harness::new(&[(ENV, &iam)]);
    h.login("hrmp", ENV, "hrmp").await;
    h.rt.browser = Box::new(NoBrowser);
    let approve = async {
        let prepared = loop {
            if let Some(view) = iam.with(|f| f.intents.values().next().cloned()) {
                break view;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        };
        h.run(&[
            "intents",
            "authorize",
            prepared["intentId"].as_str().unwrap(),
            "--profile",
            "hrmp",
            "--arguments-digest",
            prepared["argumentsDigest"].as_str().unwrap(),
            "--contract-digest",
            prepared["contractDigest"].as_str().unwrap(),
        ])
        .await
    };
    let (sent, approved) = tokio::join!(
        h.run(&[
            "intents",
            "prepare",
            LIKE,
            "--profile",
            "hrmp",
            "--params",
            LIKE_PARAMS,
            "--wait"
        ]),
        approve
    );
    assert_eq!(approved.1, 0, "{}", approved.0);
    assert_eq!(sent.1, 0, "{}", sent.0);
    assert_eq!(sent.0["data"]["state"], "succeeded");
    assert_eq!(iam.with(|f| f.writes), 1);
}
