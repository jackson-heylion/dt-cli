use super::*;

const DIRECT: &str = "supply-chain-server.fixture.direct";
const BUSINESS_TOKEN: &str = "synthetic-business-token";

fn operation(origin: &str, name: &str) -> Value {
    let mut op = supply_operations()[0].clone();
    op["operationId"] = json!(name);
    op["inputSchema"] = json!({"type":"object","additionalProperties":false,"required":["brandId"],
        "properties":{"brandId":{"type":"integer","minimum":1}}});
    op["outputSchema"] = json!({"type":"array","items":{"type":"object","additionalProperties":false,
        "required":["value","brandId"],"properties":{"value":{"type":"integer"},"brandId":{"type":"integer"}}}});
    op["direct"] = json!({"schemaVersion":1,"baseUri":origin,"method":"POST","path":"/fixture-business/read",
        "limits":{"timeoutSeconds":10,"pageBytes":4096,"maxPages":4,"maxRecords":20,"maxResultBytes":8192,"maxSeconds":60,"concurrency":1},
        "execution":{"engine":"direct-read","schemaVersion":1,"identity":{"mode":"supply-chain.brand","contextParameter":"brandId"},
            "scope":{"kind":"brand"},"request":{"path":{},"query":{},"body":{
                "brandId":{"source":"trusted","value":"brandId","required":true},
                "pageNum":{"source":"page","value":"pageNum","required":true}}},
            "response":{"httpStatus":200,"itemsPointer":"/items","projection":{"value":"/value",
                "brandId":{"source":"trusted","value":"brandId"}}},
            "pagination":{"mode":"page","pageSize":10,"pagesPointer":"/pages","pageNumPointer":"/pageNum"}}});
    op
}

fn native_reply(subject: &str) -> Reply {
    reply(
        200,
        json!({"token":format!("Bearer {BUSINESS_TOKEN}"),
        "expiresAt":(chrono::Utc::now()+chrono::Duration::seconds(60)).to_rfc3339(),
        "subjectId":subject,"trusted":{"subjectId":1,"brandId":7,"authorizedShopCodes":["shop-a"]}}),
    )
}

fn view_option(kind: &str, id: i32, object: &str, supported: bool) -> Value {
    let data_type = match kind {
        "GROUP" => 3,
        "SHOP" => 4,
        "MANUFACTURE" => 6,
        "DELIVERY" => 7,
        "SUPPLIER" => 8,
        "BRAND" => 9,
        "FRANCHISE" => 2,
        _ => panic!("unknown fixture view"),
    };
    json!({"viewId":id,"viewType":kind,"dataType":data_type,"viewName":kind,"objectId":object,
        "objectName":format!("本人{kind}对象"),"objectType":kind.to_lowercase(),"shopCode":null,
        "brandId":null,"parentId":null,"supported":supported,"unsupportedReason":if supported {Value::Null}else{json!("目标业务系统未提供该视角")}})
}

#[tokio::test]
async fn all_views_are_listed_and_explicit_task_selection_does_not_become_a_profile_default() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":42}],"pages":1,"pageNum":1})),
        );
    });
    let mut op = operation(&business.origin, DIRECT);
    op["inputSchema"] = json!({"type":"object","additionalProperties":false,"required":["view"],"properties":{
        "view":{"type":"object","additionalProperties":false,"required":["viewId","objectId"],"properties":{
            "viewId":{"type":"integer","minimum":1},"objectId":{"type":"string","minLength":1}}}}});
    op["direct"]["execution"]["identity"] =
        json!({"mode":"supply-chain.view","contextParameter":"view"});
    op["direct"]["execution"]["scope"] = json!({"kind":"native"});
    op["direct"]["execution"]["request"]["body"] =
        json!({"pageNum":{"source":"page","value":"pageNum","required":true}});
    let iam = Iam::start(SYSTEM, ENV, vec![op]);
    let options = vec![
        view_option("GROUP", 3, "1", true),
        view_option("SHOP", 4, "shop-a", true),
        view_option("MANUFACTURE", 6, "factory-a", false),
        view_option("DELIVERY", 7, "17", true),
        view_option("SUPPLIER", 8, "21", true),
        view_option("BRAND", 9, "7", true),
        view_option("FRANCHISE", 2, "7", false),
    ];
    iam.with(|f| {
        f.overrides.insert(
            "views",
            reply(
                200,
                ok(
                    "views.list",
                    json!({"systemId":SYSTEM,"environment":ENV,"subjectId":"1","options":options}),
                ),
            ),
        );
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (v, c) = h.run(&["views", "list", "--profile", "supply"]).await;
    assert_eq!(c, 0, "{v}");
    assert_eq!(v["data"]["options"].as_array().unwrap().len(), 7);
    for option in options.iter().filter(|v| v["supported"] == true) {
        let selection =
            json!({"viewId":option["viewId"],"objectId":option["objectId"]}).to_string();
        let params = json!({"view":serde_json::from_str::<Value>(&selection).unwrap()}).to_string();
        let mut credential: Value = serde_json::from_slice(&native_reply("1").2).unwrap();
        credential["trusted"]["view"] = option.clone();
        iam.with(|f| {
            f.overrides
                .insert("direct.credential", reply(200, credential));
        });
        let (v, c) = h
            .run(&[
                "api",
                "call",
                DIRECT,
                "--profile",
                "supply",
                "--params",
                &params,
            ])
            .await;
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["taskView"], *option);
        let (submitted, c) = h
            .run(&[
                "jobs",
                "submit",
                DIRECT,
                "--profile",
                "supply",
                "--params",
                &params,
            ])
            .await;
        assert_eq!(c, 0, "{submitted}");
        let job = submitted["data"]["jobId"].as_str().unwrap();
        let (done, c) = h.run(&["jobs", "wait", job, "--profile", "supply"]).await;
        assert_eq!(c, 0, "{done}");
        let (stored, c) = h.run(&["jobs", "status", job, "--profile", "supply"]).await;
        assert_eq!(c, 0, "{stored}");
        assert_eq!(stored["data"]["taskView"], *option);
    }
    let reads = business.count();
    let (v, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            "{}",
        ])
        .await;
    assert_failure(&v, c, 2, "VIEW_SELECTION_REQUIRED");
    assert_eq!(business.count(), reads);
    let unsupported = &options[2];
    let params =
        json!({"view":{"viewId":unsupported["viewId"],"objectId":unsupported["objectId"]}})
            .to_string();
    iam.with(|f| {
        f.overrides.insert(
            "direct.credential",
            reply(422, json!({"code":"VIEW_UNSUPPORTED"})),
        );
    });
    let (v, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            &params,
        ])
        .await;
    assert_failure(&v, c, 6, "VIEW_UNSUPPORTED");
    assert_eq!(business.count(), reads);
    // Alternatives must be supported and still belong to this employee.
    assert!(!v.to_string().contains("factory-a"));
}

