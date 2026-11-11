//! 受限 HTTP 客户端与安全协议错误。
use crate::output::{Failure, Result};
use std::time::Duration;
pub(crate) mod diagnostics;
pub fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(
            crate::constants::query::CONNECT_SECONDS,
        ))
        .timeout(Duration::from_secs(
            crate::constants::query::REQUEST_SECONDS,
        ))
        .build()
        .map_err(|_| Failure::new("INTERNAL_ERROR", 1, "无法建立 HTTP 客户端。"))
}

pub fn network() -> Failure {
    Failure::new("NETWORK_ERROR", 5, "网络请求未完成；请核对原任务结果。")
}
pub fn protocol() -> Failure {
    Failure::new("DEPENDENCY_UNAVAILABLE", 5, "响应不符合固定协议。")
}
