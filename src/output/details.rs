//! 有界诊断字段；不接收业务正文、认证头或服务端消息原文。
use serde::Serialize;
use std::path::PathBuf;
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Details {
    /// 稳定失败阶段，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    /// 安全失败原因，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// 操作系统错误编号，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_code: Option<i32>,
    /// 当前本机存储用途，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub area: Option<String>,
    /// 当前调用自己创建的临时文件清理结果，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<Cleanup>,
    /// 本次原子替换两项对照结果，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename_comparison: Option<RenameComparison>,
    /// 当前调用自己的临时文件路径，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup_path: Option<PathBuf>,
    /// 经字符和长度核验的原协议错误码，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// 当前受控 HTTP 状态，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
    /// 经字符和长度核验的关联号，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    /// 固定安全恢复要求，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    /// 原管理计划编号，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_id: Option<String>,
    /// 原管理执行请求号，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// 浏览器启动事实，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_launch: Option<String>,
    /// 是否核验浏览器窗口，未观察时为 false。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_window_verified: Option<bool>,
    /// 本次本机回调地址，不含授权参数。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_address: Option<String>,
    /// 已观察到的回调阶段，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_status: Option<String>,
    /// 回调最长等待时间，单位：秒，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wait_seconds: Option<u64>,
    /// 当前本机客户端设置是否已核验，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration_verified: Option<bool>,
    /// 技能最低 CLI 版本，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_cli_version: Option<String>,
    /// 当前明确技能名称，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skill: Option<String>,
    /// 安全参数错误类别，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_kind: Option<String>,
    /// 命令树中已登记的错误作用域，可空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cleanup {
    /// 清理自己的临时文件结果。
    pub status: &'static str,
    /// 本调用创建的临时路径，可空。
    pub path: Option<PathBuf>,
    /// 操作系统错误编号，可空。
    pub os_code: Option<i32>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameComparison {
    /// 原目标不存在时的原子更名结果。
    pub rename_to_new: &'static str,
    /// 替换本调用已建目标时的结果。
    pub replace_existing: &'static str,
}
