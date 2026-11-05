use dt_cli::{login::Browser, output::Result};
use serde::{Deserialize, Serialize};
pub static OPENS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub struct PlatformBrowser;
pub struct IdentityBrowser {
    /// 测试IAM登录码字符，仅选择可控员工身份。
    pub code: char,
}
impl Browser for IdentityBrowser {
    fn open(&self, address: &str) -> Result<()> {
        open(address, self.code)
    }
}
impl Browser for PlatformBrowser {
    fn open(&self, address: &str) -> Result<()> {
        open(address, 'C')
    }
}
fn open(address: &str, code: char) -> Result<()> {
    OPENS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let address = address.to_owned();
    tokio::spawn(async move {
        let authorization = url::Url::parse(&address).unwrap();
        let platform = authorization.origin().ascii_serialization();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let start = client.get(address).send().await.unwrap();
        assert_eq!(start.status(), 303);
        let cookie = start.headers()["set-cookie"]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let identity = url::Url::parse(start.headers()["location"].to_str().unwrap()).unwrap();
        let id = identity
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        let response = client
            .get(format!(
                "{platform}/platform/login/callback?state={id}&code={}",
                code.to_string().repeat(43)
            ))
            .header("Cookie", &cookie)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 303);
        let view: Consent = client
            .get(format!("{platform}/platform/cli/consent?requestId={id}"))
            .header("Cookie", &cookie)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let approved = client
            .post(format!("{platform}/platform/cli/consent"))
            .header("Cookie", &cookie)
            .header("Origin", &platform)
            .json(&Decision {
                request_id: id,
                csrf: view.csrf,
                approve: true,
            })
            .send()
            .await
            .unwrap();
        assert_eq!(approved.status(), 200);
        let next: Redirect = approved.json().await.unwrap();
        let callback = client.get(next.location).send().await.unwrap();
        assert_eq!(callback.status(), 200);
    });
    Ok(())
}
#[derive(Deserialize)]
struct Consent {
    /// 绑定当前浏览器的确认值。
    csrf: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Decision {
    /// 当前平台登录事务。
    request_id: String,
    /// 本次浏览器确认值。
    csrf: String,
    /// 是否同意建立独立 CLI 会话。
    approve: bool,
}
#[derive(Deserialize)]
struct Redirect {
    /// 已绑定原事务的本机回调地址。
    location: String,
}
