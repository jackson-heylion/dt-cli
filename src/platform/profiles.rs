//! 平台账号公开元数据，用于Agent按本人和环境选择；不读取安全存储凭证。
use super::{model::IdentityDomain, read_profile, serialize};
use crate::{Runtime, output::Result, private_store};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Account {
    /// 本地技术记录名，由Agent使用。
    name: String,
    /// 平台公开入口。
    origin: String,
    /// 固定环境。
    environment: String,
    /// IAM稳定人员标识，供已有本人绑定匹配。
    subject_id: String,
    /// 人员身份域。
    domain: IdentityDomain,
    /// 会话绝对期限；此离线列表不证明当前授权可用。
    expires_at: String,
}
#[derive(serde::Serialize)]
struct Listing {
    /// 本机公开平台记录；不含token或凭证键。
    items: Vec<Account>,
    /// 离线读取，不替代实际Gateway核权。
    online: bool,
}
pub(super) fn list(rt: &Runtime) -> Result<serde_json::Value> {
    let folder = rt.root.join("platform");
    let mut items = Vec::new();
    if private_store::validate(&folder, true)? {
        for item in std::fs::read_dir(folder).map_err(|_| private_store::unavailable())? {
            let item = item.map_err(|_| private_store::unavailable())?;
            let name = item.file_name();
            let Some(name) = name.to_str().and_then(|v| v.strip_suffix(".json")) else {
                continue;
            };
            if crate::profile::validate_name(name).is_err() {
                continue;
            }
            let row = read_profile(rt, name)?;
            items.push(Account {
                name: name.into(),
                origin: row.origin,
                environment: row.environment,
                subject_id: row.subject_id,
                domain: row.domain,
                expires_at: row.expires_at,
            });
            if items.len() > 1000 {
                return Err(private_store::unavailable());
            }
        }
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    serialize(Listing {
        items,
        online: false,
    })
}