#[tokio::test]
async fn stopped_runner_is_reported_paused_and_restored_scope_must_match_original() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":11}],"pages":2,"pageNum":1})),
        );
        f.overrides
            .insert("direct.business.page2", (1, vec![], vec![]));
    });
    let iam = Iam::start(SYSTEM, ENV, vec![operation(&business.origin, DIRECT)]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (submitted, c) = h
        .run(&[
            "jobs",
            "submit",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_eq!(c, 0, "{submitted}");
    let job = submitted["data"]["jobId"].as_str().unwrap().to_owned();
    {
        let arguments = ["jobs", "wait", &job, "--profile", "supply"];
        let execution = h.run(&arguments);
        tokio::pin!(execution);
        tokio::select! {
            result=&mut execution => panic!("runner finished before interruption: {result:?}"),
            _=async {
                let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
                while business.count()<2 {
                    assert!(tokio::time::Instant::now()<deadline);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            } => {}
        }
        let (status, c) = h
            .run(&["jobs", "status", &job, "--profile", "supply"])
            .await;
        assert_eq!(c, 0, "{status}");
        assert_eq!(status["data"]["runnerActive"], true);
    } // Drop the command future, releasing its OS lock without changing the last checkpoint.
    let (status, c) = h
        .run(&["jobs", "status", &job, "--profile", "supply"])
        .await;
    assert_eq!(c, 0, "{status}");
    assert_eq!(status["data"]["state"], "paused");
    assert_eq!(status["data"]["runnerActive"], false);
    assert_eq!(status["data"]["pageCount"], 1);
    let mut changed: Value = serde_json::from_slice(&native_reply("1").2).unwrap();
    changed["trusted"]["authorizedShopCodes"] = json!(["shop-a", "new-shop"]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", reply(200, changed));
    });
    let reads = business.count();
    let (denied, c) = h.run(&["jobs", "wait", &job, "--profile", "supply"]).await;
    assert_failure(&denied, c, 4, "DATA_SCOPE_DENIED");
    assert_eq!(business.count(), reads);
}

#[tokio::test]
async fn arbitrary_published_operations_call_business_directly_and_do_not_echo_credentials() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":42}],"pages":1,"pageNum":1})),
        );
    });
    let iam = Iam::start(
        SYSTEM,
        ENV,
        vec![
            operation(&business.origin, DIRECT),
            operation(&business.origin, "supply-chain-server.another.direct"),
        ],
    );
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let start = iam.count();
    for id in [DIRECT, "supply-chain-server.another.direct"] {
        let (v, c) = h
            .run(&[
                "api",
                "call",
                id,
                "--profile",
                "supply",
                "--params",
                r#"{"brandId":7}"#,
            ])
            .await;
        assert_eq!(c, 0, "{v}");
        assert_eq!(v["data"]["items"], json!([{"value":42,"brandId":7}]));
        assert_eq!(v["data"]["executionLocation"], "local");
        assert!(!v.to_string().contains(BUSINESS_TOKEN));
    }
    let calls = business.since(0);
    assert_eq!(calls.len(), 2);
    for call in calls {
        assert_eq!(call.path, "/fixture-business/read");
        assert_eq!(
            call.authorization.as_deref(),
            Some("Bearer synthetic-business-token")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&call.body).unwrap(),
            json!({"brandId":7,"pageNum":1})
        );
    }
    let management = iam.since(start);
    assert_eq!(
        management
            .iter()
            .filter(|r| r.path.ends_with("/direct-credential"))
            .count(),
        2
    );
    assert!(
        !management
            .iter()
            .any(|r| r.path.ends_with("/invoke") || r.path == "/cli-api/v1/jobs")
    );
}

