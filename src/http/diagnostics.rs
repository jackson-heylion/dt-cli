//! 只输出固定网络原因；底层错误文本、请求地址及代理凭证不进入诊断。
use crate::output::{Details, Failure};
use std::error::Error;

#[derive(Clone, Copy)]
enum NetworkReason {
    /// 本次连接或完整请求超过期限。
    Timeout,
    /// HTTP CONNECT 代理隧道建立失败。
    ProxyConnectFailed,
    /// 当前连接目标的域名解析失败，目标可能是平台或代理。
    DnsResolutionFailed,
    /// TLS 握手或证书验证失败。
    TlsFailed,
    /// 连接未建立；底层未提供可进一步判定的原因。
    ConnectionFailed,
    /// 底层原因无法可靠分类。
    Unknown,
}
impl NetworkReason {
    fn text(self) -> &'static str {
        match self {
            Self::Timeout => "request_timeout",
            Self::ProxyConnectFailed => "proxy_connect_failed",
            Self::DnsResolutionFailed => "dns_resolution_failed",
            Self::TlsFailed => "tls_failed",
            Self::ConnectionFailed => "connection_failed",
            Self::Unknown => "unknown",
        }
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Timeout => "核对当前网络可达性及代理响应时间，再恢复原任务。",
            Self::ProxyConnectFailed => {
                "核对 CLI 执行环境的代理服务及目标域名规则，修复后恢复原任务。"
            }
            Self::DnsResolutionFailed => {
                "核对 CLI 执行环境的 DNS 与代理域名解析；直连也需要有效 DNS。"
            }
            Self::TlsFailed => "核对目标证书、系统时间与代理 TLS 配置，保持证书校验开启。",
            Self::ConnectionFailed => "核对目标端口、代理是否运行及本机网络访问限制。",
            Self::Unknown => "核对 CLI 执行环境的网络与代理配置；当前原因尚未确定。",
        }
    }
}

fn classify(error: &reqwest::Error) -> NetworkReason {
    if error.is_timeout() {
        return NetworkReason::Timeout;
    }
    // 当前锁定依赖没有公开连接原因类型，仅识别其固定错误前缀。
    // 未识别的版本／平台错误保留宽泛原因；这些字符串只用于内部分类。
    let mut source = error.source();
    let mut reason = NetworkReason::Unknown;
    while let Some(cause) = source {
        let text = cause.to_string();
        if text.starts_with("tunnel error:") {
            return NetworkReason::ProxyConnectFailed;
        }
        if text.starts_with("dns error") {
            reason = NetworkReason::DnsResolutionFailed;
        } else if text.starts_with("invalid peer certificate")
            || text.starts_with("received corrupt message")
            || text.starts_with("peer sent no certificates")
        {
            reason = NetworkReason::TlsFailed;
        }
        source = cause.source();
    }
    if matches!(reason, NetworkReason::Unknown) && error.is_connect() {
        NetworkReason::ConnectionFailed
    } else {
        reason
    }
}

pub(crate) fn failure(error: &reqwest::Error, mut failure: Failure) -> Failure {
    let reason = classify(error);
    failure.hint = reason.hint();
    failure.details = Some(Box::new(Details {
        reason: Some(reason.text().into()),
        ..Default::default()
    }));
    failure
}

#[cfg(test)]
mod tests;
