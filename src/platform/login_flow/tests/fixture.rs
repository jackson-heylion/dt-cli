use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Copy)]
pub(super) struct Scenario {
    /// 匿名检查的 HTTP 状态。
    pub preflight_status: u16,
    /// 匿名检查的媒体类型。
    pub preflight_type: &'static str,
    /// 匿名检查的原始协议夹具。
    pub preflight_body: &'static str,
    /// 授权码换码的 HTTP 状态。
    pub token_status: u16,
    /// 换码后会话检查的 HTTP 状态。
    pub session_status: u16,
    /// 会话返回已撤销状态，用于校验业务拒绝边界。
    pub revoked: bool,
}
impl Default for Scenario {
    fn default() -> Self {
        Self {
            preflight_status: 401,
            preflight_type: "application/json; charset=utf-8",
            preflight_body: "{\"code\":\"AUTH_REQUIRED\"}",
            token_status: 200,
            session_status: 200,
            revoked: false,
        }
    }
}

pub(super) struct ObservedRequest {
    /// 实际请求的 HTTP 方法。
    pub method: String,
    /// 实际请求路径，不保存查询串或认证值。
    pub path: String,
    /// 是否带有认证头；不保留认证头值。
    pub authenticated: bool,
    /// 换码表单包含约定的授权码、回调及 PKCE 校验字段。
    pub valid_exchange: bool,
}

pub(super) struct Server {
    /// 隔离服务 origin，仅绑定本机随机端口。
    pub origin: String,
    /// 不含凭证值的实际 HTTP 请求观察结果。
    pub requests: Arc<Mutex<Vec<ObservedRequest>>>,
    /// 隔离服务器任务，离开夹具时停止。
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    pub async fn start(scenario: Scenario, purpose: SessionPurpose) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(vec![]));
        let observed = requests.clone();
        let namespace = purpose.namespace();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let header_end = loop {
                    let mut buffer = [0; 2048];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&buffer[..n]);
                    if let Some(index) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break index + 4;
                    }
                };
                let headers = std::str::from_utf8(&bytes[..header_end]).unwrap();
                let mut line = headers.lines().next().unwrap().split(' ');
                let method = line.next().unwrap().to_owned();
                let path = line.next().unwrap().to_owned();
                let authenticated = headers
                    .lines()
                    .any(|h| h.to_ascii_lowercase().starts_with("authorization:"));
                let length: usize = headers
                    .lines()
                    .find_map(|h| {
                        let (key, value) = h.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < header_end + length {
                    let mut buffer = [0; 2048];
                    let n = socket.read(&mut buffer).await.unwrap();
                    assert_ne!(n, 0);
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let form: Vec<_> = url::form_urlencoded::parse(&bytes[header_end..]).collect();
                let valid_exchange = [
                    "code",
                    "code_verifier",
                    "redirect_uri",
                    "grant_type",
                    "client_id",
                ]
                .iter()
                .all(|key| {
                    form.iter()
                        .any(|(name, value)| name == key && !value.is_empty())
                });
                observed.lock().unwrap().push(ObservedRequest {
                    method,
                    path: path.clone(),
                    authenticated,
                    valid_exchange,
                });
                let (status, kind, body) = if path.ends_with("/oauth2/token") {
                    (scenario.token_status, "application/json", token(purpose))
                } else if path.ends_with("/oauth2/revoke") {
                    (200, "application/json", "{}".into())
                } else if path == format!("{namespace}/me") && authenticated {
                    let view = SessionView {
                        session_id: "A".repeat(43),
                        subject_id: "fixture-employee".into(),
                        domain: IdentityDomain::Employee,
                        display_name: "测试员工".into(),
                        purpose: purpose.text().into(),
                        expires_at: "2099-01-01T00:00:00Z".into(),
                        current: true,
                        revoked: scenario.revoked,
                    };
                    (
                        scenario.session_status,
                        "application/json",
                        serde_json::to_string(&view).unwrap(),
                    )
                } else {
                    (
                        scenario.preflight_status,
                        scenario.preflight_type,
                        scenario.preflight_body.into(),
                    )
                };
                let reply = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                socket.write_all(reply.as_bytes()).await.unwrap();
            }
        });
        Self {
            origin,
            requests,
            task,
        }
    }
}

fn token(purpose: SessionPurpose) -> String {
    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::seconds(3600);
    format!(
        r#"{{"access_token":"{}{suffix}","refresh_token":"{}{suffix}","token_type":"Bearer","expires_in":3600,"authorizationId":"{suffix}","authorizationExpiresAt":"2099-01-01T00:00:00Z","environment":"fixture","systemId":"platform","accessExpiresAt":"{}","serverTime":"{}"}}"#,
        purpose.access_prefix(),
        purpose.refresh_prefix(),
        expires.to_rfc3339(),
        now.to_rfc3339(),
        suffix = "A".repeat(43)
    )
}

pub(super) struct BrowserProbe {
    /// 模拟浏览器启动调用次数。
    pub calls: Arc<AtomicUsize>,
    /// 本次回调端口，用于观察监听保活；不保存授权链接。
    pub address: Arc<Mutex<Option<std::net::SocketAddr>>>,
    /// 模拟系统打开调用失败。
    pub fail: bool,
    /// 是否模拟本人确认并返回有效回调。
    pub callback: bool,
    /// 本人操作延时，单位：秒；测试使用虚拟时钟。
    pub delay_seconds: u64,
    /// 浏览器实际取得的回调页面，不含授权参数。
    pub pages: Arc<Mutex<Vec<String>>>,
}
impl Browser for BrowserProbe {
    fn open(&self, address: &str) -> Result<()> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let url = url::Url::parse(address).unwrap();
        let value = |key| {
            url.query_pairs()
                .find(|(name, _)| name == key)
                .unwrap()
                .1
                .into_owned()
        };
        let mut callback = url::Url::parse(&value("redirect_uri")).unwrap();
        *self.address.lock().unwrap() = Some(callback.socket_addrs(|| None).unwrap()[0]);
        if self.fail {
            return Err(Failure::new(
                "BROWSER_OPEN_FAILED",
                6,
                "模拟系统打开调用失败。",
            ));
        }
        if self.callback {
            callback.query_pairs_mut().extend_pairs([
                ("state", value("state")),
                ("iss", url.origin().ascii_serialization()),
                ("code", "fixture-code".into()),
            ]);
            let pages = self.pages.clone();
            let delay = self.delay_seconds;
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(delay)).await;
                // 延迟回调用例暂停时钟验证保活，进入 HTTP 阶段后恢复真实 I/O 调度。
                if delay > 0 {
                    tokio::time::resume();
                }
                let page = local_client()
                    .get(callback)
                    .send()
                    .await
                    .unwrap()
                    .text()
                    .await
                    .unwrap();
                pages.lock().unwrap().push(page);
            });
        }
        Ok(())
    }
}

pub(super) fn local_client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

pub(super) struct WriteFailureStore {
    /// 本测试私有凭证目录；读取和清理仍使用真实文件存储。
    pub inner: FileStore,
}
impl CredentialStore for WriteFailureStore {
    fn read(&self, key: &str) -> Result<Option<Credentials>> {
        self.inner.read(key)
    }
    fn write(&self, _: &str, _: &Credentials) -> Result<()> {
        Err(crate::output::storage())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.inner.delete(key)
    }
}