#[tokio::test]
async fn wrong_employee_credential_is_rejected_before_business_request() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    let iam = Iam::start(SYSTEM, ENV, vec![operation(&business.origin, DIRECT)]);
    iam.with(|f| {
        f.overrides
            .insert("direct.credential", native_reply("other-employee"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (v, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_ne!(c, 0);
    assert_eq!(business.count(), 0);
    assert!(!v.to_string().contains(BUSINESS_TOKEN));
}

#[tokio::test]
async fn credential_origin_and_arguments_cannot_be_supplied_by_employee() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    let iam = Iam::start(SYSTEM, ENV, vec![operation(&business.origin, DIRECT)]);
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let start = iam.count();
    let (v, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7,"url":"https://unregistered.example","token":"supplied"}"#,
        ])
        .await;
    assert_failure(&v, c, 2, "INVALID_ARGUMENT");
    assert_eq!(business.count(), 0);
    assert!(
        !iam.since(start)
            .iter()
            .any(|r| r.path.ends_with("/direct-credential"))
    );
}

#[tokio::test]
async fn local_job_resumes_saved_pages_and_rechecks_permission_before_returning_saved_result() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":11}],"pages":2,"pageNum":1})),
        );
        f.overrides.insert(
            "direct.business.page2",
            reply(503, json!({"message":"temporarily unavailable"})),
        );
    });
    let iam = Iam::start(SYSTEM, ENV, vec![operation(&business.origin, DIRECT)]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let start = iam.count();
    let (submitted, c) = h
        .run(&[
            "jobs",
            "submit",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_eq!(c, 0, "{submitted}");
    let job = submitted["data"]["jobId"].as_str().unwrap().to_owned();
    assert!(job.starts_with("local_"));
    assert_eq!(business.count(), 0);
    let (paused, c) = h.run(&["jobs", "wait", &job, "--profile", "supply"]).await;
    assert_failure(&paused, c, 5, "DEPENDENCY_UNAVAILABLE");
    let (status, c) = h
        .run(&["jobs", "status", &job, "--profile", "supply"])
        .await;
    assert_eq!(c, 0, "{status}");
    assert_eq!(status["data"]["state"], "paused");
    assert_eq!(status["data"]["pageCount"], 1);
    assert_eq!(status["data"]["itemCount"], 1);
    let (partial, c) = h
        .run(&["jobs", "result", &job, "--profile", "supply"])
        .await;
    assert_failure(&partial, c, 7, "PARTIAL_RESULT");
    business.with(|f| {
        f.overrides.insert(
            "direct.business.page2",
            reply(200, json!({"items":[{"value":22}],"pages":2,"pageNum":2})),
        );
    });
    let (done, c) = h.run(&["jobs", "wait", &job, "--profile", "supply"]).await;
    assert_eq!(c, 0, "{done}");
    assert_eq!(
        done["data"]["items"],
        json!([{"value":11,"brandId":7},{"value":22,"brandId":7}])
    );
    let pages: Vec<_> = business
        .seen()
        .iter()
        .map(|r| serde_json::from_str::<Value>(&r.body).unwrap()["pageNum"].clone())
        .collect();
    assert_eq!(pages, vec![json!(1), json!(2), json!(2)]);
    let reads = business.count();
    let (saved, c) = h
        .run(&["jobs", "result", &job, "--profile", "supply"])
        .await;
    assert_eq!(c, 0, "{saved}");
    assert_eq!(saved["meta"]["asOf"], done["meta"]["asOf"]);
    assert!(done["meta"]["asOf"].is_string());
    assert!(done["meta"]["readStartedAt"].is_string());
    assert_eq!(done["meta"]["snapshotConsistent"], false);
    assert_eq!(business.count(), reads);
    iam.with(|f| {
        f.overrides.insert(
            "direct.credential",
            reply(403, json!({"code":"DATA_SCOPE_DENIED"})),
        );
    });
    let (denied, c) = h
        .run(&["jobs", "result", &job, "--profile", "supply"])
        .await;
    assert_ne!(c, 0, "{denied}");
    assert!(denied["data"]["items"].is_null());
    assert_eq!(business.count(), reads);
    assert!(
        !iam.since(start)
            .iter()
            .any(|r| r.path.starts_with("/cli-api/v1/jobs") || r.path.ends_with("/invoke"))
    );
    let checkpoint = std::fs::read_to_string(h.file(&format!("local-jobs/{job}.json"))).unwrap();
    assert!(!checkpoint.contains(BUSINESS_TOKEN));
}

#[tokio::test]
async fn altered_local_checkpoint_stops_before_permission_or_business_requests() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    let iam = Iam::start(SYSTEM, ENV, vec![operation(&business.origin, DIRECT)]);
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (v, c) = h
        .run(&[
            "jobs",
            "submit",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_eq!(c, 0, "{v}");
    let job = v["data"]["jobId"].as_str().unwrap();
    let path = h.file(&format!("local-jobs/{job}.json"));
    let mut record: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    record["record"]["params"]["brandId"] = json!(99);
    std::fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
    let start = iam.count();
    let (failure, c) = h.run(&["jobs", "wait", job, "--profile", "supply"]).await;
    assert_failure(&failure, c, 1, "CREDENTIAL_STORE_UNAVAILABLE");
    assert_eq!(business.count(), 0);
    assert_eq!(iam.count(), start);
}

#[tokio::test]
#[ignore = "requires the explicitly built isolated-cli acceptance executable"]
async fn killed_cli_process_preserves_completed_page_and_next_process_resumes_it() {
    use dt_cli::credentials::FileStore;
    let executable =
        std::env::var_os("DT_CLI_ISOLATED_BIN").expect("set the built isolated-cli path");
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":11}],"pages":2,"pageNum":1})),
        );
        f.overrides
            .insert("direct.business.page2", (1, vec![], vec![]));
    });
    let iam = Iam::start(SYSTEM, ENV, vec![operation(&business.origin, DIRECT)]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (submitted, c) = h
        .run(&[
            "jobs",
            "submit",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_eq!(c, 0, "{submitted}");
    let job = submitted["data"]["jobId"].as_str().unwrap().to_owned();
    // The child uses the production private file store, with only this fixture's own credentials.
    let file_store = FileStore::new(h.root.path());
    for (key, text) in h.store.0.lock().unwrap().iter() {
        file_store
            .write(key, &serde_json::from_str::<Credentials>(text).unwrap())
            .unwrap();
    }
    for (key, bytes) in h.store.1.lock().unwrap().iter() {
        file_store.write_integrity_key(key, bytes).unwrap();
    }
    let fixture = h.root.path().join("fixture.json");
    std::fs::write(
        &fixture,
        serde_json::to_vec(
            &json!({"origin":iam.origin,"environment":ENV,"root":h.root.path(),
        "browser_request":h.root.path().join("browser.txt")}),
        )
        .unwrap(),
    )
    .unwrap();
    let start = iam.count();
    let mut child = std::process::Command::new(&executable)
        .env("DT_CLI_ISOLATED_FIXTURE", &fixture)
        .args(["jobs", "wait", &job, "--profile", "supply"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while business.count() < 2 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "child failed to reach second page"
        );
        assert!(
            child.try_wait().unwrap().is_none(),
            "child exited before interruption"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    child.kill().unwrap();
    child.wait().unwrap();
    let (status, c) = h
        .run(&["jobs", "status", &job, "--profile", "supply"])
        .await;
    assert_eq!(c, 0, "{status}");
    assert_eq!(status["data"]["state"], "paused");
    assert_eq!(status["data"]["pageCount"], 1);
    business.with(|f| {
        f.overrides.insert(
            "direct.business.page2",
            reply(200, json!({"items":[{"value":22}],"pages":2,"pageNum":2})),
        );
    });
    let completed = std::process::Command::new(&executable)
        .env("DT_CLI_ISOLATED_FIXTURE", &fixture)
        .args(["jobs", "wait", &job, "--profile", "supply"])
        .output()
        .unwrap();
    let output: Value = serde_json::from_slice(&completed.stdout).unwrap();
    assert!(completed.status.success(), "{output}");
    assert_eq!(
        output["data"]["items"],
        json!([{"value":11,"brandId":7},{"value":22,"brandId":7}])
    );
    let pages: Vec<_> = business
        .seen()
        .iter()
        .map(|r| serde_json::from_str::<Value>(&r.body).unwrap()["pageNum"].clone())
        .collect();
    assert_eq!(pages, vec![json!(1), json!(2), json!(2)]);
    assert!(
        !iam.since(start)
            .iter()
            .any(|r| r.path.starts_with("/cli-api/v1/jobs"))
    );
    assert!(!String::from_utf8_lossy(&completed.stdout).contains(BUSINESS_TOKEN));
    assert!(!String::from_utf8_lossy(&completed.stderr).contains(BUSINESS_TOKEN));
}

#[tokio::test]
async fn malformed_pages_and_changed_totals_never_commit_the_invalid_page() {
    for (first, second) in [
        (
            json!({"items":[{"value":11}],"pages":2,"pageNum":1,"total":2}),
            json!({"items":[{"value":22}],"pages":2,"pageNum":2,"total":3}),
        ),
        (
            json!({"items":[{"value":11}],"pages":2,"pageNum":1,"total":2}),
            json!({"items":[{"value":"invalid number"}],"pages":2,"pageNum":2,"total":2}),
        ),
        (
            json!({"items":[{"value":11}],"pages":2,"pageNum":1,"total":2}),
            json!({"items":[{"value":22}],"pages":2,"pageNum":1,"total":2}),
        ),
    ] {
        let business = Iam::start(SYSTEM, ENV, vec![]);
        business.with(|f| {
            f.overrides.insert("direct.business", reply(200, first));
            f.overrides
                .insert("direct.business.page2", reply(200, second));
        });
        let mut op = operation(&business.origin, DIRECT);
        op["direct"]["execution"]["pagination"]["totalPointer"] = json!("/total");
        let iam = Iam::start(SYSTEM, ENV, vec![op]);
        iam.with(|f| {
            f.overrides.insert("direct.credential", native_reply("1"));
        });
        let h = Harness::new(&[(ENV, &iam)]);
        h.login("supply", ENV, SYSTEM).await;
        let (submitted, c) = h
            .run(&[
                "jobs",
                "submit",
                DIRECT,
                "--profile",
                "supply",
                "--params",
                r#"{"brandId":7}"#,
            ])
            .await;
        assert_eq!(c, 0, "{submitted}");
        let job = submitted["data"]["jobId"].as_str().unwrap();
        let (failed, c) = h.run(&["jobs", "wait", job, "--profile", "supply"]).await;
        assert_failure(&failed, c, 5, "UPSTREAM_CONTRACT_MISMATCH");
        let (status, c) = h.run(&["jobs", "status", job, "--profile", "supply"]).await;
        assert_eq!(c, 0, "{status}");
        assert_eq!(status["data"]["pageCount"], 1);
        assert_eq!(status["data"]["itemCount"], 1);
        assert_eq!(status["data"]["complete"], false);
    }
}

#[tokio::test]
async fn unique_brand_is_inferred_from_live_candidates_and_ambiguity_never_chooses_first() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":42}],"pages":1,"pageNum":1})),
        );
    });
    let iam = Iam::start(SYSTEM, ENV, vec![operation(&business.origin, DIRECT)]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
        f.overrides.insert(
            "views",
            reply(
                200,
                ok(
                    "views.list",
                    json!({"systemId":SYSTEM,"environment":ENV,"subjectId":"1",
            "options":[view_option("BRAND",12,"7",true)]}),
                ),
            ),
        );
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (result, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            "{}",
        ])
        .await;
    assert_eq!(c, 0, "{result}");
    assert_eq!(
        serde_json::from_str::<Value>(&business.seen()[0].body).unwrap()["brandId"],
        7
    );
    iam.with(|f| {
        f.overrides.insert(
            "views",
            reply(
                200,
                ok(
                    "views.list",
                    json!({"systemId":SYSTEM,"environment":ENV,"subjectId":"1",
        "options":[view_option("BRAND",12,"7",true),view_option("BRAND",12,"8",true)]}),
                ),
            ),
        );
    });
    let reads = business.count();
    let (ambiguous, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            "{}",
        ])
        .await;
    assert_failure(&ambiguous, c, 2, "VIEW_SELECTION_REQUIRED");
    assert_eq!(business.count(), reads);
}

