//! 平台账号公开元数据，用于Agent按本人和环境选择；不读取安全存储凭证。
use super::{model::IdentityDomain, read_profile, serialize};
use crate::{
    Runtime,
    output::{Failure, Result},
    private_store,
};

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
    if private_store::validate(&folder, true).map_err(directory_failure)? {
        for item in std::fs::read_dir(folder)
            .map_err(|e| directory_failure(private_store::io_failure("list", e)))?
        {
            let item = item.map_err(|e| directory_failure(private_store::io_failure("list", e)))?;
            let name = item.file_name();
            let Some(name) = name.to_str().and_then(|v| v.strip_suffix(".json")) else {
                continue;
            };
            if crate::profile::validate_name(name).is_err() {
                continue;
            }
            let row = read_profile(rt, name).map_err(|failure| record_failure(failure, name))?;
            items.push(Account {
                name: name.into(),
                origin: row.origin,
                environment: row.environment,
                subject_id: row.subject_id,
                domain: row.domain,
                expires_at: row.expires_at,
            });
            if items.len() > 1000 {
                return Err(directory_failure(private_store::unavailable()));
            }
        }
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    serialize(Listing {
        items,
        online: false,
    })
}

fn directory_failure(failure: Failure) -> Failure {
    let mut failure = private_store::context(failure, "profile_directory");
    failure.hint = "核对本机平台账号目录的权限和所有者，保留已有账号记录。";
    failure
}

fn record_failure(failure: Failure, name: &str) -> Failure {
    let mut failure = private_store::context(failure, "profile_record");
    failure.hint = "核对失败的本地账号记录，保留原记录和凭证。";
    if name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        failure
            .details
            .get_or_insert_with(Default::default)
            .profile_record = Some(name.into());
    }
    failure
}
