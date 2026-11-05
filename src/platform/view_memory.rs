//! 最近视角只标记当前合法候选，不能授予执行权限。
use super::{
    input_model::TaskView,
    model::{IdentityDomain, PlatformProfile},
};
use crate::{Runtime, output::Result, private_store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Recent {
    /// 本人平台地址。
    origin: String,
    /// 原身份主体。
    subject_id: String,
    /// 原身份域。
    domain: IdentityDomain,
    /// 原固定环境。
    environment: String,
    /// 业务系统编码。
    system_id: String,
    /// 最近明确使用的任务视角，只作提示。
    view: TaskView,
}
fn path(rt: &Runtime, p: &PlatformProfile, system: &str) -> std::path::PathBuf {
    let key = format!(
        "{:x}",
        Sha256::digest(format!(
            "{}\n{}\n{:?}\n{}\n{}",
            p.origin, p.subject_id, p.domain, p.environment, system
        ))
    );
    rt.root
        .join("platform-view-hints")
        .join(format!("{key}.json"))
}
pub(super) fn recent(rt: &Runtime, p: &PlatformProfile, system: &str) -> Option<TaskView> {
    let saved: Recent = private_store::read(&path(rt, p, system)).ok()??;
    if saved.origin != p.origin
        || saved.subject_id != p.subject_id
        || saved.domain != p.domain
        || saved.environment != p.environment
        || saved.system_id != system
    {
        return None;
    }
    Some(saved.view)
}
pub(super) fn remember(
    rt: &Runtime,
    p: &PlatformProfile,
    system: &str,
    view: &TaskView,
) -> Result<()> {
    private_store::write(
        &path(rt, p, system),
        &Recent {
            origin: p.origin.clone(),
            subject_id: p.subject_id.clone(),
            domain: p.domain.clone(),
            environment: p.environment.clone(),
            system_id: system.into(),
            view: view.clone(),
        },
    )
}
