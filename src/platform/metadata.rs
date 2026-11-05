//! 进程内短期复用不可变契约与运行地址；恢复和明确拒绝会失效，不缓存许可。
use super::{contract::Published, model::PlatformProfile};
use std::{
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq)]
struct Scope {
    /// 完整平台地址，跨平台不复用。
    origin: String,
    /// 固定环境。
    environment: String,
    /// 固定系统上下文；平台会话不代表系统授权。
    system: String,
    /// IAM稳定人员标识。
    subject: String,
    /// 身份域序列化值。
    domain: String,
    /// 本次独立平台会话，不复活旧会话缓存。
    session: String,
    /// 客户端兼容条件。
    cli_version: &'static str,
}
impl Scope {
    fn of(p: &PlatformProfile) -> Self {
        Self {
            origin: p.origin.clone(),
            environment: p.environment.clone(),
            system: p.system_id.clone(),
            subject: p.subject_id.clone(),
            domain: format!("{:?}", p.domain),
            session: p.session_id.clone(),
            cli_version: env!("CARGO_PKG_VERSION"),
        }
    }
}
struct ContractEntry {
    /// 已验证的完整隔离上下文。
    scope: Scope,
    /// 含固定系统、版本、摘要及兼容性信息的契约。
    published: Published,
    /// 缓存最初建立时刻；命中不延长。
    created: Instant,
    /// 契约传输体积，用于累计8MiB缓存上限。
    bytes: usize,
}
struct RuntimeEntry {
    /// 已验证的完整隔离上下文。
    scope: Scope,
    /// 部署方提供的Gateway地址。
    origin: String,
    /// 最初建立时刻；命中不延长。
    created: Instant,
}
#[derive(Default)]
struct Metadata {
    /// 最多128份固定契约。
    contracts: Vec<ContractEntry>,
    /// 最多128份运行配置。
    runtimes: Vec<RuntimeEntry>,
}
const TTL: Duration = Duration::from_secs(60);
fn cache() -> &'static Mutex<Metadata> {
    static CACHE: OnceLock<Mutex<Metadata>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}
pub(super) fn contract(p: &PlatformProfile, operation: &str, version: u64) -> Option<Published> {
    let mut cache = cache().lock().ok()?;
    cache.contracts.retain(|e| e.created.elapsed() < TTL);
    cache
        .contracts
        .iter()
        .find(|e| {
            e.scope == Scope::of(p)
                && e.published.contract.operation_id == operation
                && e.published.contract.operation_version == version
        })
        .map(|e| e.published.clone())
}
pub(super) fn remember_contract(p: &PlatformProfile, published: &Published) {
    if let Ok(mut cache) = cache().lock() {
        cache.contracts.retain(|e| e.created.elapsed() < TTL);
        let bytes = match serde_json::to_vec(published) {
            Ok(value) => value.len(),
            Err(_) => return,
        };
        if bytes > 2_097_152 {
            return;
        }
        while cache.contracts.len() >= 128
            || cache.contracts.iter().map(|e| e.bytes).sum::<usize>() + bytes > 8_388_608
        {
            cache.contracts.remove(0);
        }
        cache.contracts.push(ContractEntry {
            scope: Scope::of(p),
            published: published.clone(),
            created: Instant::now(),
            bytes,
        });
    }
}
pub(super) fn runtime(p: &PlatformProfile) -> Option<String> {
    let mut cache = cache().lock().ok()?;
    cache.runtimes.retain(|e| e.created.elapsed() < TTL);
    cache
        .runtimes
        .iter()
        .find(|e| e.scope == Scope::of(p))
        .map(|e| e.origin.clone())
}
pub(super) fn remember_runtime(p: &PlatformProfile, origin: String) {
    if let Ok(mut cache) = cache().lock() {
        cache.runtimes.retain(|e| e.created.elapsed() < TTL);
        if cache.runtimes.len() >= 128 {
            cache.runtimes.remove(0);
        }
        cache.runtimes.push(RuntimeEntry {
            scope: Scope::of(p),
            origin,
            created: Instant::now(),
        });
    }
}
pub(super) fn invalidate(p: &PlatformProfile) {
    if let Ok(mut cache) = cache().lock() {
        cache.contracts.retain(|e| e.scope != Scope::of(p));
        cache.runtimes.retain(|e| e.scope != Scope::of(p));
    }
}