fn evidence() -> Value {
    json!({"sourceCommit":"bc5a3574f17bd77bbedd851c3c171892a79889b0","sourcePath":"fixture/ReadController.java","notes":"明确登记的只读核对来源"})
}

#[tokio::test]
async fn business_shop_codes_resolve_tree_ids_and_only_own_branches_are_returned() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f|{
        f.overrides.insert("direct.shops",reply(200,json!({"pageNum":1,"pages":1,"total":2,"list":[
            {"id":501,"code":"shop-a"},{"id":502,"code":"outside"}]})));
        f.overrides.insert("direct.business",reply(200,json!({"items":[{"id":1,"name":"区域","children":[
            {"id":2,"name":"其他区域","children":[],"shops":[{"id":502,"name":"无权门店","secret":"hidden"}]}],
            "shops":[{"id":501,"name":"本人门店","secret":"hidden"},{"id":502,"name":"无权门店"}],"secret":"hidden"}]})));
    });
    let mut op = operation(&business.origin, DIRECT);
    op["outputSchema"] = json!({"type":"array","items":{"type":"object","additionalProperties":false,"required":["id","name","children","shops"],
        "properties":{"id":{"type":"integer"},"name":{"type":"string"},"children":{"type":"array"},"shops":{"type":"array"}}}});
    let execution = &mut op["direct"]["execution"];
    execution["scope"] = json!({"kind":"authorized-shop-tree","tree":{"childrenField":"children","shopsField":"shops","shopIdField":"id",
        "nodeFields":["id","name"],"shopFields":["id","name"]}});
    execution["preflight"] = json!({"shopDirectory":{"path":"/fixture-business/shops","pageSize":100,"maxPages":100,"readOnlyEvidence":evidence()}});
    execution["pagination"] = json!({"mode":"none"});
    execution["response"]["projection"] =
        json!({"id":"/id","name":"/name","children":"/children","shops":"/shops"});
    let iam = Iam::start(SYSTEM, ENV, vec![op]);
    let mut native: Value = serde_json::from_slice(&native_reply("1").2).unwrap();
    // An AUTH shop id belongs to another namespace and cannot identify the business store.
    native["trusted"]["authorizedShops"] = json!([{"shopId":"AUTH-A","shopCode":"shop-a"}]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", reply(200, native));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (result, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_eq!(c, 0, "{result}");
    assert_eq!(
        result["data"]["items"],
        json!([{"id":1,"name":"区域","children":[],"shops":[{"id":501,"name":"本人门店"}]}])
    );
    assert!(!result.to_string().contains("hidden"));
    assert!(!result.to_string().contains("无权门店"));
    let calls = business.seen();
    assert_eq!(calls.len(), 2);
    assert!(calls[0].path.starts_with("/fixture-business/shops?"));
    assert_eq!(calls[1].path, "/fixture-business/read");
    assert!(
        calls
            .iter()
            .all(|r| r.authorization.as_deref() == Some("Bearer synthetic-business-token"))
    );
}

