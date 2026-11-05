use super::super::{
    contract::ExecutionContract,
    input_model::WireInvocation,
    model::{IdentityDomain, SessionView},
};
use super::*;
use crate::credentials::{Credentials, FileStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn request(socket: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    while !bytes.windows(4).any(|part| part == b"\r\n\r\n") {
        let mut chunk = [0_u8; 4096];
        let size = socket.read(&mut chunk).await.unwrap();
        assert!(size > 0 && bytes.len() < 16384);
        bytes.extend_from_slice(&chunk[..size]);
    }
    String::from_utf8(bytes).unwrap()
}

async fn reply(socket: &mut tokio::net::TcpStream, status: u16, body: &[u8]) {
    socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
    socket.write_all(body).await.unwrap();
}

async fn recovery_case(deny_identity: bool) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let root = tempfile::tempdir().unwrap();
    let rt = Runtime {
        root: root.path().into(),
        store: Box::new(FileStore::new(root.path())),
        environments: Default::default(),
        browser: Box::new(crate::login::SystemBrowser),
        interactive: false,
        aggregate_budget: std::time::Duration::from_secs(30),
    };
    let expiry = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let profile = PlatformProfile {
        purpose: super::super::model::SessionPurpose::Cli,
        schema_version: 1,
        origin: format!("http://{}", listener.local_addr().unwrap()),
        environment: "fixture".into(),
        system_id: "orders".into(),
        subject_id: "1024".into(),
        domain: IdentityDomain::Employee,
        session_id: "S".repeat(43),
        expires_at: expiry.clone(),
        credential_key: "fixture-credentials".into(),
    };
    rt.store
        .write(
            &profile.credential_key,
            &Credentials {
                access_token: format!("dtp_cli_at_{}", "A".repeat(43)),
                refresh_token: format!("dtp_cli_rt_{}", "R".repeat(43)),
                version: 1,
                access_expires_at: Some(expiry.clone()),
                authorization_expires_at: Some(expiry.clone()),
                generation: 0,
                refresh_state: "ready".into(),
                refresh_request_id: None,
            },
        )
        .unwrap();
    let contract: ExecutionContract = serde_json::from_str(r#"{
        "contractSchemaVersion":1,"operationId":"orders","operationVersion":1,"minimumCliVersion":"0.6.0",
        "systemId":"orders","environment":"fixture","title":"写入","method":"POST","path":"/orders",
        "effect":"WRITE","media":"JSON","parameters":[],"requestSchema":"{}","responseSchema":"{}",
        "success":{"statusCodes":[200],"bodyPointer":null,"expectedJson":null},
        "pagination":{"kind":"NONE","pageParameter":null,"sizeParameter":null,"itemsPointer":null,"nextPointer":null,"totalPointer":null,"firstPage":1,"pageSize":1,"placement":"QUERY"},
        "viewTypes":[],"dataScope":"self","sensitiveFields":[],"routeBindingDigest":"fixture-route",
        "limits":{"maxPages":1,"maxRecords":1,"maxBytes":65536,"maxSeconds":30},"requestBodyRequired":true
    }"#).unwrap();
    let published = super::super::contract::Published {
        digest: executor::digest(&serde_json::to_vec(&contract).unwrap()),
        contract,
    };
    let mut row = Record {
        schema_version: 1,
        id: "R".repeat(43),
        profile_name: "fixture".into(),
        profile: profile.clone(),
        published,
        request: WireInvocation {
            path: "/orders".into(),
            query: String::new(),
            headers: vec![],
            body: "{}".into(),
            view: None,
        },
        instruction: "仅核对原意图，不派发".into(),
        stage: Stage::Authorized,
        intent: None,
        result: None,
    };
    row.intent = Some(Intent {
        id: "I".repeat(43),
        client_request_id: row.id.clone(),
        instruction: row.instruction.clone(),
        subject_id: profile.subject_id.clone(),
        session_id: profile.session_id.clone(),
        system_id: "orders".into(),
        environment: "fixture".into(),
        operation_id: "orders".into(),
        operation_version: 1,
        contract_digest: row.published.digest.clone(),
        fingerprint: fingerprint::compute(&row).unwrap(),
        view: None,
        state: WriteState::Authorized,
        expires_at: chrono::Utc::now().timestamp() + 1800,
        http_status: None,
        response_digest: None,
    });
    save(&rt, &row).await.unwrap();
    let path = records::path(&rt, "writes", &row.id).unwrap();
    let original = std::fs::read(&path).unwrap();
    let identity = serde_json::to_vec(&SessionView {
        session_id: profile.session_id.clone(),
        subject_id: profile.subject_id.clone(),
        domain: profile.domain.clone(),
        display_name: "隔离人员".into(),
        purpose: "CLI".into(),
        expires_at: expiry,
        current: true,
        revoked: false,
    })
    .unwrap();
    let contract = serde_json::to_vec(&row.published).unwrap();
    let intent = serde_json::to_vec(row.intent.as_ref().unwrap()).unwrap();
    let server = tokio::spawn(async move {
        let mut waiting = Vec::new();
        // 两项独立核验都到达才响应：串行实现会有界超时，不能靠删减检查通过。
        for _ in 0..2 {
            let (mut socket, _) =
                tokio::time::timeout(std::time::Duration::from_secs(3), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let head = request(&mut socket).await;
            waiting.push((socket, head));
        }
        assert!(
            waiting
                .iter()
                .any(|(_, head)| head.starts_with("GET /platform/cli/me "))
        );
        assert!(
            waiting
                .iter()
                .any(|(_, head)| head.starts_with("GET /platform/cli/catalog/orders/1 "))
        );
        for (mut socket, head) in waiting {
            if head.starts_with("GET /platform/cli/me ") {
                reply(
                    &mut socket,
                    if deny_identity { 403 } else { 200 },
                    if deny_identity { b"{}" } else { &identity },
                )
                .await;
            } else {
                reply(&mut socket, 200, &contract).await;
            }
        }
        if !deny_identity {
            let (mut socket, _) = listener.accept().await.unwrap();
            assert!(
                request(&mut socket)
                    .await
                    .starts_with("GET /platform/cli/writes/")
            );
            reply(&mut socket, 200, &intent).await;
        }
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        status(&rt, &profile, &row.id),
    )
    .await
    .unwrap();
    server.await.unwrap();
    if deny_identity {
        assert_eq!(result.unwrap_err().code, "AUTH_DENIED");
        assert_eq!(std::fs::read(path).unwrap(), original);
    } else {
        assert_eq!(result.unwrap()["intent"]["state"], "AUTHORIZED");
        assert!(load(&rt, &row.id).await.unwrap().stage == Stage::Authorized);
    }
}

#[tokio::test]
async fn independent_authority_reads_overlap_before_original_intent_readback() {
    recovery_case(false).await;
}

#[tokio::test]
async fn parallel_identity_denial_preserves_original_record_and_blocks_readback() {
    recovery_case(true).await;
}
