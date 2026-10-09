use super::*;

#[tokio::test]
async fn read_call_is_verified_online_exchanged_exactly_and_projected() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let start = iam.count();
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
    assert_eq!(exit, 0, "{value}");
    assert_eq!(value["data"]["items"][0]["minOrderNum"], "1");
    assert_eq!(value["data"]["permissionVerified"], true);
    assert_eq!(
        value["meta"]["contractDigest"],
        digest(&format!("{READ}@1.0.0"))
    );
    assert_eq!(value["meta"]["state"], "succeeded");
    assert_eq!(value["meta"]["traceId"], "trace-invoke");
    assert!(value["meta"].get("upstreamHost").is_none(), "{value}");
    assert_eq!(
        iam.paths_since(start),
        vec![
            "POST /cli-api/oauth2/exchange",
            "POST /cli-api/v1/systems/supply-chain-server/invoke",
        ]
    );
    let seen = iam.since(start);
    let exchange = seen[0].form();
    let read = operation(READ, "1.0.0", true);
    assert_eq!(exchange["audience"], SYSTEM);
    assert_eq!(
        exchange["grant_type"],
        "urn:ietf:params:oauth:grant-type:token-exchange"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&exchange["operation_contracts"]).unwrap(),
        json!([key(&read)])
    );
    assert!(exchange["subject_token"].starts_with("dtcli_g_a_"));
    let system_token = iam.with(|f| f.system_tokens[0].clone());
    assert_eq!(
        seen[1].authorization.as_deref(),
        Some(format!("Bearer {system_token}").as_str())
    );
    let body: Value = serde_json::from_str(&seen[1].body).unwrap();
    assert_eq!(
        body,
        json!({"contract":{"operationId":READ,"operationVersion":"1.0.0",
            "contractDigest":read["contractDigest"]},
            "arguments":{"shopCode":"2021003","itemCode":"101010009"}})
    );
    // Extra or mistyped downstream fields are never passed through.
    iam.with(|f| {
        f.items = json!([{"shopCode":"2021003","itemCode":"101010009","itemName":"x",
            "minOrderNum":1,"minSafeStock":"9","creator":"someone"}])
    });
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
    assert_failure(&value, exit, 5, "UPSTREAM_CONTRACT_MISMATCH");
    assert!(value["data"].is_null());
    assert!(!value.to_string().contains("minSafeStock"));
    // A response for another contract than the one exchanged is a contract change.
    iam.with(|f| {
        f.items = items();
        f.overrides.insert(
            "invoke",
            reply(
                200,
                json!({"schemaVersion":"1.0","operationId":READ,"ok":true,"profile":null,
                    "data":items(),"error":null,"meta":{"contractVersion":"1.0.0",
                        "contractDigest":"0".repeat(64),"state":"succeeded"}}),
            ),
        );
    });
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
    assert_failure(&value, exit, 6, "CONTRACT_CHANGED");
}

// A2: the server refuses stale capabilities at exchange; no business invoke follows.
#[tokio::test]
async fn changed_or_withdrawn_contracts_stop_at_exchange() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let changed_digest = operation(READ, "1.0.0", true);
    let mut changed_digest = changed_digest.clone();
    changed_digest["contractDigest"] = json!(digest("route changed"));
    let mut newer = operation(READ, "1.1.0", true);
    newer["contractDigest"] = json!(digest("1.1.0"));
    let mut unconsented = operation(READ, "1.0.0", false);
    unconsented["consented"] = json!(false);
    for (live, code, exit) in [
        (vec![changed_digest], "CONTRACT_CHANGED", 6),
        (vec![newer], "SCOPE_DENIED", 4),
        (vec![operation(BATCH, "1.0.0", true)], "SCOPE_DENIED", 4),
        (vec![unconsented], "SCOPE_DENIED", 4),
    ] {
        iam.with(|f| f.operations = live);
        let start = iam.count();
        let (value, actual) = h
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
        assert_failure(&value, actual, exit, code);
        assert_eq!(
            iam.paths_since(start),
            vec!["POST /cli-api/oauth2/exchange"]
        );
    }
    // After a sync against the republished contract, the new digest is exchanged.
    let mut republished = operation(READ, "1.1.0", true);
    republished["contractDigest"] = json!(digest("republished"));
    iam.with(|f| f.operations = vec![republished.clone()]);
    let (value, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_eq!(exit, 0, "{value}");
    let start = iam.count();
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
    assert_eq!(exit, 0, "{value}");
    let exchange = iam.since(start)[0].form();
    assert_eq!(
        serde_json::from_str::<Value>(&exchange["operation_contracts"]).unwrap(),
        json!([key(&republished)])
    );
}

