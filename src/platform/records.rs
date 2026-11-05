//! 复用既有私有文件和操作系统锁，记录签名覆盖完整上下文与原始结果。
use crate::{
    Runtime,
    output::{Result, invalid},
    private_store,
    tasks::journal::lock,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::Sha256;
use std::path::{Path, PathBuf};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Sealed<T> {
    /// 具有明确载荷类型的任务、写入或结果记录。
    record: T,
    /// 覆盖完整序列化记录的HMAC，密钥只保存在既有安全存储。
    signature: String,
}
pub(super) fn id(value: &str) -> Result<()> {
    if value.len() != 43
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return Err(invalid());
    }
    Ok(())
}
pub(super) fn path(rt: &Runtime, area: &str, id_value: &str) -> Result<PathBuf> {
    id(id_value)?;
    if !["jobs", "writes"].contains(&area) {
        return Err(invalid());
    }
    Ok(rt
        .root
        .join("platform")
        .join(area)
        .join(format!("{id_value}.json")))
}
pub(super) async fn acquire(rt: &Runtime, area: &str, id_value: &str) -> Result<std::fs::File> {
    let path = path(rt, area, id_value)?;
    lock::acquire(&path.with_extension("lock")).await
}
pub(super) async fn write<T: Serialize>(
    rt: &Runtime,
    path: &Path,
    value: &T,
    limit: u64,
) -> Result<()> {
    let key = lock::key(rt, true).await?;
    let bytes = serde_json::to_vec(value).map_err(|_| private_store::unavailable())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| private_store::unavailable())?;
    mac.update(&bytes);
    let sealed = Sealed {
        record: value,
        signature: URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()),
    };
    private_store::write_private_result(path, &sealed, limit)
}
pub(super) async fn read<T: DeserializeOwned + Serialize>(
    rt: &Runtime,
    path: &Path,
    limit: u64,
) -> Result<Option<T>> {
    let Some(sealed) = private_store::read_private_result::<Sealed<T>>(path, limit)? else {
        return Ok(None);
    };
    let key = lock::key(rt, false).await?;
    let bytes = serde_json::to_vec(&sealed.record).map_err(|_| private_store::unavailable())?;
    let signature = URL_SAFE_NO_PAD
        .decode(sealed.signature)
        .map_err(|_| private_store::unavailable())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| private_store::unavailable())?;
    mac.update(&bytes);
    mac.verify_slice(&signature)
        .map_err(|_| private_store::unavailable())?;
    Ok(Some(sealed.record))
}
pub(super) fn same_profile(
    original: &super::model::PlatformProfile,
    current: &super::model::PlatformProfile,
) -> Result<()> {
    if original.origin != current.origin
        || original.subject_id != current.subject_id
        || original.domain != current.domain
        || original.system_id != current.system_id
        || original.environment != current.environment
    {
        return Err(crate::output::Failure::new(
            "TASK_CONTEXT_CHANGED",
            4,
            "原任务固定的本人、系统或环境发生变化，未继续执行；已完成结果保留。",
        ));
    }
    Ok(())
}

pub(super) struct Index {
    /// 当前页私有记录标识，按最近修改时间排序。
    pub ids: Vec<String>,
    /// 本地记录总数。
    pub total: usize,
    /// 当前页码。
    pub page: u32,
}
pub(super) fn index(rt: &Runtime, area: &str, page: u32) -> Result<Index> {
    if !(1..=100000).contains(&page) || !["jobs", "writes"].contains(&area) {
        return Err(invalid());
    }
    let folder = rt.root.join("platform").join(area);
    if !private_store::validate(&folder, true)? {
        return Ok(Index {
            ids: vec![],
            total: 0,
            page,
        });
    }
    let mut entries = Vec::new();
    for item in std::fs::read_dir(folder).map_err(|_| private_store::unavailable())? {
        let item = item.map_err(|_| private_store::unavailable())?;
        let name = item.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(id_value) = name.strip_suffix(".json") else {
            continue;
        };
        if id(id_value).is_err() {
            continue;
        }
        private_store::validate(&item.path(), false)?;
        let modified = item
            .metadata()
            .and_then(|m| m.modified())
            .map_err(|_| private_store::unavailable())?;
        entries.push((modified, id_value.to_owned()));
        if entries.len() > 100000 {
            return Err(private_store::unavailable());
        }
    }
    entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let total = entries.len();
    let ids = entries
        .into_iter()
        .skip((page as usize - 1) * 20)
        .take(20)
        .map(|(_, id)| id)
        .collect();
    Ok(Index { ids, total, page })
}
pub(super) fn created(event: &str, id: &str) -> Result<()> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Created<'a> {
        /// 记录创建事件类型，不包含业务参数。
        event: &'a str,
        /// 已持久化的本地记录标识。
        record_id: &'a str,
    }
    eprintln!(
        "{}",
        serde_json::to_string(&Created {
            event,
            record_id: id
        })
        .map_err(|_| private_store::unavailable())?
    );
    Ok(())
}
