//! 客户端与Gateway分段观测，数值仅用于报告，不作为权限或重试依据。
#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Timing {
    /// 当前CLI调用数据面的总毫秒，包含本次准备和响应校验。
    pub client_elapsed_millis: u64,
    /// 当前访问凭证与运行地址准备毫秒；读取入口另包含契约和视角准备。
    pub preparation_millis: u64,
    /// 请求发出至完整响应取得的客户端毫秒，包含网络和Gateway嵌套区间。
    pub gateway_request_millis: u64,
    /// Gateway提供的六个嵌套观测区间，未提供或格式无效时为空。
    pub gateway: Option<GatewayTiming>,
}
#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct GatewayTiming {
    /// 平台核权往返毫秒。
    platform_auth: Option<f64>,
    /// 请求契约校验毫秒。
    input_validation: Option<f64>,
    /// 实际业务上游毫秒。
    upstream: Option<f64>,
    /// 响应脱敏及契约处理毫秒。
    response_processing: Option<f64>,
    /// 写入结果回报毫秒。
    write_report: Option<f64>,
    /// 平台当前身份核验毫秒。
    platform_identity: Option<f64>,
    /// 平台会话与接口授权毫秒。
    platform_authority: Option<f64>,
    /// 平台业务凭证取得或复用毫秒。
    platform_credential: Option<f64>,
    /// 业务凭证缓存查找毫秒，包含于platformCredential。
    platform_cache_lookup: Option<f64>,
    /// 业务凭证租约协调等待毫秒，包含于platformCredential。
    platform_coordination: Option<f64>,
    /// 实际业务凭证换证毫秒，包含于platformCredential。
    platform_exchange: Option<f64>,
    /// 平台审计写入毫秒。
    platform_audit: Option<f64>,
    /// 平台总区间毫秒，包含于Gateway核权，不重复相加。
    platform_total: Option<f64>,
    /// Gateway总区间毫秒，不能与嵌套区间重复相加。
    gateway_total: Option<f64>,
}
pub(super) fn parse(value: Option<&str>) -> Option<GatewayTiming> {
    let value = value.filter(|v| v.len() <= 1024 && !v.chars().any(char::is_control))?;
    let mut result = GatewayTiming::default();
    for item in value.split(',') {
        let (name, duration) = item.trim().split_once(";dur=")?;
        let duration: f64 = duration.parse().ok()?;
        if !duration.is_finite() || !(0.0..=300_000.0).contains(&duration) {
            return None;
        }
        let field = match name {
            "platform_auth" => &mut result.platform_auth,
            "platform_identity" => &mut result.platform_identity,
            "platform_authority" => &mut result.platform_authority,
            "platform_credential" => &mut result.platform_credential,
            "platform_cache_lookup" => &mut result.platform_cache_lookup,
            "platform_coordination" => &mut result.platform_coordination,
            "platform_exchange" => &mut result.platform_exchange,
            "platform_audit" => &mut result.platform_audit,
            "platform_total" => &mut result.platform_total,
            "input_validation" => &mut result.input_validation,
            "upstream" => &mut result.upstream,
            "response_processing" => &mut result.response_processing,
            "write_report" => &mut result.write_report,
            "gateway_total" => &mut result.gateway_total,
            _ => return None,
        };
        if field.replace(duration).is_some() {
            return None;
        }
    }
    result.gateway_total.map(|_| result)
}
pub(super) fn millis(started: std::time::Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}