// A2 兼容 + A3 读写：未同意、写入、不兼容版本与多版本均零请求拒绝。
#[tokio::test]
async fn unconsented_write_and_incompatible_operations_never_execute() {
    let mut write = operation("supply-chain-server.demo.write", "1.0.0", true);
    write["effect"] = json!("write");
    let mut future = operation("supply-chain-server.demo.future", "1.0.0", true);
    future["contractSchemaVersion"] = json!(2);
    future["inputSchema"] = json!({"$ref":"https://schemas.invalid/next"});
    let mut newer = operation("supply-chain-server.demo.newer", "1.0.0", true);
    newer["minimumCliVersion"] = json!("99.0.0");
    let mut operations = supply_operations();
    operations.extend([write, future, newer]);
    let iam = Iam::start(SYSTEM, ENV, operations);
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (found, exit) = h
        .run(&["discover", "--profile", "supply", "--query", "demo"])
        .await;
    assert_eq!(exit, 0, "{found}");
    let compatible: BTreeMap<String, Value> = found["data"]["operations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| {
            (
                op["operationId"].as_str().unwrap().to_owned(),
                op["cliCompatible"].clone(),
            )
        })
        .collect();
    assert_eq!(compatible["supply-chain-server.demo.write"], true);
    assert_eq!(compatible["supply-chain-server.demo.future"], false);
    assert_eq!(compatible["supply-chain-server.demo.newer"], false);
    let before = iam.count();
    for (args, exit, code) in [
        (vec!["api", "call", SHOPS], 4, "SCOPE_DENIED"),
        (
            vec!["api", "call", "supply-chain-server.demo.write"],
            4,
            "SCOPE_DENIED",
        ),
        (
            vec!["api", "call", "supply-chain-server.demo.future"],
            6,
            "CLIENT_UPGRADE_REQUIRED",
        ),
        (
            vec!["jobs", "submit", "supply-chain-server.demo.newer"],
            6,
            "CLIENT_UPGRADE_REQUIRED",
        ),
        (
            vec!["api", "call", "supply-chain-server.unknown"],
            6,
            "UNKNOWN_OPERATION",
        ),
    ] {
        let mut full = args.clone();
        full.extend(["--profile", "supply", "--params", READ_PARAMS]);
        let (value, actual) = h.run(&full).await;
        assert_failure(&value, actual, exit, code);
    }
    assert_eq!(iam.count(), before, "rejected locally with zero requests");
    // Two versions of one operation need an explicit --version.
    iam.with(|f| {
        let mut second = operation(READ, "1.1.0", true);
        second["contractDigest"] = json!(digest("second"));
        f.operations = vec![operation(READ, "1.0.0", true), second];
    });
    let (value, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_eq!(exit, 0, "{value}");
    let before = iam.count();
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
    assert_failure(&value, exit, 2, "VERSION_REQUIRED");
    assert_eq!(iam.count(), before);
    let (value, exit) = h
        .run(&[
            "api",
            "call",
            READ,
            "--profile",
            "supply",
            "--version",
            "1.0.0",
            "--params",
            READ_PARAMS,
        ])
        .await;
    assert_eq!(exit, 0, "{value}");
    // A catalog exposing internal routing is rejected and the cached catalog is kept.
    iam.with(|f| {
        let mut leaking = operation(READ, "1.0.0", true);
        leaking["target"] =
            json!({"host":"supply.internal.invalid","path":"/deliveryCenters/current/shopItems"});
        f.operations = vec![leaking];
    });
    let (value, exit) = h.run(&["catalog", "sync", "--profile", "supply"]).await;
    assert_failure(&value, exit, 2, "CATALOG_INVALID");
    let (found, exit) = h.run(&["discover", "--profile", "supply"]).await;
    assert_eq!(exit, 0, "{found}");
    assert_eq!(found["data"]["operations"].as_array().unwrap().len(), 2);
}

