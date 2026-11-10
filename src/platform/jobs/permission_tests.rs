use super::*;
use crate::credentials::{Credentials, FileStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn gateway_permission_denial_preserves_checkpoint_and_admin_guidance() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut input = [0_u8; 4096];
        let size = socket.read(&mut input).await.unwrap();
        let request = String::from_utf8_lossy(&input[..size]);
        assert!(request.starts_with("GET /platform-gateway/fixture/orders/1/orders"));
        let body = "{\"code\":\"INTERFACE_PERMISSION_REQUIRED\"}";
        let response = format!(
            "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nX-Platform-Error-Code: INTERFACE_PERMISSION_REQUIRED\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    let root = tempfile::tempdir().unwrap();
    let rt = Runtime {
        root: root.path().into(),
        store: Box::new(FileStore::new(root.path())),
        browser: Box::new(crate::login::SystemBrowser),
        interactive: false,
        aggregate_budget: std::time::Duration::from_secs(30),
    };
    let expiry = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let profile = PlatformProfile {
        purpose: super::super::model::SessionPurpose::Cli,
        schema_version: 1,
        origin: origin.clone(),
        environment: "fixture".into(),
        system_id: "platform".into(),
        subject_id: "1024".into(),
        domain: super::super::model::IdentityDomain::Employee,
        session_id: "S".repeat(43),
        expires_at: expiry.clone(),
        credential_key: format!(
            "platform:{}:fixture:{}",
            super::super::transport::challenge(&origin),
            "K".repeat(43)
        ),
    };
    rt.store
        .write(
            &profile.credential_key,
            &Credentials {
                access_token: format!("dtp_cli_at_{}", "A".repeat(43)),
                refresh_token: format!("dtp_cli_rt_{}", "R".repeat(43)),
                version: 1,
                access_expires_at: Some(expiry.clone()),
                authorization_expires_at: Some(expiry),
                generation: 0,
                refresh_state: "ready".into(),
                refresh_request_id: None,
            },
        )
        .unwrap();
    let contract: ExecutionContract = serde_json::from_str(r#"{
        "contractSchemaVersion":1,"operationId":"orders","operationVersion":1,"minimumCliVersion":"0.6.0",
        "systemId":"orders","environment":"fixture","title":"查询","method":"GET","path":"/orders",
        "effect":"READ","media":"NONE","parameters":[],"requestSchema":null,"responseSchema":"{}",
        "success":{"statusCodes":[200],"bodyPointer":null,"expectedJson":null},
        "pagination":{"kind":"NONE","pageParameter":null,"sizeParameter":null,"itemsPointer":null,"nextPointer":null,"totalPointer":null,"firstPage":1,"pageSize":100,"placement":"QUERY"},
        "viewTypes":[],"dataScope":"self","sensitiveFields":[],"routeBindingDigest":"fixture-route",
        "limits":{"maxPages":100,"maxRecords":10000,"maxBytes":2097152,"maxSeconds":120},"requestBodyRequired":false
    }"#).unwrap();
    let published = Published {
        contract,
        digest: "fixture-digest".into(),
    };
    super::super::metadata::remember_contract(&profile, &published);
    super::super::metadata::remember_runtime(&profile, origin);
    let request = super::super::input_model::WireInvocation {
        path: "/orders".into(),
        query: String::new(),
        headers: vec![],
        body: String::new(),
        view: None,
    };
    let mut progress = paging::initial(&published.contract, &request).unwrap();
    progress.pages = 1;
    progress.records = 2;
    progress.bytes = 24;
    let mut job = Job {
        schema_version: 1,
        id: "J".repeat(43),
        profile_name: "fixture".into(),
        profile,
        published,
        request,
        progress,
        state: JobState::Running,
        reason: None,
        created_at: now(),
        updated_at: now(),
    };
    let failure = run(&rt, &mut job, false).await.unwrap_err();
    assert_eq!(failure.code, "INTERFACE_PERMISSION_REQUIRED");
    server.await.unwrap();
    let persisted = load(&rt, &job.id).await.unwrap();
    assert!(persisted.state == JobState::Paused);
    assert_eq!(persisted.progress.pages, 1);
    assert_eq!(persisted.progress.records, 2);
    assert_eq!(persisted.progress.bytes, 24);
    assert_eq!(
        persisted.reason.as_deref(),
        Some("INTERFACE_PERMISSION_REQUIRED")
    );
    let result = summary(&persisted).unwrap();
    assert!(result["message"].as_str().unwrap().contains("管理员"));
    assert!(result["message"].as_str().unwrap().contains("接口工作区"));
}