#[tokio::test]
async fn warehouse_must_resolve_to_selected_own_shop_before_data_request() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.warehouse",
            reply(200, json!({"id":31,"type":"SHOP","ownerId":501})),
        );
        f.overrides
            .insert("direct.owner", reply(200, json!({"code":"shop-a"})));
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":42}],"pages":1,"pageNum":1})),
        );
    });
    let mut op = operation(&business.origin, DIRECT);
    op["inputSchema"]["properties"]["warehouseId"] = json!({"type":"integer","minimum":1});
    op["inputSchema"]["required"] = json!(["brandId", "warehouseId"]);
    op["direct"]["execution"]["scope"] =
        json!({"kind":"shops","bindSingleShop":true,"warehouseParameter":"warehouseId"});
    op["direct"]["execution"]["preflight"] = json!({"warehouse":{"path":"/fixture-business/warehouses/{id}","shopPath":"/fixture-business/shop/{id}",
        "readOnlyEvidence":evidence(),"shopReadOnlyEvidence":evidence()}});
    let iam = Iam::start(SYSTEM, ENV, vec![op]);
    let mut native: Value = serde_json::from_slice(&native_reply("1").2).unwrap();
    native["trusted"]["warehouseId"] = json!(31);
    native["trusted"]["selectedShopCodes"] = json!(["shop-a"]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", reply(200, native));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (result, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7,"warehouseId":31}"#,
        ])
        .await;
    assert_eq!(c, 0, "{result}");
    assert_eq!(
        business
            .since(0)
            .iter()
            .filter(|r| r.path == "/fixture-business/read")
            .count(),
        1
    );
    let reads = business.count();
    business.with(|f| {
        f.overrides
            .insert("direct.owner", reply(200, json!({"code":"outside"})));
    });
    let (denied, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7,"warehouseId":31}"#,
        ])
        .await;
    assert_failure(&denied, c, 4, "DATA_SCOPE_DENIED");
    assert!(
        !business
            .since(reads)
            .iter()
            .any(|r| r.path == "/fixture-business/read")
    );
}