// §4.1 错误映射：422/404/429/409/403/502/503 固定码，业务原文不外泄。
#[tokio::test]
async fn governed_error_mappings_are_fixed_and_sanitized() {
    let iam = supply();
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let rejected = json!({"schemaVersion":"1.0","operationId":READ,"ok":false,"profile":null,
        "data":null,"error":{"code":"BUSINESS_REJECTED","category":"business",
            "message":"当前配送中心不等于门店关联的配送中心","retryable":false,"hint":"x",
            "downstreamCode":"BUSINESS_EXCEPTION"},"meta":{"traceId":"trace-422"}});
    let cases: Vec<(&'static str, Reply, u8, &str)> = vec![
        ("invoke", reply(422, rejected), 6, "BUSINESS_REJECTED"),
        (
            "invoke",
            failure(404, "RESOURCE_UNAVAILABLE"),
            6,
            "RESOURCE_UNAVAILABLE",
        ),
        (
            "invoke",
            (
                429,
                vec![("Retry-After".into(), "7".into())],
                serde_json::to_vec(&json!({"schemaVersion":"1.0","ok":false,
                    "error":{"code":"RATE_LIMITED"},"meta":{"traceId":"trace-429"}}))
                .unwrap(),
            ),
            5,
            "RATE_LIMITED",
        ),
        (
            "invoke",
            failure(409, "CONTRACT_CHANGED"),
            6,
            "CONTRACT_CHANGED",
        ),
        (
            "invoke",
            failure(403, "DATA_SCOPE_DENIED"),
            4,
            "DATA_SCOPE_DENIED",
        ),
        (
            "invoke",
            failure(502, "UPSTREAM_CONTRACT_MISMATCH"),
            5,
            "UPSTREAM_CONTRACT_MISMATCH",
        ),
        (
            "invoke",
            (500, Vec::new(), b"<html>internal</html>".to_vec()),
            5,
            "NETWORK_ERROR",
        ),
        (
            "exchange",
            oauth_error(409, "invalid_target", "CONTRACT_CHANGED"),
            6,
            "CONTRACT_CHANGED",
        ),
        (
            "exchange",
            oauth_error(503, "temporarily_unavailable", "DEPENDENCY_UNAVAILABLE"),
            5,
            "DEPENDENCY_UNAVAILABLE",
        ),
        (
            "exchange",
            oauth_error(403, "invalid_scope", "SCOPE_DENIED"),
            4,
            "SCOPE_DENIED",
        ),
    ];
    for (label, response, exit, code) in cases {
        iam.with(|f| {
            f.overrides.clear();
            f.overrides.insert(label, response);
        });
        let start = iam.count();
        let (value, actual) = h
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
        assert_failure(&value, actual, exit, code);
        assert!(value["data"].is_null(), "{value}");
        let text = value.to_string();
        assert!(!text.contains("当前配送中心"), "{text}");
        assert!(!text.contains("<html>"), "{text}");
        let paths = iam.paths_since(start);
        match label {
            "catalog" => assert_eq!(paths, vec!["GET /cli-api/v1/catalog"]),
            "exchange" => assert_eq!(paths.len(), 1, "no invoke after a failed exchange"),
            _ if code == "RATE_LIMITED" => {
                assert_eq!(paths.len(), 6, "three bounded read attempts")
            }
            _ => assert_eq!(paths.len(), 2),
        }
        match code {
            "CONTRACT_CHANGED" => {
                assert_eq!(value["meta"]["actions"][0]["id"], "sync-contract");
            }
            "DATA_SCOPE_DENIED" | "SCOPE_DENIED" => {
                assert_eq!(value["meta"]["actions"][0]["actor"], "admin");
                assert_eq!(value["meta"]["actions"][0]["kind"], "manual");
            }
            "BUSINESS_REJECTED" => {
                assert_eq!(value["meta"]["downstreamCode"], "BUSINESS_EXCEPTION");
                assert_eq!(value["meta"]["traceId"], "trace-422");
            }
            "RATE_LIMITED" => {
                assert_eq!(value["error"]["retryable"], true);
                assert_eq!(value["meta"]["retryAfterSeconds"], 7);
            }
            _ => {}
        }
    }
}

// A6 批量（CLI 侧）：提交/状态/取结果/取消；未完成绝不当完整结果返回。