#[tokio::test]
async fn expired_business_credential_reissues_once_in_original_context_and_never_changes_view() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.once_replies.insert(
            "direct.business",
            reply(401, json!({"code":"BUSINESS_TOKEN_EXPIRED"})),
        );
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":42}],"pages":1,"pageNum":1})),
        );
    });
    let iam = Iam::start(SYSTEM, ENV, vec![operation(&business.origin, DIRECT)]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let start = iam.count();
    let (result, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_eq!(c, 0, "{result}");
    assert_eq!(business.count(), 2);
    let requests: Vec<_> = iam
        .since(start)
        .into_iter()
        .filter(|r| r.path.ends_with("/direct-credential"))
        .collect();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);
    // A second refusal is terminal, rather than an unbounded refresh or an alternate view.
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(401, json!({"code":"BUSINESS_TOKEN_EXPIRED"})),
        );
    });
    let reads = business.count();
    let (denied, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_failure(&denied, c, 3, "BUSINESS_CREDENTIAL_REJECTED");
    assert_eq!(business.count() - reads, 2);
}

#[tokio::test]
async fn published_body_pointers_create_nested_json_and_preserve_array_wrapping() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":42}],"pages":1,"pageNum":1})),
        );
    });
    let mut op = operation(&business.origin, DIRECT);
    op["direct"]["execution"]["request"]["body"] = json!({
        "/filters/brandId":{"source":"trusted","value":"brandId","required":true},
        "/filters/warehouseIds":{"source":"fixed","value":31,"wrapArray":true},
        "/pageNum":{"source":"page","value":"pageNum","required":true}});
    let iam = Iam::start(SYSTEM, ENV, vec![op]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (result, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_eq!(c, 0, "{result}");
    assert_eq!(
        serde_json::from_str::<Value>(&business.seen()[0].body).unwrap(),
        json!({"filters":{"brandId":7,"warehouseIds":[31]},"pageNum":1})
    );
}

#[tokio::test]
async fn repeated_cursor_preserves_previous_page_and_resume_uses_the_original_cursor() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(
                200,
                json!({"items":[{"value":11}],"nextCursor":"original-cursor"}),
            ),
        );
        f.overrides.insert(
            "direct.business.page2",
            reply(
                200,
                json!({"items":[{"value":22}],"nextCursor":"original-cursor"}),
            ),
        );
    });
    let mut op = operation(&business.origin, DIRECT);
    op["direct"]["execution"]["pagination"] =
        json!({"mode":"cursor","pageSize":10,"nextCursorPointer":"/nextCursor"});
    op["direct"]["execution"]["request"]["body"]["cursor"] =
        json!({"source":"page","value":"cursor"});
    let iam = Iam::start(SYSTEM, ENV, vec![op]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (submitted, c) = h
        .run(&[
            "jobs",
            "submit",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_eq!(c, 0, "{submitted}");
    let job = submitted["data"]["jobId"].as_str().unwrap();
    let (failed, c) = h.run(&["jobs", "wait", job, "--profile", "supply"]).await;
    assert_failure(&failed, c, 5, "UPSTREAM_CONTRACT_MISMATCH");
    let (status, c) = h.run(&["jobs", "status", job, "--profile", "supply"]).await;
    assert_eq!(c, 0, "{status}");
    assert_eq!(status["data"]["pageCount"], 1);
    business.with(|f| {
        f.overrides.insert(
            "direct.business.page2",
            reply(200, json!({"items":[{"value":22}],"nextCursor":null})),
        );
    });
    let (completed, c) = h.run(&["jobs", "wait", job, "--profile", "supply"]).await;
    assert_eq!(c, 0, "{completed}");
    assert_eq!(completed["data"]["items"].as_array().unwrap().len(), 2);
    let calls = business.seen();
    assert_eq!(calls.len(), 3);
    assert!(
        serde_json::from_str::<Value>(&calls[0].body)
            .unwrap()
            .get("cursor")
            .is_none()
    );
    for call in &calls[1..] {
        assert_eq!(
            serde_json::from_str::<Value>(&call.body).unwrap()["cursor"],
            "original-cursor"
        );
    }
}

#[tokio::test]
async fn concurrent_waiters_share_one_job_progress_and_separate_jobs_do_not_overwrite_each_other() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":42}],"pages":1,"pageNum":1})),
        );
    });
    let iam = Iam::start(SYSTEM, ENV, vec![operation(&business.origin, DIRECT)]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (submitted, c) = h
        .run(&[
            "jobs",
            "submit",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_eq!(c, 0, "{submitted}");
    let job = submitted["data"]["jobId"].as_str().unwrap();
    let args = ["jobs", "wait", job, "--profile", "supply"];
    let (first, second) = tokio::join!(h.run(&args), h.run(&args));
    assert_eq!(first.1, 0, "{}", first.0);
    assert_eq!(second.1, 0, "{}", second.0);
    assert_eq!(business.count(), 1);
    assert_eq!(first.0["data"]["items"], second.0["data"]["items"]);
    let (other, c) = h
        .run(&[
            "jobs",
            "submit",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_eq!(c, 0, "{other}");
    let other_id = other["data"]["jobId"].as_str().unwrap();
    assert_ne!(job, other_id);
    let (finished, c) = h
        .run(&["jobs", "wait", other_id, "--profile", "supply"])
        .await;
    assert_eq!(c, 0, "{finished}");
    assert_eq!(business.count(), 2);
    for id in [job, other_id] {
        let (status, c) = h.run(&["jobs", "status", id, "--profile", "supply"]).await;
        assert_eq!(c, 0, "{status}");
        assert_eq!(status["data"]["jobId"], id);
        assert_eq!(status["data"]["itemCount"], 1);
    }
}

#[tokio::test]
async fn record_and_byte_limits_keep_completed_pages_and_total_time_bounds_a_slow_request() {
    for (key, value) in [("maxRecords", json!(1)), ("maxResultBytes", json!(35))] {
        let business = Iam::start(SYSTEM, ENV, vec![]);
        business.with(|f| {
            f.overrides.insert(
                "direct.business",
                reply(200, json!({"items":[{"value":11}],"pages":2,"pageNum":1})),
            );
            f.overrides.insert(
                "direct.business.page2",
                reply(200, json!({"items":[{"value":22}],"pages":2,"pageNum":2})),
            );
        });
        let mut op = operation(&business.origin, DIRECT);
        op["direct"]["limits"][key] = value;
        let iam = Iam::start(SYSTEM, ENV, vec![op]);
        iam.with(|f| {
            f.overrides.insert("direct.credential", native_reply("1"));
        });
        let h = Harness::new(&[(ENV, &iam)]);
        h.login("supply", ENV, SYSTEM).await;
        let (submitted, c) = h
            .run(&[
                "jobs",
                "submit",
                DIRECT,
                "--profile",
                "supply",
                "--params",
                r#"{"brandId":7}"#,
            ])
            .await;
        assert_eq!(c, 0, "{submitted}");
        let job = submitted["data"]["jobId"].as_str().unwrap();
        let (failed, c) = h.run(&["jobs", "wait", job, "--profile", "supply"]).await;
        assert_failure(&failed, c, 7, "RESOURCE_LIMIT");
        let (status, c) = h.run(&["jobs", "status", job, "--profile", "supply"]).await;
        assert_eq!(c, 0, "{status}");
        assert_eq!(status["data"]["pageCount"], 1);
        assert_eq!(status["data"]["complete"], false);
    }
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert("direct.business", (1, vec![], vec![]));
    });
    let mut op = operation(&business.origin, DIRECT);
    op["direct"]["limits"]["maxSeconds"] = json!(1);
    let iam = Iam::start(SYSTEM, ENV, vec![op]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let started = tokio::time::Instant::now();
    let (failed, c) = h
        .run(&[
            "api",
            "call",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_failure(&failed, c, 7, "RESOURCE_LIMIT");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn retired_original_version_preserves_checkpoint_and_never_switches_to_new_version() {
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":11}],"pages":2,"pageNum":1})),
        );
        f.overrides.insert(
            "direct.business.page2",
            reply(503, json!({"code":"UNAVAILABLE"})),
        );
    });
    let iam = Iam::start(SYSTEM, ENV, vec![operation(&business.origin, DIRECT)]);
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let (submitted, c) = h
        .run(&[
            "jobs",
            "submit",
            DIRECT,
            "--profile",
            "supply",
            "--params",
            r#"{"brandId":7}"#,
        ])
        .await;
    assert_eq!(c, 0, "{submitted}");
    let job = submitted["data"]["jobId"].as_str().unwrap();
    let (failed, c) = h.run(&["jobs", "wait", job, "--profile", "supply"]).await;
    assert_failure(&failed, c, 5, "DEPENDENCY_UNAVAILABLE");
    iam.with(|f| {
        f.operations[0]["operationVersion"] = json!("1.1.0");
        f.operations[0]["contractDigest"] = json!("b".repeat(64));
    });
    let reads = business.count();
    let (failed, c) = h.run(&["jobs", "wait", job, "--profile", "supply"]).await;
    assert_failure(&failed, c, 6, "UNKNOWN_OPERATION");
    assert_eq!(business.count(), reads);
    let (status, c) = h.run(&["jobs", "status", job, "--profile", "supply"]).await;
    assert_eq!(c, 0, "{status}");
    assert_eq!(status["data"]["operationVersion"], "1.0.0");
    assert_eq!(status["data"]["pageCount"], 1);
    assert_eq!(status["data"]["complete"], false);
}

#[tokio::test]
async fn business_request_frequency_is_shared_by_operations_and_rate_rejection_stops_without_retry()
{
    let business = Iam::start(SYSTEM, ENV, vec![]);
    business.with(|f| {
        f.overrides.insert(
            "direct.business",
            reply(200, json!({"items":[{"value":42}],"pages":1,"pageNum":1})),
        );
    });
    let other = "supply-chain-server.another.direct";
    let operations = [DIRECT, other].map(|name| {
        let mut op = operation(&business.origin, name);
        op["direct"]["limits"]["minRequestIntervalMillis"] = json!(350);
        op
    });
    let iam = Iam::start(SYSTEM, ENV, operations.to_vec());
    iam.with(|f| {
        f.overrides.insert("direct.credential", native_reply("1"));
    });
    let h = Harness::new(&[(ENV, &iam)]);
    h.login("supply", ENV, SYSTEM).await;
    let first = [
        "api",
        "call",
        DIRECT,
        "--profile",
        "supply",
        "--params",
        r#"{"brandId":7}"#,
    ];
    let second = [
        "api",
        "call",
        other,
        "--profile",
        "supply",
        "--params",
        r#"{"brandId":7}"#,
    ];
    let started = std::time::Instant::now();
    let (a, b) = tokio::join!(h.run(&first), h.run(&second));
    assert_eq!(a.1, 0, "{}", a.0);
    assert_eq!(b.1, 0, "{}", b.0);
    assert_eq!(business.count(), 2);
    assert!(started.elapsed() >= std::time::Duration::from_millis(700));
    business.with(|f| {
        f.overrides.insert("direct.business", reply(429, json!({})));
    });
    let (denied, code) = h.run(&first).await;
    assert_ne!(code, 0);
    assert_eq!(denied["error"]["code"], "RATE_LIMITED", "{denied}");
    assert_eq!(
        business.count(),
        3,
        "rate rejection must not trigger an automatic replay"
    );
}
